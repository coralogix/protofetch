use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::model::protofetch::Coordinate;

/// A map of per-coordinate locks. libgit2 is per-`Repository` thread-safe, but
/// concurrent fetches or worktree-add calls into the same on-disk bare repo can
/// race on ref updates. Acquiring the same `Mutex` per coordinate serializes
/// operations on one repo while still allowing different repos to run in
/// parallel.
///
/// The outer `Mutex` is only held long enough to look up or insert the inner
/// `Arc<Mutex<()>>` (microseconds), so it is not contended in practice.
#[derive(Default, Clone)]
pub struct CoordinateLocks {
    inner: Arc<Mutex<HashMap<Coordinate, Arc<Mutex<()>>>>>,
}

impl CoordinateLocks {
    pub fn lock_for(&self, coord: &Coordinate) -> Arc<Mutex<()>> {
        self.inner
            .lock()
            .expect("coord lock map poisoned")
            .entry(coord.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use crate::model::protofetch::Coordinate;

    use super::CoordinateLocks;

    fn coord(name: &str) -> Coordinate {
        Coordinate::from_url(&format!("example.com/org/{}", name)).unwrap()
    }

    #[test]
    fn same_coord_returns_same_lock() {
        let locks = CoordinateLocks::default();
        let a = locks.lock_for(&coord("foo"));
        let b = locks.lock_for(&coord("foo"));
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn different_coords_return_different_locks() {
        let locks = CoordinateLocks::default();
        let a = locks.lock_for(&coord("foo"));
        let b = locks.lock_for(&coord("bar"));
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn concurrent_lock_for_does_not_panic() {
        let locks = CoordinateLocks::default();
        let mut handles = Vec::new();
        for i in 0..32 {
            let locks = locks.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..50 {
                    let _l = locks.lock_for(&coord(&format!("dep{}", i % 4)));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }
}
