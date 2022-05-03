use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    str::Utf8Error,
};

use crate::{
    cache::{CacheError, RepositoryCache},
    model::protofetch::{Coordinate, Dependency, LockFile, LockedDependency, Revision},
    proto_repository::ProtoRepository,
};

use crate::model::protofetch::{DependencyName, Descriptor};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache: {0}")]
    Cache(#[from] CacheError),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob: {0}")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Bad output dir {0}")]
    BadOutputDir(String),
    #[error("Bad file path {0}")]
    BadFilePath(String),
    #[error("Error while processing protobuf repository: {0}")]
    ProtoRepoError(#[from] crate::proto_repository::ProtoRepoError),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

pub fn lock<Cache: RepositoryCache>(
    descriptor: &Descriptor,
    cache: &Cache,
) -> Result<LockFile, FetchError> {
    fn go<Cache: RepositoryCache>(
        cache: &Cache,
        dep_map: &mut HashMap<DependencyName, Vec<Revision>>,
        repo_map: &mut HashMap<DependencyName, (Coordinate, ProtoRepository, Vec<DependencyName>)>,
        dependencies: &[Dependency],
        parent: Option<&DependencyName>,
    ) -> Result<(), FetchError> {
        for dependency in dependencies {
            log::info!("Resolving {:?}", dependency.coordinate);

            dep_map
                .entry(dependency.name.clone())
                .and_modify(|vec| vec.push(dependency.revision.clone()))
                .or_insert_with(|| vec![dependency.revision.clone()]);

            let repo = cache.clone_or_update(&dependency.coordinate)?;
            let descriptor = repo.extract_descriptor(&dependency.name, &dependency.revision)?;

            repo_map.entry(dependency.name.clone()).or_insert((
                dependency.coordinate.clone(),
                repo,
                vec![],
            ));

            if let Some(p) = parent {
                repo_map
                    .entry(p.clone())
                    .and_modify(|(_c, _p, deps)| deps.push(dependency.name.clone()));
            }
            go(
                cache,
                dep_map,
                repo_map,
                &descriptor.dependencies,
                Some(&dependency.name),
            )?;
        }

        Ok(())
    }

    let mut dep_map: HashMap<DependencyName, Vec<Revision>> = HashMap::new();
    let mut repo_map: HashMap<DependencyName, (Coordinate, ProtoRepository, Vec<DependencyName>)> =
        HashMap::new();

    go(
        cache,
        &mut dep_map,
        &mut repo_map,
        &descriptor.dependencies,
        None,
    )?;

    let no_conflicts = resolve_conflicts(dep_map);
    let with_repos: HashMap<
        DependencyName,
        (Coordinate, ProtoRepository, Revision, Vec<DependencyName>),
    > = no_conflicts
        .into_iter()
        .filter_map(|(coordinate, revision)| {
            repo_map
                .remove(&coordinate)
                .map(|(dep_name, repo, deps)| (coordinate, (dep_name, repo, revision, deps)))
        })
        .collect();

    let locked_dependencies = locked_dependencies(&with_repos)?;

    Ok(LockFile::new(
        descriptor.name.clone(),
        descriptor.proto_out_dir.clone(),
        locked_dependencies,
    ))
}

pub fn fetch<Cache: RepositoryCache>(
    cache: &Cache,
    lockfile: &LockFile,
    cache_src_dir: &Path,
    proto_out_dir: &Path,
) -> Result<(), FetchError> {
    info!("Fetching dependencies source files...");

    if !cache_src_dir.exists() {
        std::fs::create_dir_all(cache_src_dir)?;
    }

    if cache_src_dir.is_dir() {
        for dep in &lockfile.dependencies {
            //If the dependency is already in the cache, we don't need to fetch it again
            if cache_src_dir
                .join(&dep.name.value)
                .join(PathBuf::from(&dep.commit_hash))
                .exists()
            {
                debug!("Skipping fetching {:?}. Already in cache", dep.name);
                continue;
            }
            let repo = cache.clone_or_update(&dep.coordinate)?;
            let work_tree_res = repo.create_worktrees(
                &lockfile.module_name,
                &dep.name,
                &dep.commit_hash,
                cache_src_dir,
            );
            if let Err(err) = work_tree_res {
                error!("Error while trying to create worktrees {err}. \
                Most likely the worktree sources have been deleted but the worktree metadata has not. \
                Please delete the cache and run protofetch fetch again.")
            }
        }
        //Copy proto files to actual target
        copy_proto_files(proto_out_dir, cache_src_dir, lockfile)?;
        Ok(())
    } else {
        Err(FetchError::BadOutputDir(
            cache_src_dir.to_str().unwrap_or("").to_string(),
        ))
    }
}
/// proto_out_dir: Base path to the directory where the proto files are to be copied to
/// cache_src_dir: Base path to the directory where the dependencies sources are cached
/// lockfile: The lockfile that contains the dependencies to be copied
pub fn copy_proto_files(
    proto_out_dir: &Path,
    cache_src_dir: &Path,
    lockfile: &LockFile,
) -> Result<(), FetchError> {
    info!("Copying proto files described in {}...", lockfile.module_name);
    if !proto_out_dir.exists() {
        std::fs::create_dir_all(proto_out_dir)?;
    }

    for dep in &lockfile.dependencies {
        debug!("Copying proto files for dependency {}", dep.name.value);
        let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
        for file in dep_dir.read_dir()? {
            let path = file?.path();
            let proto_files = find_proto_files(path.as_path())?;
            for proto_file_source in proto_files {
                trace!(
                    "Copying proto file {}",
                    &proto_file_source.to_string_lossy()
                );
                let proto_src = proto_file_source.strip_prefix(&dep_dir).map_err(|_err| {
                    FetchError::BadOutputDir(format!(
                        "Could not create proto source path in {}. Wrong base dir {}",
                        proto_file_source.to_string_lossy(),
                        dep_dir.to_string_lossy()
                    ))
                })?;
                let proto_out_dist = proto_out_dir.join(&proto_src);
                let prefix = proto_out_dist.parent().ok_or_else(|| {
                    FetchError::BadFilePath(format!(
                        "Bad parent dest file for {}",
                        &proto_out_dist.to_string_lossy()
                    ))
                })?;
                std::fs::create_dir_all(prefix)?;
                std::fs::copy(proto_file_source.as_path(), proto_out_dist.as_path())?;
            }
        }
    }
    Ok(())
}

fn find_proto_files(dir: &Path) -> Result<Vec<PathBuf>, FetchError> {
    let mut files: Vec<PathBuf> = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let rec_call = find_proto_files(&path)?;
                files.append(&mut rec_call.clone());
            } else if let Some(extension) = path.extension() {
                if extension == "proto" {
                    files.push(path);
                }
            }
        }
    }
    Ok(files)
}

