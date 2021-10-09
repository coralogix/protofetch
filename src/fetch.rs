use std::collections::HashMap;
use std::path::Path;
use std::str::Utf8Error;

use crate::cache::{CacheError, RepositoryCache};
use crate::model::{Coordinate, Dependency, LockFile, LockedDependency, Revision};
use crate::proto_repository::ProtoRepository;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache: {0}")]
    Cache(#[from] CacheError),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Missing output dir {0}")]
    MissingOutputDir(String),
    #[error("Error while processing protobuf repository")]
    ProtoRepoError(#[from] crate::proto_repository::ProtoRepoError),
}

pub fn lock<Cache: RepositoryCache>(
    self_module_name: &str,
    out_dir: &Path,
    cache: &Cache,
    dependencies: &[Dependency],
) -> Result<LockFile, FetchError> {
    fn go<Cache: RepositoryCache>(
        cache: &Cache,
        dep_map: &mut HashMap<Coordinate, Vec<Revision>>,
        repo_map: &mut HashMap<Coordinate, ProtoRepository>,
        dependencies: &[Dependency],
    ) -> Result<(), FetchError> {
        for dependency in dependencies {
            eprintln!("Resolving {:?}", dependency.coordinate);

            dep_map
                .entry(dependency.coordinate.clone())
                .and_modify(|vec| vec.push(dependency.revision.clone()))
                .or_insert_with(|| vec![dependency.revision.clone()]);

            let repo = cache.clone_or_update(&dependency.coordinate)?;
            let descriptor = repo.extract_descriptor(&dependency.name, &dependency.revision)?;

            repo_map
                .entry(dependency.coordinate.clone())
                .or_insert(repo);

            go(cache, dep_map, repo_map, &descriptor.dependencies)?;
        }

        Ok(())
    }

    let mut dep_map: HashMap<Coordinate, Vec<Revision>> = HashMap::new();
    let mut repo_map: HashMap<Coordinate, ProtoRepository> = HashMap::new();

    go(cache, &mut dep_map, &mut repo_map, dependencies)?;

    let no_conflicts = resolve_conflicts(dep_map);
    let mut with_repos: HashMap<Coordinate, (ProtoRepository, Revision)> = no_conflicts
        .into_iter()
        .filter_map(|(coordinate, revision)| {
            repo_map
                .remove(&coordinate)
                .map(|repo| (coordinate, (repo, revision)))
        })
        .collect();

    let lockfile = lock_dependencies(&with_repos)?;

    let for_worktrees = lockfile
        .dependencies
        .clone()
        .into_iter()
        .filter_map(|locked_dep| {
            with_repos.remove(&locked_dep.coordinate).map(|tp| {
                (
                    locked_dep.coordinate.repository,
                    (tp.0, locked_dep.commit_hash),
                )
            })
        })
        .collect::<HashMap<_, _>>();

    create_worktrees(self_module_name, out_dir, &for_worktrees)?;

    Ok(lockfile)
}

fn resolve_conflicts(dep_map: HashMap<Coordinate, Vec<Revision>>) -> HashMap<Coordinate, Revision> {
    dep_map
        .into_iter()
        .filter_map(|(k, mut v)| {
            let len = v.len();

            match v.len() {
                0 => None,
                1 => Some((k, v.remove(0))),
                _ => {
                    eprintln!(
                        "Warning: discarded {} dependencies while resolving conflicts for {}",
                        len - 1,
                        k
                    );
                    Some((k, v.remove(0)))
                }
            }
        })
        .collect()
}

pub fn lock_dependencies(
    dep_map: &HashMap<Coordinate, (ProtoRepository, Revision)>,
) -> Result<LockFile, FetchError> {
    let mut locked_deps: Vec<LockedDependency> = Vec::new();
    for (coordinate, (repository, revision)) in dep_map {
        eprintln!("Locking {:?} at {:?}", coordinate, revision);

        let commit_hash = repository.resolve_revision(revision)?;
        let locked_dep = LockedDependency {
            coordinate: coordinate.clone(),
            commit_hash,
        };

        locked_deps.push(locked_dep);
    }

    Ok(LockFile {
        dependencies: locked_deps,
    })
}

pub fn create_worktrees(
    self_module_name: &str,
    out_dir: &Path,
    dep_map: &HashMap<String, (ProtoRepository, String)>,
) -> Result<(), FetchError> {
    if !out_dir.exists() {
        Err(FetchError::MissingOutputDir(
            out_dir.to_str().unwrap_or("").to_string(),
        ))
    } else {
        for (dep_name, (repo, commit)) in dep_map {
            repo.create_worktrees(dep_name, self_module_name, commit, out_dir)?;
        }

        Ok(())
    }
}
