use std::path::PathBuf;

use crate::{
    flock::{Error, FileLock},
    model::protofetch::Coordinate,
};

/// Cross-process locks, one per `forge/org/repo` in the cache.
///
/// Fetching into a cached bare repo or creating/resetting one of its worktrees
/// holds that repo's lock (reading worktree files afterwards does not), so
/// protofetch runs sharing a cache (for example parallel Cargo build scripts)
/// only wait for each other when they need the same repository.
///
/// The key intentionally ignores `protocol`: two coordinates that differ
/// only in `https` vs `ssh` resolve to the same on-disk bare repo, and so
/// must share a lock.
pub struct CoordinateLocks {
    root: PathBuf,
}

impl CoordinateLocks {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn lock(&self, coordinate: &Coordinate) -> Result<FileLock, Error> {
        let path = self.path(coordinate);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        FileLock::exclusive(&path)
    }

    fn path(&self, coordinate: &Coordinate) -> PathBuf {
        let mut path = self.root.join(coordinate.to_path()).into_os_string();
        path.push(".lock");
        path.into()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, path::PathBuf};

    use fs4::fs_std::FileExt;

    use crate::model::protofetch::{Coordinate, Protocol};

    use super::CoordinateLocks;

    fn coord(name: &str) -> Coordinate {
        Coordinate::from_url(&format!("example.com/org/{}", name)).unwrap()
    }

    fn can_lock(locks: &CoordinateLocks, coordinate: &Coordinate) -> bool {
        let file = File::create(locks.path(coordinate)).unwrap();
        FileExt::try_lock_exclusive(&file).unwrap()
    }

    #[test]
    fn lock_excludes_other_handles_to_the_same_repository() {
        let dir = tempfile::tempdir().unwrap();
        let locks = CoordinateLocks::new(dir.path().to_path_buf());

        let held = locks.lock(&coord("foo")).unwrap();
        assert!(!can_lock(&locks, &coord("foo")));
        drop(held);
        // Blocking: a `git` child forked by a concurrent test may share the
        // lock's file descriptor until it execs, so a try-lock here is racy.
        locks.lock(&coord("foo")).unwrap();
    }

    #[test]
    fn different_repositories_lock_independently() {
        let dir = tempfile::tempdir().unwrap();
        let locks = CoordinateLocks::new(dir.path().to_path_buf());

        let _held = locks.lock(&coord("foo")).unwrap();
        assert!(can_lock(&locks, &coord("bar")));
    }

    #[test]
    fn same_path_different_protocol_shares_a_lock() {
        let mut https = Coordinate::from_url("github.com/org/repo").unwrap();
        https.protocol = Some(Protocol::Https);
        let mut ssh = Coordinate::from_url("github.com/org/repo").unwrap();
        ssh.protocol = Some(Protocol::Ssh);

        let locks = CoordinateLocks::new(PathBuf::from("locks"));
        assert_eq!(locks.path(&https), locks.path(&ssh));
    }
}
