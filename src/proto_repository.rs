use std::{
    path::{Path, PathBuf},
    str::Utf8Error,
};

use crate::model::{Descriptor, Revision};
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
            .revparse_single(&format!("{}:module.toml", rendered_revision));
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
                    Err(ProtoRepoError::Revparse(
                        dep_name.to_string(),
                        rendered_revision,
                        e,
                    ))
                }
            }
            Ok(obj) => match obj.kind() {
                Some(git2::ObjectType::Blob) => {
                    let blob = obj.peel_to_blob()?;
                    let content = std::str::from_utf8(blob.content())?;
                    let descriptor = Descriptor::from_str(content)?;

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
        self_name: &str,
        worktree_name_prefix: &str,
        revision: &str,
        out_dir: &Path,
    ) -> Result<(), ProtoRepoError> {
        let worktree_path: PathBuf = out_dir.join(PathBuf::from(self_name));
        self.git_repo.worktree(
            &format!("{}_{}", &worktree_name_prefix, self_name),
            &worktree_path,
            None,
        )?;

        let worktree_repo = Repository::open(worktree_path)?;
        let worktree_head_object = worktree_repo.revparse_single(revision)?;

        worktree_repo.reset(&worktree_head_object, ResetType::Hard, None)?;

        Ok(())
    }

    pub fn resolve_revision(&self, revision: &Revision) -> Result<String, ProtoRepoError> {
        Ok(self
            .git_repo
            .revparse_single(&revision.to_string())?
            .peel_to_commit()?
            .id()
            .to_string())
    }
}
