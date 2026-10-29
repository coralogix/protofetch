use log::info;
use rayon::prelude::*;

use crate::{cache::RepositoryCache, engine::model::ResolvedRootModule, engine::FetchError};

/// Fans dependencies out across `network_jobs` rayon workers.
pub fn fetch<C>(
    cache: C,
    resolved: &ResolvedRootModule,
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
            cache.fetch(
                &module.coordinate,
                &module.specification,
                &module.commit_hash,
            )
        })
    })
    .map_err(FetchError::Cache)
}
