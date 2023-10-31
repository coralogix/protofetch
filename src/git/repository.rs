use std::{path::PathBuf, str::Utf8Error};

use crate::model::protofetch::{DependencyName, Descriptor, Revision, RevisionSpecification};
use git2::{Oid, Repository, ResetType};
use log::{debug, warn};
use thiserror::Error;

use super::cache::ProtofetchGitCache;

#[derive(Error, Debug)]
pub enum ProtoRepoError {
    #[error("Error while performing revparse in dep {0} for commit {1}: {2}")]
    Revparse(String, String, git2::Error),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Bad git object kind {kind} found for {commit_hash} (expected blob)")]
    BadObjectKind { kind: String, commit_hash: String },
    #[error("Missing protofetch.toml for {commit_hash}")]
    MissingDescriptor { commit_hash: String },
    #[error("Branch {branch} was not found.")]
    BranchNotFound { branch: String },
    #[error("Revision {revision} does not belong to the branch {branch}.")]
    RevisionNotOnBranch { revision: String, branch: String },
    #[error("Worktree with name {name} already exists at {existing_path} but we need it at {wanted_path}")]
    WorktreeExists {
        name: String,
        existing_path: String,
        wanted_path: String,
    },
    #[error("Error while canonicalizing path {path}: {error}")]
    Canonicalization { path: String, error: std::io::Error },
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

pub struct ProtoGitRepository<'a> {
    cache: &'a ProtofetchGitCache,
    git_repo: Repository,
}

impl<'a> ProtoGitRepository<'a> {
    pub fn new(cache: &'a ProtofetchGitCache, git_repo: Repository) -> ProtoGitRepository {
        ProtoGitRepository { cache, git_repo }
    }

    pub fn fetch(&self, _specification: &RevisionSpecification) -> anyhow::Result<()> {
        let mut remote = self.git_repo.find_remote("origin")?;
        // TODO: we only need to fetch refspecs from RevisionSpecification
        let refspecs: Vec<String> = remote
            .refspecs()
            .filter_map(|refspec| refspec.str().map(|s| s.to_string()))
            .collect();
        remote.fetch(&refspecs, Some(&mut self.cache.fetch_options()?), None)?;
        Ok(())
    }

    pub fn fetch_commit(
        &self,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()> {
        let oid = Oid::from_str(commit_hash)?;
        if self.git_repo.find_commit(oid).is_ok() {
            return Ok(());
        }
        let mut remote = self.git_repo.find_remote("origin")?;

        if let Err(error) =
            remote.fetch(&[commit_hash], Some(&mut self.cache.fetch_options()?), None)
        {
            warn!(
                "Failed to fetch a single commit {}, falling back to a full fetch: {}",
                commit_hash, error
            );
            self.fetch(specification)?;
        }

        Ok(())
    }

    pub fn extract_descriptor(
        &self,
        dep_name: &DependencyName,
        commit_hash: &str,
    ) -> Result<Descriptor, ProtoRepoError> {
        let result = self
            .git_repo
            .revparse_single(&format!("{commit_hash}:protofetch.toml"));

        match result {
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                log::debug!("Couldn't find protofetch.toml, assuming module has no dependencies");
                Ok(Descriptor {
                    name: dep_name.value.clone(),
                    description: None,
                    proto_out_dir: None,
                    dependencies: Vec::new(),
                })
            }
            Err(e) => Err(ProtoRepoError::Revparse(
                dep_name.value.to_string(),
                commit_hash.to_owned(),
                e,
            )),
            Ok(obj) => match obj.kind() {
                Some(git2::ObjectType::Blob) => {
                    let blob = obj.peel_to_blob()?;
                    let content = std::str::from_utf8(blob.content())?;
                    let descriptor = Descriptor::from_toml_str(content)?;

                    Ok(descriptor)
                }
                Some(kind) => Err(ProtoRepoError::BadObjectKind {
                    kind: kind.to_string(),
                    commit_hash: commit_hash.to_owned(),
                }),
                None => Err(ProtoRepoError::MissingDescriptor {
                    commit_hash: commit_hash.to_owned(),
                }),
            },
        }
    }

