pub mod error;
pub mod types;

pub mod libgit2;

#[cfg(feature = "git-backend-cli")]
pub mod cli;

use std::{
    panic::{RefUnwindSafe, UnwindSafe},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::bail;
use log::info;
use serde::Deserialize;

use error::GitBackendError;
use types::GitOid;

/// Per-repository operations. Obtained from a [`GitBackend`].
/// The implementing type stores the repository path internally.
pub trait GitRepository {
    fn remote_add(&self, name: &str, url: &str) -> Result<(), GitBackendError>;
    fn remote_get_url(&self, name: &str) -> Result<Option<String>, GitBackendError>;
    fn remote_set_url(&self, name: &str, url: &str) -> Result<(), GitBackendError>;
    fn fetch(&self, remote_name: &str, refspecs: &[String]) -> Result<(), GitBackendError>;
    fn commit_exists(&self, oid: &str) -> Result<bool, GitBackendError>;
    fn revparse_commit(&self, spec: &str) -> Result<GitOid, GitBackendError>;
    fn read_blob(&self, commit: &str, blob_path: &str) -> Result<Option<Vec<u8>>, GitBackendError>;
    /// Check if `ancestor` is an ancestor of `descendant`.
    fn is_ancestor(&self, ancestor: &GitOid, descendant: &GitOid) -> Result<bool, GitBackendError>;
    fn create_worktree(
        &self,
        name: &str,
        worktree_path: &Path,
        commit: &str,
    ) -> Result<WorktreeResult, GitBackendError>;
    /// Hard-reset the repository to a specific commit.
    fn reset(&self, commit: &str) -> Result<(), GitBackendError>;
}

/// Factory for opening or creating git repositories.
pub trait GitBackend: Send + Sync + UnwindSafe + RefUnwindSafe {
    /// Initialize a new bare repository at the given path and return a handle to it.
    fn init_bare(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError>;

    /// Open an existing repository at the given path and return a handle to it.
    /// Returns an error if no repository exists there.
    fn open(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError>;
}

/// Result of a worktree creation attempt.
pub enum WorktreeResult {
    /// A new worktree was created. The inner value is a repository handle for the worktree.
    Created(Box<dyn GitRepository>),
    /// An existing worktree was found at the given canonical path.
    Existing(PathBuf, Box<dyn GitRepository>),
}

impl std::fmt::Debug for WorktreeResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeResult::Created(_) => write!(f, "Created(..)"),
            WorktreeResult::Existing(p, _) => write!(f, "Existing({:?}, ..)", p),
        }
    }
}

/// The type of git backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
pub enum GitBackendType {
    #[default]
    #[serde(rename = "libgit2")]
    Libgit2,
    #[cfg(feature = "git-backend-cli")]
    #[serde(rename = "cli")]
    Cli,
}

impl FromStr for GitBackendType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "libgit2" => Ok(GitBackendType::Libgit2),
            #[cfg(feature = "git-backend-cli")]
            "cli" => Ok(GitBackendType::Cli),
            _ => bail!("invalid git backend type: {s}"),
        }
    }
}

/// Create a git backend of the specified type.
pub fn create_backend(
    backend_type: GitBackendType,
    git_executable: Option<String>,
) -> Box<dyn GitBackend> {
    match backend_type {
        GitBackendType::Libgit2 => {
            info!("Using libgit2 git backend");
            Box::new(libgit2::Libgit2Backend::new())
        }
        #[cfg(feature = "git-backend-cli")]
        GitBackendType::Cli => {
            info!("Using git CLI backend");
            Box::new(cli::CliBackend::new(
                git_executable.unwrap_or_else(|| "git".to_string()),
            ))
        }
    }
}
