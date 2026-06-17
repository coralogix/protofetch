use crate::{
    fetch::FetchError,
    fetch2::resolve,
    git::coord_locks::CoordinateLocks,
    model::protofetch::{lock::LockFile, resolved::ResolvedModule, Descriptor},
    resolver::ModuleResolver,
};

/// Tunables for the parallel resolver / fetcher.
#[derive(Debug, Clone, Copy)]
pub struct ParallelConfig {
    /// Maximum number of in-flight network operations (resolve / fetch).
    pub network_jobs: usize,
    /// Maximum number of in-flight disk-bound operations (worktree + copy).
    pub copy_jobs: usize,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            network_jobs: 16,
            copy_jobs: (cpus / 2).max(4),
        }
    }
}

/// Run dependency resolution in parallel.
pub fn parallel_resolve<R>(
    descriptor: &Descriptor,
    resolver: R,
    coord_locks: CoordinateLocks,
    network_jobs: usize,
) -> Result<(ResolvedModule, LockFile), FetchError>
where
    R: ModuleResolver + Clone + 'static,
{
    let (resolved, mut lockfile) = resolve(descriptor, resolver, coord_locks, network_jobs)?;
    lockfile
        .dependencies
        .sort_by(|left, right| left.name.cmp(&right.name));
    Ok((ResolvedModule::from(resolved), lockfile))
}