    pub fn resolve_commit_hash(
        &self,
        specification: &RevisionSpecification,
    ) -> Result<String, ProtoRepoError> {
        let RevisionSpecification { branch, revision } = specification;
        let oid = match (branch, revision) {
            (None, Revision::Arbitrary) => self.commit_hash_for_obj_str("HEAD")?,
            (None, Revision::Pinned { revision }) => self.commit_hash_for_obj_str(revision)?,
            (Some(branch), Revision::Arbitrary) => self
                .commit_hash_for_obj_str(&format!("origin/{branch}"))
                .map_err(|_| ProtoRepoError::BranchNotFound {
                    branch: branch.to_owned(),
                })?,
            (Some(branch), Revision::Pinned { revision }) => {
                let branch_commit = self
                    .commit_hash_for_obj_str(&format!("origin/{branch}"))
                    .map_err(|_| ProtoRepoError::BranchNotFound {
                        branch: branch.to_owned(),
                    })?;
                let revision_commit = self.commit_hash_for_obj_str(revision)?;
                if self.is_ancestor(revision_commit, branch_commit)? {
                    revision_commit
                } else {
                    return Err(ProtoRepoError::RevisionNotOnBranch {
                        revision: revision.to_owned(),
                        branch: branch.to_owned(),
                    });
                }
            }
        };
        Ok(oid.to_string())
    }

    pub fn create_worktree(
        &self,
        name: &DependencyName,
        commit_hash: &str,
    ) -> Result<PathBuf, ProtoRepoError> {
        let base_path = self.cache.worktrees_path().join(&name.value);

        if !base_path.exists() {
            std::fs::create_dir_all(&base_path)?;
        }

        let worktree_path = base_path.join(PathBuf::from(commit_hash));
        let worktree_name = commit_hash;

        debug!("Finding worktree {} for {}.", worktree_name, name.value);

        match self.git_repo.find_worktree(worktree_name) {
            Ok(worktree) => {
                let canonical_existing_path = worktree.path().canonicalize().map_err(|e| {
                    ProtoRepoError::Canonicalization {
                        path: worktree.path().to_string_lossy().to_string(),
                        error: e,
                    }
                })?;

                let canonical_wanted_path =
                    worktree_path
                        .canonicalize()
                        .map_err(|e| ProtoRepoError::Canonicalization {
                            path: worktree_path.to_string_lossy().to_string(),
                            error: e,
                        })?;

                if canonical_existing_path != canonical_wanted_path {
                    return Err(ProtoRepoError::WorktreeExists {
                        name: worktree_name.to_string(),
                        existing_path: worktree.path().to_str().unwrap_or("").to_string(),
                        wanted_path: worktree_path.to_str().unwrap_or("").to_string(),
                    });
                } else {
                    log::info!(
                        "Found existing worktree for {} at {}.",
                        name.value,
                        canonical_wanted_path.to_string_lossy()
                    );
                }
            }
            Err(_) => {
                log::info!(
                    "Creating new worktree for {} at {}.",
                    name.value,
                    worktree_path.to_string_lossy()
                );

                self.git_repo
                    .worktree(worktree_name, &worktree_path, None)?;
            }
        };

        let worktree_repo = Repository::open(&worktree_path)?;
        let worktree_head_object = worktree_repo.revparse_single(commit_hash)?;

        worktree_repo.reset(&worktree_head_object, ResetType::Hard, None)?;

        Ok(worktree_path)
    }

    fn commit_hash_for_obj_str(&self, str: &str) -> Result<Oid, ProtoRepoError> {
        Ok(self.git_repo.revparse_single(str)?.peel_to_commit()?.id())
    }

    // Check if `a` is an ancestor of `b`
    fn is_ancestor(&self, a: Oid, b: Oid) -> Result<bool, ProtoRepoError> {
        Ok(self.git_repo.merge_base(a, b)? == a)
    }
}
