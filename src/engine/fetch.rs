use std::thread;

use log::info;

use crate::{
    cache::RepositoryCache, engine::FetchError, git::coord_locks::CoordinateLocks,
    model::protofetch::resolved::ResolvedDependency, sync::Semaphore,
};

/// Fans dependencies out across `network_jobs` worker threads, gated by
/// the network semaphore and serialized per-coordinate so two fetches into
/// the same on-disk bare repo don't race.
pub fn fetch<C>(
    cache: C,
    dependencies: Vec<ResolvedDependency>,
    coord_locks: CoordinateLocks,
    network_jobs: usize,
) -> Result<(), FetchError>
where
    C: RepositoryCache + Clone + 'static,
{
    info!("Fetching dependencies source files...");
    let net_sem = Semaphore::new(network_jobs.max(1));

    thread::scope(|s| -> Result<(), FetchError> {
        let mut handles = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let cache = cache.clone();
            let coord_lock = coord_locks.lock_for(&dependency.coordinate);
            let net_sem = &net_sem;
            handles.push(s.spawn(move || {
                let _permit = net_sem.acquire();
                let _g = coord_lock.lock().expect("coord lock poisoned");
                cache
                    .fetch(
                        &dependency.coordinate,
                        &dependency.specification,
                        &dependency.commit_hash,
                    )
                    .map_err(FetchError::Cache)
            }));
        }
        for h in handles {
            match h.join() {
                Ok(result) => result?,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
        Ok(())
    })
}
