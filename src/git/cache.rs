use std::path::{Path, PathBuf};

use log::{debug, info, trace};
use thiserror::Error;

use crate::{
    flock::FileLock,
    git::{
        backend::{create_backend, GitBackend, GitBackendType, GitRepository},
        repository::ProtoGitRepository,
    },
    model::protofetch::{Coordinate, Protocol},
};

const WORKTREES_DIR: &str = "dependencies";

pub struct ProtofetchGitCache {
    location: PathBuf,
    worktrees: PathBuf,
    default_protocol: Protocol,
    backend: Box<dyn GitBackend>,
    _lock: FileLock,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git backend error: {0}")]
    Backend(#[from] crate::git::backend::error::GitBackendError),
    #[error("Cache location {location} does not exist")]
    BadLocation { location: String },
    #[error("Cache lock cannot be acquired")]
    Lock(#[from] crate::flock::Error),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

impl ProtofetchGitCache {
    pub fn new(
        location: PathBuf,
        default_protocol: Protocol,
        backend_type: GitBackendType,
    ) -> Result<ProtofetchGitCache, CacheError> {
        if location.exists() {
            if !location.is_dir() {
                return Err(CacheError::BadLocation {
                    location: location.to_str().unwrap_or("").to_string(),
                });
            }
        } else {
            std::fs::create_dir_all(&location)?;
        }

        let lock = Self::acquire_lock(&location)?;

        let worktrees = location.join(WORKTREES_DIR);
        let backend = create_backend(backend_type);

        Ok(ProtofetchGitCache {
            location,
            worktrees,
            default_protocol,
            backend,
            _lock: lock,
        })
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        if self.location.exists() {
            info!(
                "Clearing protofetch repository cache {}.",
                &self.location.display()
            );
            std::fs::remove_dir_all(&self.location)?;
        }
        Ok(())
    }

    pub fn repository(&self, entry: &Coordinate) -> Result<ProtoGitRepository, CacheError> {
        let mut path = self.location.clone();
        path.push(entry.to_path());

        let url = entry.to_git_url(self.default_protocol);

        let repo = if path.exists() {
            self.open_entry(&path, &url)?
        } else {
            self.create_repo(&path, &url)?
        };

        Ok(ProtoGitRepository::new(repo, url, &self.worktrees))
    }

    fn acquire_lock(location: &Path) -> Result<FileLock, CacheError> {
        let location = location.join(".lock");
        debug!(
            "Acquiring a lock on the cache location: {}",
            location.display()
        );
        let lock = FileLock::new(&location)?;
        info!("Acquired a lock on the cache location");
        Ok(lock)
    }

    fn open_entry(&self, path: &Path, url: &str) -> Result<Box<dyn GitRepository>, CacheError> {
        trace!("Opening existing repository at {}", path.display());

        let repo = self.backend.open(path)?;

        let current_url = repo.remote_get_url("origin")?;
        if current_url.as_deref() != Some(url) {
            trace!(
                "Updating remote existing url {:?} to new url {}",
                current_url,
                url
            );
            repo.remote_set_url("origin", url)?;
        }

        Ok(repo)
    }

    fn create_repo(&self, path: &Path, url: &str) -> Result<Box<dyn GitRepository>, CacheError> {
        trace!("Creating a new repository at {}", path.display());

        std::fs::create_dir_all(path)?;
        let repo = self.backend.init_bare(path)?;
        repo.remote_add("origin", url)?;

        Ok(repo)
    }
}
