use log::info;
use rayon::prelude::*;

use crate::{
    cache::RepositoryCache, engine::model::ResolvedRootModule, engine::FetchError,
    git::coord_locks::CoordinateLocks,
};

/// Fans dependencies out across `network_jobs` rayon workers, serialized
/// per-coordinate so two fetches into the same on-disk bare repo don't race.
pub fn fetch<C>(
    cache: C,
    resolved: &ResolvedRootModule,
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
        resolved.modules.par_iter().try_for_each(|module| {
            let _lock = coord_locks.lock(&module.coordinate)?;
            cache.fetch(
                &module.coordinate,
                &module.specification,
                &module.commit_hash,
            )
        })
    })
    .map_err(FetchError::Cache)
}
