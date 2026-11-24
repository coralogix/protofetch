use std::collections::{BTreeMap, BTreeSet};

use log::warn;
use rayon::prelude::*;

use crate::{
    cache::RepositoryCache,
    model::protofetch::{lock::LockFile, Coordinate},
};

/// Fetch every commit the lock file pins, in one fetch per repository.
///
/// A locked fetch knows all the commits it needs upfront, but resolution
/// discovers them one descriptor at a time, taking a fetch session per commit
/// against repositories that supply several commits. Failures are only
/// warnings: resolution still fetches and verifies every commit it reaches,
/// with the dependency context this plan lacks.
pub fn prefetch_locked<C>(cache: &C, lock_file: &LockFile, network_jobs: usize)
where
    C: RepositoryCache,
{
    // A pool that cannot be built here cannot be built by engine::fetch
    // either, which reports the error.
    let Ok(pool) = rayon::ThreadPoolBuilder::new()
        .num_threads(network_jobs.max(1))
        .build()
    else {
        return;
    };

    pool.install(|| {
        group_by_repository(lock_file)
            .par_iter()
            .for_each(|(coordinate, commits)| {
                let commits = commits.iter().copied().collect::<Vec<_>>();
                if let Err(error) = cache.fetch_commits(coordinate, &commits) {
                    warn!(
                        "Failed to prefetch commits of {coordinate}, fetching them one at a time: {error:#}"
                    );
                }
            })
    });
}

/// Group the commits a lock file pins by repository.
///
/// Coordinates that do not parse are left out: resolution reports them with
/// the dependency context this grouping does not have.
fn group_by_repository(lock_file: &LockFile) -> BTreeMap<Coordinate, BTreeSet<&str>> {
    let mut by_repository: BTreeMap<Coordinate, BTreeSet<&str>> = BTreeMap::new();
    for dependency in &lock_file.dependencies {
        if let Ok(coordinate) = Coordinate::from_url_protocol(
            &dependency.coordinate.url,
            dependency.coordinate.protocol,
        ) {
            by_repository
                .entry(coordinate)
                .or_default()
                .insert(dependency.commit_hash.as_str());
        }
    }
    by_repository
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, path::PathBuf, sync::Mutex};

    use super::*;
    use crate::model::protofetch::{
        lock::{LockedCoordinate, LockedDependency},
        ModuleName, RevisionSpecification,
    };

    fn coordinate(url: &str) -> Coordinate {
        Coordinate::from_url_protocol(url, None).unwrap()
    }

    fn locked(url: &str, commit_hash: &str) -> LockedDependency {
        LockedDependency {
            name: ModuleName::from("dep"),
            coordinate: LockedCoordinate {
                url: url.to_owned(),
                protocol: None,
            },
            specification: RevisionSpecification::default(),
            commit_hash: commit_hash.to_owned(),
        }
    }

    fn lock_file() -> LockFile {
        LockFile {
            dependencies: vec![
                locked("example.com/org/shared", "c1"),
                locked("example.com/org/shared", "c2"),
                locked("example.com/org/shared", "c1"),
                locked("example.com/org/other", "c3"),
                locked("not-a-coordinate", "c4"),
            ],
        }
    }

    #[test]
    fn groups_commits_per_repository_deduplicated_and_parsable_only() {
        let lock_file = lock_file();
        let grouped = group_by_repository(&lock_file);

        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped[&coordinate("example.com/org/shared")],
            BTreeSet::from(["c1", "c2"])
        );
        assert_eq!(
            grouped[&coordinate("example.com/org/other")],
            BTreeSet::from(["c3"])
        );
    }

    /// Records every `fetch_commits` call and always fails.
    struct FailingCache(Mutex<Vec<String>>);

    impl RepositoryCache for FailingCache {
        fn fetch(&self, _: &Coordinate, _: &RevisionSpecification, _: &str) -> anyhow::Result<()> {
            unreachable!("prefetch must not fetch single commits")
        }

        fn fetch_commits(&self, coordinate: &Coordinate, commits: &[&str]) -> anyhow::Result<()> {
            let call = format!("{coordinate}:{}", commits.join(","));
            self.0.lock().unwrap().push(call);
            Err(anyhow::anyhow!("batch fetch failed"))
        }

        fn create_worktree(&self, _: &Coordinate, _: &str) -> anyhow::Result<PathBuf> {
            unreachable!("prefetch must not create worktrees")
        }
    }

    #[test]
    fn prefetches_one_batched_call_per_repository_and_survives_failures() {
        let cache = FailingCache(Mutex::new(Vec::new()));
        prefetch_locked(&cache, &lock_file(), 4);

        let mut calls = cache.0.lock().unwrap().clone();
        calls.sort();
        assert_eq!(
            calls,
            vec!["example.com/org/other:c3", "example.com/org/shared:c1,c2"]
        );
    }
}
