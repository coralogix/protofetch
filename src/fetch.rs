use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::{from_utf8, Utf8Error};

use git2::{Repository, ResetType};

use crate::cache::{CacheError, RepositoryCache};
use crate::model::{Coordinate, Dependency, Descriptor, LockFile, LockedDependency, Revision};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache: {0}")]
    Cache(#[from] CacheError),
    #[error("Error while performing revparse in dep {0} for revision {1}: {2}")]
    Revparse(String, String, git2::Error),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] Utf8Error),
    #[error("Bad git object kind {kind} found for {revision} (expected blob)")]
    BadObjectKind { kind: String, revision: String },
    #[error("Missing `module.toml` for revision {revision}")]
    MissingDescriptor { revision: String },
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Missing output dir {0}")]
    MissingOutputDir(String),
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
        repo_map: &mut HashMap<Coordinate, Repository>,
        dependencies: &[Dependency],
    ) -> Result<(), FetchError> {
        for dependency in dependencies {
            eprintln!("Resolving {:?}", dependency.coordinate);

            dep_map
                .entry(dependency.coordinate.clone())
                .and_modify(|vec| vec.push(dependency.revision.clone()))
                .or_insert_with(|| vec![dependency.revision.clone()]);

            let repo = cache.clone_or_update(&dependency.coordinate)?;
            let descriptor = extract_descriptor(&dependency.name, &repo, &dependency.revision)?;

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
    let mut with_repos: HashMap<Coordinate, (Repository, Revision)> = no_conflicts
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

fn extract_descriptor(
    dep_name: &str,
    repo: &Repository,
    revision: &Revision,
) -> Result<Descriptor, FetchError> {
    let rendered_revision = revision.to_string();
    let result = repo.revparse_single(&format!("{}:module.toml", rendered_revision));
    //.map_err(FetchError::Revparse)?;

    match result {
        Err(e) => {
            if let git2::ErrorCode::NotFound = e.code() {
                eprintln!("Couldn't find module.toml, assuming module has no dependencies");
                Ok(Descriptor {
                    name: dep_name.to_string(),
                    dependencies: Vec::new(),
                })
            } else {
                Err(FetchError::Revparse(
                    dep_name.to_string(),
                    rendered_revision,
                    e,
                ))
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

pub fn create_worktrees(
    self_module_name: &str,
    out_dir: &Path,
    dep_map: &HashMap<String, (git2::Repository, String)>,
) -> Result<(), FetchError> {
    if !out_dir.exists() {
        Err(FetchError::MissingOutputDir(
            out_dir.to_str().unwrap_or("").to_string(),
        ))
    } else {
        for (dep_name, (repo, commit)) in dep_map {
            let worktree_path: PathBuf = out_dir.join(PathBuf::from(dep_name));
            repo.worktree(
                &format!("{}_{}", self_module_name, dep_name),
                &worktree_path,
                None,
            )?;

            let worktree_repo = Repository::open(worktree_path)?;
            let worktree_head_object = worktree_repo.revparse_single(commit)?;

            eprintln!("Object {:?}", worktree_head_object);
            eprintln!("Revparsed {}", commit);

            worktree_repo.reset(&worktree_head_object, ResetType::Hard, None)?;
        }

        Ok(())
    }
}
