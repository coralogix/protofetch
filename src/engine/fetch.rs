use log::info;
use rayon::prelude::*;

use crate::{
    cache::RepositoryCache, engine::FetchError, git::coord_locks::CoordinateLocks,
    model::protofetch::resolved::ResolvedDependency,
};

/// Fans dependencies out across `network_jobs` rayon workers, serialized
/// per-coordinate so two fetches into the same on-disk bare repo don't race.
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
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(network_jobs.max(1))
        .build()?;

    pool.install(|| {
        dependencies.into_par_iter().try_for_each(|dependency| {
            let cache = cache.clone();
            let coord_lock = coord_locks.lock_for(&dependency.coordinate);

            let _g = coord_lock.lock().expect("coord lock poisoned");
            cache.fetch(
                &dependency.coordinate,
                &dependency.specification,
                &dependency.commit_hash,
            )
        })
    })
    .map_err(FetchError::Cache)
}