//TODO: Make sure we get the last version. Getting the biggest string is extremely error prone.
//      Use semver
fn resolve_conflicts(
    dep_map: HashMap<DependencyName, Vec<Revision>>,
) -> HashMap<DependencyName, Revision> {
    dep_map
        .into_iter()
        .filter_map(|(k, mut v)| {
            let len = v.len();

            match v.len() {
                0 => None,
                1 => Some((k, v.remove(0))),
                _ => {
                    log::warn!(
                        "discarded {} dependencies while resolving conflicts for {:?}",
                        len - 1,
                        k
                    );
                    Some((k, v.into_iter().max().unwrap()))
                }
            }
        })
        .collect()
}

pub fn locked_dependencies(
    dep_map: &HashMap<DependencyName, (Coordinate, ProtoRepository, Revision, Vec<DependencyName>)>,
) -> Result<Vec<LockedDependency>, FetchError> {
    let mut locked_deps: Vec<LockedDependency> = Vec::new();
    for (name, (coordinate, repository, revision, deps)) in dep_map {
        log::info!("Locking {:?} at {:?}", coordinate, revision);

        let commit_hash = repository.resolve_commit_hash(revision, coordinate.branch.clone())?;
        let locked_dep = LockedDependency {
            name: name.clone(),
            commit_hash,
            coordinate: coordinate.clone(),
            dependencies: deps.clone(),
        };

        locked_deps.push(locked_dep);
    }

    Ok(locked_deps)
}

#[test]
fn remove_duplicates() {
    let mut input: HashMap<DependencyName, Vec<Revision>> = HashMap::new();
    let mut result: HashMap<DependencyName, Revision> = HashMap::new();
    let name = DependencyName::new("foo".to_string());
    input.insert(name.clone(), vec![
        Revision::Arbitrary {
            revision: "1.0.0".to_string(),
        },
        Revision::Arbitrary {
            revision: "3.0.0".to_string(),
        },
        Revision::Arbitrary {
            revision: "2.0.0".to_string(),
        },
    ]);
    result.insert(name, Revision::Arbitrary {
        revision: "3.0.0".to_string(),
    });
    assert_eq!(resolve_conflicts(input), result)
}
