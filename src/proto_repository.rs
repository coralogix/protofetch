use std::{
    path::{Path, PathBuf},
    str::Utf8Error,
};

use crate::model::protofetch::{Descriptor, Revision};
use git2::{Repository, ResetType};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtoRepoError {
    #[error("Error while performing revparse in dep {0} for revision {1}: {2}")]
    Revparse(String, String, git2::Error),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Bad git object kind {kind} found for {revision} (expected blob)")]
    BadObjectKind { kind: String, revision: String },
    #[error("Missing `module.toml` for revision {revision}")]
    MissingDescriptor { revision: String },
    #[error("Branch {branch} was not found.")]
    BranchNotFound { branch: String },
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

pub struct ProtoRepository {
    git_repo: Repository,
}

impl ProtoRepository {
    pub fn new(git_repo: Repository) -> ProtoRepository {
        ProtoRepository { git_repo }
    }

    pub fn extract_descriptor(
        &self,
        dep_name: &str,
        revision: &Revision,
    ) -> Result<Descriptor, ProtoRepoError> {
        let rendered_revision = revision.to_string();
        let result = self
            .git_repo
            .revparse_single(&format!("{}:protofetch.toml", rendered_revision));

        match result {
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                log::debug!("Couldn't find protofetch.toml, assuming module has no dependencies");
                Ok(Descriptor {
                    name: dep_name.to_string(),
                    description: None,
                    proto_out_dir: None,
                    dependencies: Vec::new(),
                })
            }
            Err(e) => Err(ProtoRepoError::Revparse(
                dep_name.to_string(),
                rendered_revision,
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
                    revision: rendered_revision,
                }),
                None => Err(ProtoRepoError::MissingDescriptor {
                    revision: rendered_revision,
                }),
            },
        }
    }

    pub fn create_worktrees(
        &self,
        module_name: &str,
        dep_name: &str,
        commit_hash: &str,
        out_dir: &Path,
    ) -> Result<(), ProtoRepoError> {
        let base_path = out_dir.join(PathBuf::from(dep_name));

        if !base_path.exists() {
            std::fs::create_dir(&base_path)?;
        }

        let worktree_path = base_path.join(PathBuf::from(commit_hash));
        let worktree_name = commit_hash;

        debug!(
            "Module[{}] Finding worktree {} for dep {}.",
            module_name, worktree_name, dep_name
        );

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
                        "Module[{}] Found existing worktree for dep {} at {}.",
                        module_name,
                        dep_name,
                        canonical_wanted_path.to_string_lossy()
                    );
                }
            }
            Err(_) => {
                log::info!(
                    "Module[{}] Creating new worktree for dep {} at {}.",
                    module_name,
                    dep_name,
                    worktree_path.to_string_lossy()
                );

                self.git_repo
                    .worktree(worktree_name, &worktree_path, None)?;
            }
        };

        let worktree_repo = Repository::open(worktree_path)?;
        let worktree_head_object = worktree_repo.revparse_single(commit_hash)?;

        worktree_repo.reset(&worktree_head_object, ResetType::Hard, None)?;

        Ok(())
    }

    pub fn resolve_commit_hash(
        &self,
        revision: &Revision,
        branch: Option<String>,
    ) -> Result<String, ProtoRepoError> {
        match branch {
            Some(branch) => {
                info!(
                    "Found branch! Fetching commit hash for branch {} instead of revision {}.",
                    &branch,
                    &revision.to_string()
                );
                let branch_str = format!("origin/{}", branch);
                Self::commit_hash_for_obj_str(&self.git_repo, &branch_str)
                    .map_err(|_err| ProtoRepoError::BranchNotFound { branch })
            }
            None => Self::commit_hash_for_obj_str(&self.git_repo, &revision.to_string()),
        }
    }

    fn commit_hash_for_obj_str(repo: &Repository, str: &str) -> Result<String, ProtoRepoError> {
        let str = repo
            .revparse_single(str)?
            .peel_to_commit()?
            .id()
            .to_string();
        Ok(str)
    }
}
