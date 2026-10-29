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
    lock: FileLock,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git backend error: {0}")]
    Backend(#[from] GitBackendError),
    #[error("Cache location {location} does not exist")]
    BadLocation { location: String },
    #[error("Lock cannot be acquired: {0}")]
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
        let coord_locks = CoordinateLocks::new(location.join(CACHE_VERSION).join("locks"));

        Ok(ProtofetchGitCache {
            unversioned_location: location,
            default_protocol,
            coord_locks,
            backend,
            lock,
        })
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        self.lock.upgrade()?;
        if self.unversioned_location.exists() {
            info!(
                "Clearing protofetch repository cache {}",
                &self.unversioned_location.display()
            );
            std::fs::remove_dir_all(&self.unversioned_location)?;
        }
        Ok(())
    }

    /// Opens the cached repo for `entry`, creating it if needed.
    ///
    /// The returned handle holds the repo's cross-process lock until dropped, so
    /// don't open the same coordinate twice on one thread: the second call waits forever.
    pub fn repository(&self, entry: &Coordinate) -> Result<ProtoGitRepository, CacheError> {
        let lock = self.coord_locks.lock(entry)?;
        let mut path = self.repositories_path();
        path.push(entry.to_path());

        let url = entry.to_git_url(self.default_protocol);

        let repo = if path.exists() {
            self.open_entry(&path, &url)?
        } else {
            self.create_repo(&path, &url)?
        };

        let worktrees = self.worktrees_path();
        Ok(ProtoGitRepository::new(repo, url, &worktrees, lock))
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

    /// Shared, so concurrent runs can use the cache; `clear` upgrades it to exclusive.
    fn acquire_lock(root: &Path) -> Result<FileLock, CacheError> {
        let location = root.join(".lock");
        debug!(
            "Acquiring a lock on the cache location: {}",
            location.display()
        );
        let lock = FileLock::shared(&location)?;
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

#[cfg(test)]
mod tests {
    use std::fs::File;

    use fs4::fs_std::FileExt;

    use crate::{
        git::backend::GitBackendType,
        model::protofetch::{Coordinate, Protocol},
    };

    use super::ProtofetchGitCache;

    fn cache(dir: &tempfile::TempDir) -> ProtofetchGitCache {
        ProtofetchGitCache::new(
            dir.path().to_path_buf(),
            Protocol::Https,
            GitBackendType::default(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn open_cache_admits_other_runs_but_not_clear() {
        let dir = tempfile::tempdir().unwrap();
        let _cache = cache(&dir);

        let other = File::open(dir.path().join(".lock")).unwrap();
        assert!(!FileExt::try_lock_exclusive(&other).unwrap());
        assert!(FileExt::try_lock_shared(&other).unwrap());
    }

    #[test]
    fn repository_holds_its_lock() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(&dir);
        let coordinate = Coordinate::from_url("example.com/org/repo").unwrap();

        let _repository = cache.repository(&coordinate).unwrap();
        let other = File::create(dir.path().join("v2/locks/example.com/org/repo.lock")).unwrap();
        assert!(!FileExt::try_lock_exclusive(&other).unwrap());
    }
}
