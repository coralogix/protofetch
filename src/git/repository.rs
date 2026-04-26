use std::path::{Path, PathBuf};

use crate::model::protofetch::{Descriptor, ModuleName, Revision, RevisionSpecification};
use log::{debug, warn};
use thiserror::Error;

use super::backend::{error::GitBackendError, GitRepository, WorktreeResult};

#[derive(Error, Debug)]
pub enum ProtoRepoError {
    #[error("Error while performing revparse in dep {0} for commit {1}: {2}")]
    Revparse(ModuleName, String, GitBackendError),
    #[error("Git backend error: {0}")]
    BackendError(#[from] GitBackendError),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] std::str::Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
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

pub struct ProtoGitRepository {
    repo: Box<dyn GitRepository>,
    origin: String,
    worktrees_base: PathBuf,
}

impl ProtoGitRepository {
    pub fn new(
        repo: Box<dyn GitRepository>,
        origin: String,
        worktrees_base: &Path,
    ) -> ProtoGitRepository {
        ProtoGitRepository {
            repo,
            origin,
            worktrees_base: worktrees_base.to_path_buf(),
        }
    }

    pub fn fetch(&self, specification: &RevisionSpecification) -> anyhow::Result<()> {
        let mut refspecs = Vec::with_capacity(3);
        if let Revision::Pinned { revision } = &specification.revision {
            refspecs.push(format!("+refs/tags/{}:refs/tags/{}", revision, revision));
            refspecs.push("+refs/heads/*:refs/remotes/origin/*".to_owned());
        }
        if let Some(branch) = &specification.branch {
            refspecs.push(format!(
                "+refs/heads/{}:refs/remotes/origin/{}",
                branch, branch
            ));
        }

        debug!("Fetching {:?} from {}", refspecs, self.origin);
        self.repo.fetch("origin", &refspecs)?;
        Ok(())
    }

    pub fn fetch_commit(
        &self,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()> {
        if self.repo.commit_exists(commit_hash)? {
            return Ok(());
        }

        debug!("Fetching {} from {}", commit_hash, self.origin);
        if let Err(error) = self.repo.fetch("origin", &[commit_hash.to_string()]) {
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
        dep_name: &ModuleName,
        commit_hash: &str,
    ) -> Result<Descriptor, ProtoRepoError> {
        let result = self.repo.read_blob(commit_hash, "protofetch.toml");

        match result {
            Ok(None) => {
                log::debug!("Couldn't find protofetch.toml, assuming module has no dependencies");
                Ok(Descriptor {
                    name: dep_name.clone(),
                    description: None,
                    proto_out_dir: None,
                    dependencies: Vec::new(),
                })
            }
            Err(GitBackendError::NotFound(_)) => {
                log::debug!("Couldn't find protofetch.toml, assuming module has no dependencies");
                Ok(Descriptor {
                    name: dep_name.clone(),
                    description: None,
                    proto_out_dir: None,
                    dependencies: Vec::new(),
                })
            }
            Err(e) => Err(ProtoRepoError::Revparse(
                dep_name.to_owned(),
                commit_hash.to_owned(),
                e,
            )),
            Ok(Some(blob)) => {
                let content = std::str::from_utf8(&blob)?;
                let descriptor = Descriptor::from_toml_str(content)?;
                Ok(descriptor)
            }
        }
    }

    pub fn resolve_commit_hash(
        &self,
        specification: &RevisionSpecification,
    ) -> Result<String, ProtoRepoError> {
        let RevisionSpecification { branch, revision } = specification;
        let oid = match (branch, revision) {
            (None, Revision::Arbitrary) => self.repo.revparse_commit("HEAD")?,
            (None, Revision::Pinned { revision }) => self.repo.revparse_commit(revision)?,
            (Some(branch), Revision::Arbitrary) => self
                .repo
                .revparse_commit(&format!("origin/{branch}"))
                .map_err(|_| ProtoRepoError::BranchNotFound {
                    branch: branch.to_owned(),
                })?,
            (Some(branch), Revision::Pinned { revision }) => {
                let branch_commit = self
                    .repo
                    .revparse_commit(&format!("origin/{branch}"))
                    .map_err(|_| ProtoRepoError::BranchNotFound {
                        branch: branch.to_owned(),
                    })?;
                let revision_commit = self.repo.revparse_commit(revision)?;
                if self.repo.is_ancestor(&revision_commit, &branch_commit)? {
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
        name: &ModuleName,
        commit_hash: &str,
    ) -> Result<PathBuf, ProtoRepoError> {
        let base_path = self.worktrees_base.join(name.as_str());

        if !base_path.exists() {
            std::fs::create_dir_all(&base_path)?;
        }

        let worktree_path = base_path.join(PathBuf::from(commit_hash));
        let worktree_name = commit_hash;

        debug!("Finding worktree {} for {}.", worktree_name, name);

        match self
            .repo
            .create_worktree(worktree_name, &worktree_path, commit_hash)?
        {
            WorktreeResult::Created(worktree_repo) => {
                worktree_repo.reset(commit_hash)?;
            }
            WorktreeResult::Existing(canonical_existing_path) => {
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
                        existing_path: canonical_existing_path.to_str().unwrap_or("").to_string(),
                        wanted_path: worktree_path.to_str().unwrap_or("").to_string(),
                    });
                } else {
                    log::debug!(
                        "Found existing worktree for {} at {}.",
                        name,
                        canonical_wanted_path.to_string_lossy()
                    );
                }
            }
        }

        Ok(worktree_path)
    }
}
