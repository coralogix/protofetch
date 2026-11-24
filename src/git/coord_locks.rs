use std::path::PathBuf;

use crate::flock::{Error, FileLock};

/// Cross-process locks, one per host and repository path in the cache.
///
/// Fetching into a cached bare repo or creating/resetting one of its worktrees
/// holds that repo's lock (reading worktree files afterwards does not), so
/// protofetch runs sharing a cache (for example parallel Cargo build scripts)
/// only wait for each other when they need the same repository.
///
/// Callers use the same key for the repository and its lock.
pub struct CoordinateLocks {
    root: PathBuf,
}

impl CoordinateLocks {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn lock(&self, cache_key: &str) -> Result<FileLock, Error> {
        std::fs::create_dir_all(&self.root)?;
        FileLock::exclusive(&self.path(cache_key))
    }

    fn path(&self, cache_key: &str) -> PathBuf {
        self.root.join(format!("{cache_key}.lock"))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use fs4::fs_std::FileExt;

    use super::*;

    fn can_lock(locks: &CoordinateLocks, key: &str) -> bool {
        let file = File::create(locks.path(key)).unwrap();
        FileExt::try_lock_exclusive(&file).unwrap()
    }

    #[test]
    fn lock_excludes_other_handles_to_the_same_repository() {
        let dir = tempfile::tempdir().unwrap();
        let locks = CoordinateLocks::new(dir.path().to_path_buf());

        let held = locks.lock("foo").unwrap();
        assert!(!can_lock(&locks, "foo"));
        drop(held);
        // Blocking: a `git` child forked by a concurrent test may share the
        // lock's file descriptor until it execs, so a try-lock here is racy.
        locks.lock("foo").unwrap();
    }

    #[test]
    fn different_repositories_lock_independently() {
        let dir = tempfile::tempdir().unwrap();
        let locks = CoordinateLocks::new(dir.path().to_path_buf());

        let _held = locks.lock("foo").unwrap();
        assert!(can_lock(&locks, "bar"));
    }
}
