use std::path::{Path, PathBuf};

use log::{debug, info, trace};
use thiserror::Error;

use crate::{
    flock::FileLock,
    git::{
        backend::{
            create_backend, error::GitBackendError, GitBackend, GitBackendType, GitRepository,
        },
        coord_locks::CoordinateLocks,
        repository::ProtoGitRepository,
    },
    model::protofetch::{Coordinate, Protocol},
};

const CACHE_VERSION: &str = "v2";

pub struct ProtofetchGitCache {
    unversioned_location: PathBuf,
    default_protocol: Protocol,
    coord_locks: CoordinateLocks,
    backend: Box<dyn GitBackend>,
    _lock: FileLock,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git backend error: {0}")]
    Backend(#[from] GitBackendError),
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
        git_executable: Option<String>,
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
        let backend = create_backend(backend_type, git_executable);

        Ok(ProtofetchGitCache {
            unversioned_location: location,
            default_protocol,
            coord_locks: CoordinateLocks::default(),
            backend,
            _lock: lock,
        })
    }

    pub fn coord_locks(&self) -> &CoordinateLocks {
        &self.coord_locks
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        if self.unversioned_location.exists() {
            info!(
                "Clearing protofetch repository cache {}",
                &self.unversioned_location.display()
            );
            std::fs::remove_dir_all(&self.unversioned_location)?;
        }
        Ok(())
    }

    pub fn repository(&self, entry: &Coordinate) -> Result<ProtoGitRepository, CacheError> {
        let mut path = self.repositories_path();
        path.push(entry.to_path());

        let url = entry.to_git_url(self.default_protocol);

        let repo = if path.exists() {
            self.open_entry(&path, &url)?
        } else {
            self.create_repo(&path, &url)?
        };

        let worktrees = self.worktrees_path();
        Ok(ProtoGitRepository::new(repo, url, &worktrees))
    }

    fn root_path(&self) -> PathBuf {
        self.unversioned_location.join(CACHE_VERSION)
    }

    fn repositories_path(&self) -> PathBuf {
        let mut path = self.root_path();
        path.push("repositories");
        path
    }

    pub fn worktrees_path(&self) -> PathBuf {
        let mut path = self.root_path();
        path.push("worktrees");
        path
    }

    fn acquire_lock(root: &Path) -> Result<FileLock, CacheError> {
        let location = root.join(".lock");
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
            // If true then the protocol was updated before updating the cache.
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

#[allow(dead_code)]
fn _assert_traits() {
    fn assert<T: Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe>() {}
    assert::<ProtofetchGitCache>();
}
