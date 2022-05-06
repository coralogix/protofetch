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

use crate::model::protofetch::{DependencyName, Descriptor, Rules};
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
        repo_map: &mut HashMap<
            DependencyName,
            (Rules, Coordinate, ProtoRepository, Vec<DependencyName>),
        >,
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
                dependency.rules.clone(),
                dependency.coordinate.clone(),
                repo,
                vec![],
            ));

            if let Some(p) = parent {
                repo_map
                    .entry(p.clone())
                    .and_modify(|(_r, _c, _p, deps)| deps.push(dependency.name.clone()));
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
    let mut repo_map: HashMap<
        DependencyName,
        (Rules, Coordinate, ProtoRepository, Vec<DependencyName>),
    > = HashMap::new();

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
        (
            Rules,
            Coordinate,
            ProtoRepository,
            Revision,
            Vec<DependencyName>,
        ),
    > = no_conflicts
        .into_iter()
        .filter_map(|(coordinate, revision)| {
            repo_map
                .remove(&coordinate)
                .map(|(rules, dep_name, repo, deps)| {
                    (coordinate, (rules, dep_name, repo, revision, deps))
                })
        })
        .collect();

    let locked_dependencies = locked_dependencies(&with_repos)?;

    Ok(LockFile::new(
        descriptor.name.clone(),
        descriptor.proto_out_dir.clone(),
        locked_dependencies,
    ))
}

pub fn fetch_sources<Cache: RepositoryCache>(
    cache: &Cache,
    lockfile: &LockFile,
    cache_src_dir: &Path,
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
        Ok(())
    } else {
        Err(FetchError::BadOutputDir(
            cache_src_dir.to_str().unwrap_or("").to_string(),
        ))
    }
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

fn locked_dependencies(
    dep_map: &HashMap<
        DependencyName,
        (
            Rules,
            Coordinate,
            ProtoRepository,
            Revision,
            Vec<DependencyName>,
        ),
    >,
) -> Result<Vec<LockedDependency>, FetchError> {
    let mut locked_deps: Vec<LockedDependency> = Vec::new();
    for (name, (rules, coordinate, repository, revision, deps)) in dep_map {
        log::info!("Locking {:?} at {:?}", coordinate, revision);

        let commit_hash = repository.resolve_commit_hash(revision, coordinate.branch.clone())?;
        let locked_dep = LockedDependency {
            name: name.clone(),
            commit_hash,
            coordinate: coordinate.clone(),
            dependencies: deps.clone(),
            rules: rules.clone(),
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
