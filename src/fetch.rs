use std::collections::HashMap;
use std::str::{from_utf8, Utf8Error};

use git2::Repository;

use crate::cache::{CacheError, ProtofetchCache};
use crate::model::{Coordinate, Dependency, Descriptor, LockFile, LockedDependency, Revision};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache: {0}")]
    Cache(#[from] CacheError),
    #[error("Error while performing revparse: {0}")]
    Revparse(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] Utf8Error),
    #[error("Bad git object kind {kind} found for {revision} (expected blob)")]
    BadObjectKind { kind: String, revision: String },
    #[error("Missing `module.toml` for revision {revision}")]
    MissingDescriptor { revision: String },
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
}

pub fn lock(cache: &ProtofetchCache, dependencies: &[Dependency]) -> Result<LockFile, FetchError> {
    fn go(
        cache: &ProtofetchCache,
        dep_map: &mut HashMap<Coordinate, Vec<Revision>>,
        repo_map: &mut HashMap<Coordinate, Repository>,
        dependencies: &[Dependency],
    ) -> Result<(), FetchError> {
        for dependency in dependencies {
            eprintln!("Resolving {:?}", dependency.coordinate);

            dep_map
                .entry(dependency.coordinate.clone())
                .and_modify(|vec| vec.push(dependency.revision.clone()))
                .or_insert_with(|| vec![dependency.revision.clone()]);

            let repo = cache.clone_or_fetch(&dependency.coordinate)?;
            let descriptor = extract_descriptor(&repo, &dependency.revision)?;

            repo_map
                .entry(dependency.coordinate.clone())
                .or_insert(repo);

            go(cache, dep_map, repo_map, &descriptor.dependencies)?;
        }

        Ok(())
    }

    let mut dep_map: HashMap<Coordinate, Vec<Revision>> = HashMap::new();
    let mut repo_map: HashMap<Coordinate, Repository> = HashMap::new();

    go(cache, &mut dep_map, &mut repo_map, dependencies)?;

    let no_conflicts = resolve_conflicts(dep_map);
    let with_repos: HashMap<Coordinate, (Repository, Revision)> = no_conflicts
        .into_iter()
        .filter_map(|(coordinate, revision)| {
            repo_map
                .remove(&coordinate)
                .map(|repo| (coordinate, (repo, revision)))
        })
        .collect();

    lock_dependencies(&with_repos)
}

fn extract_descriptor(repo: &Repository, revision: &Revision) -> Result<Descriptor, FetchError> {
    let rendered_revision = revision.to_string();
    let result = repo.revparse_single(&format!("{}:module.toml", rendered_revision));
    //.map_err(FetchError::Revparse)?;

    match result {
        Err(e) => {
            if let git2::ErrorCode::NotFound = e.code() {
                eprintln!("Couldn't find module.toml, assuming module has no dependencies");
                Ok(Descriptor {
                    dependencies: Vec::new(),
                })
            } else {
                Err(FetchError::Revparse(e))
            }
        }
        Ok(obj) => match obj.kind() {
            Some(git2::ObjectType::Blob) => {
                let blob = obj.peel_to_blob()?;
                let content = from_utf8(blob.content())?;
                let descriptor = Descriptor::from_str(content)?;

                Ok(descriptor)
            }
            Some(kind) => Err(FetchError::BadObjectKind {
                kind: kind.to_string(),
                revision: rendered_revision,
            }),
            None => Err(FetchError::MissingDescriptor {
                revision: rendered_revision,
            }),
        },
    }
}

fn resolve_conflicts(dep_map: HashMap<Coordinate, Vec<Revision>>) -> HashMap<Coordinate, Revision> {
    dep_map
        .into_iter()
        .filter_map(|(k, mut v)| {
            let len = v.len();

            if len > 1 {
                eprintln!(
                    "Warning: discarded {} dependencies while resolving conflicts for {}",
                    len - 1,
                    k
                );
                Some((k, v.remove(0)))
            } else if len == 1 {
                Some((k, v.remove(0)))
            } else {
                None
            }
        })
        .collect()
}

pub fn lock_dependencies(
    dep_map: &HashMap<Coordinate, (git2::Repository, Revision)>,
) -> Result<LockFile, FetchError> {
    let mut locked_deps: Vec<LockedDependency> = Vec::new();
    for (coordinate, (repository, revision)) in dep_map {
        eprintln!("Locking {:?} at {:?}", coordinate, revision);

        let commit_hash = repository
            .revparse_single(&revision.to_string())?
            .peel_to_commit()?
            .id()
            .to_string();
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
