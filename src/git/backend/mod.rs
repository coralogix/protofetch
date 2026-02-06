pub mod error;
pub mod types;

use std::path::{Path, PathBuf};

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
    fn is_ancestor(&self, ancestor: &GitOid, descendant: &GitOid) -> Result<bool, GitBackendError>;
    fn create_worktree(
        &self,
        name: &str,
        worktree_path: &Path,
        commit: &str,
    ) -> Result<WorktreeResult, GitBackendError>;
    fn find_worktree(&self, name: &str) -> Result<Option<PathBuf>, GitBackendError>;
    /// Hard-reset the repository to a specific commit.
    fn reset(&self, commit: &str) -> Result<(), GitBackendError>;
}

/// Factory for opening or creating git repositories.
pub trait GitBackend {
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
    Existing(PathBuf),
}

impl std::fmt::Debug for WorktreeResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeResult::Created(_) => write!(f, "Created(..)"),
            WorktreeResult::Existing(p) => write!(f, "Existing({:?})", p),
        }
    }
}

