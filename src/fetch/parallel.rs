use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use log::{info, warn};
use tokio::{sync::Semaphore, task::JoinSet};

use crate::{
    fetch::FetchError,
    git::coord_locks::CoordinateLocks,
    model::protofetch::{
        lock::{LockFile, LockedCoordinate, LockedDependency},
        resolved::{ResolvedDependency, ResolvedModule},
        Dependency, Descriptor, ModuleName, RevisionSpecification,
    },
    resolver::{CommitAndDescriptor, ModuleResolver},
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

/// Run dependency resolution in parallel, fanning out across the blocking
/// pool. Behavior matches the sequential resolver in [`crate::fetch::resolve`]:
/// the same `ModuleName` resolved twice keeps the first occurrence and a
/// warning is logged for any conflicting coordinate or revision specification.
///
/// `network_jobs` caps the number of concurrent resolver invocations.
/// `coord_locks` serializes calls hitting the same on-disk bare repo.
pub async fn parallel_resolve<R>(
    descriptor: &Descriptor,
    resolver: Arc<R>,
    coord_locks: CoordinateLocks,
    network_jobs: usize,
) -> Result<(ResolvedModule, LockFile), FetchError>
where
    R: ModuleResolver + ?Sized + 'static,
{
    let net_sem = Arc::new(Semaphore::new(network_jobs.max(1)));
    let mut set = JoinSet::new();

    // Per-name dedup: tracks the (coord, spec) we first saw for a name so we
    // can match the sequential implementation's "first wins + warn" semantics.
    let mut seen: HashMap<ModuleName, (LockedCoordinate, RevisionSpecification)> = HashMap::new();
    let mut results: BTreeMap<ModuleName, (LockedDependency, ResolvedDependency)> = BTreeMap::new();

    let mut to_schedule: Vec<Dependency> = descriptor.dependencies.clone();
    let consider_dependency =
        |dep: Dependency,
         seen: &mut HashMap<ModuleName, (LockedCoordinate, RevisionSpecification)>|
         -> Option<Dependency> {
            let locked_coord = LockedCoordinate::from(&dep.coordinate);
            match seen.get(&dep.name) {
                None => {
                    seen.insert(dep.name.clone(), (locked_coord, dep.specification.clone()));
                    Some(dep)
                }
                Some((existing_coord, existing_spec)) => {
                    if existing_coord != &locked_coord {
                        warn!(
                            "discarded {} in favor of {} for {}",
                            dep.coordinate, existing_coord, &dep.name
                        );
                    } else if existing_spec != &dep.specification {
                        warn!(
                            "discarded {} in favor of {} for {}",
                            dep.specification, existing_spec, &dep.name
                        );
                    }
                    None
                }
            }
        };

    fn spawn_resolve<R>(
        set: &mut JoinSet<Result<(Dependency, CommitAndDescriptor), FetchError>>,
        net_sem: &Arc<Semaphore>,
        coord_locks: &CoordinateLocks,
        resolver: &Arc<R>,
        dep: Dependency,
    ) where
        R: ModuleResolver + ?Sized + 'static,
    {
        let net_sem = net_sem.clone();
        let coord_lock = coord_locks.lock_for(&dep.coordinate);
        let resolver = resolver.clone();
        set.spawn(async move {
            let permit = net_sem
                .acquire_owned()
                .await
                .expect("network semaphore closed");
            let dep_for_blocking = dep.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let _g = coord_lock.lock().expect("coord lock poisoned");
                info!("Resolving {}", dep_for_blocking.coordinate);
                resolver.resolve(
                    &dep_for_blocking.coordinate,
                    &dep_for_blocking.specification,
                    None,
                    &dep_for_blocking.name,
                )
            })
            .await
            .map_err(|e| FetchError::Resolver(anyhow::anyhow!(e)))?
            .map_err(FetchError::Resolver)?;
            Ok((dep, result))
        });
    }

    for dep in to_schedule.drain(..) {
        if let Some(dep) = consider_dependency(dep, &mut seen) {
            spawn_resolve(&mut set, &net_sem, &coord_locks, &resolver, dep);
        }
    }

    while let Some(joined) = set.join_next().await {
        let (dep, cd) = joined.map_err(|e| FetchError::Resolver(anyhow::anyhow!(e)))??;
        let CommitAndDescriptor {
            commit_hash,
            descriptor: dep_descriptor,
        } = cd;

        let locked = LockedDependency {
            name: dep.name.clone(),
            commit_hash: commit_hash.clone(),
            coordinate: LockedCoordinate::from(&dep.coordinate),
            specification: dep.specification.clone(),
        };
        let resolved = ResolvedDependency {
            name: dep.name.clone(),
            commit_hash,
            coordinate: dep.coordinate.clone(),
            specification: dep.specification.clone(),
            rules: dep.rules.clone(),
            dependencies: dep_descriptor
                .dependencies
                .iter()
                .map(|d| d.name.clone())
                .collect(),
        };
        results.insert(dep.name.clone(), (locked, resolved));

        for child in dep_descriptor.dependencies {
            if let Some(child) = consider_dependency(child, &mut seen) {
                spawn_resolve(&mut set, &net_sem, &coord_locks, &resolver, child);
            }
        }
    }

    let (locked, resolved): (Vec<_>, Vec<_>) = results.into_values().unzip();
    let resolved = ResolvedModule {
        module_name: descriptor.name.clone(),
        dependencies: resolved,
    };
    let lockfile = LockFile {
        dependencies: locked,
    };
    Ok((resolved, lockfile))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use anyhow::anyhow;

    use crate::{
        fetch::{parallel::parallel_resolve, resolve},
        git::coord_locks::CoordinateLocks,
        model::protofetch::{
            lock::{LockedCoordinate, LockedDependency},
            Coordinate, Dependency, Descriptor, ModuleName, Revision, RevisionSpecification, Rules,
        },
        resolver::{CommitAndDescriptor, ModuleResolver},
    };

    struct FakeResolver {
        entries: BTreeMap<Coordinate, BTreeMap<RevisionSpecification, CommitAndDescriptor>>,
        delay_ms: u64,
        in_flight: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
    }

    impl ModuleResolver for FakeResolver {
        fn resolve(
            &self,
            coordinate: &Coordinate,
            specification: &RevisionSpecification,
            _: Option<&str>,
            _: &ModuleName,
        ) -> anyhow::Result<CommitAndDescriptor> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(self.delay_ms));
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(self
                .entries
                .get(coordinate)
                .ok_or_else(|| anyhow!("Coordinate not found: {}", coordinate))?
                .get(specification)
                .ok_or_else(|| anyhow!("Specification not found: {}", specification))?
                .clone())
        }
    }

    fn coord(name: &str) -> Coordinate {
        Coordinate::from_url(&format!("example.com/org/{}", name)).unwrap()
    }

    fn dep(name: &str, revision: &str) -> Dependency {
        Dependency {
            name: ModuleName::from(name),
            coordinate: coord(name),
            specification: RevisionSpecification {
                revision: Revision::pinned(revision),
                branch: None,
            },
            rules: Rules::default(),
        }
    }

    fn locked(name: &str, revision: &str, hash: &str) -> LockedDependency {
        LockedDependency {
            name: ModuleName::from(name),
            coordinate: LockedCoordinate {
                url: format!("example.com/org/{}", name),
                protocol: None,
            },
            specification: RevisionSpecification {
                revision: Revision::pinned(revision),
                branch: None,
            },
            commit_hash: hash.to_owned(),
        }
    }

    fn build_resolver_with(deps: &[(&str, &str, &str, Vec<Dependency>)]) -> FakeResolver {
        let mut entries: BTreeMap<Coordinate, BTreeMap<RevisionSpecification, CommitAndDescriptor>> =
            BTreeMap::new();
        for (name, rev, hash, child_deps) in deps {
            entries.entry(coord(name)).or_default().insert(
                RevisionSpecification {
                    revision: Revision::pinned(*rev),
                    branch: None,
                },
                CommitAndDescriptor {
                    commit_hash: hash.to_string(),
                    descriptor: Descriptor {
                        name: ModuleName::from(*name),
                        description: None,
                        proto_out_dir: None,
                        dependencies: child_deps.clone(),
                    },
                },
            );
        }
        FakeResolver {
            entries,
            delay_ms: 0,
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn matches_sequential_diamond_graph() {
        let entries = [
            ("foo", "1.0.0", "c1", vec![dep("bar", "2.0.0")]),
            ("bar", "2.0.0", "c2", Vec::new()),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            dependencies: vec![dep("foo", "1.0.0")],
        };
        let resolver = build_resolver_with(&entries);
        let (_, sequential) = resolve(&descriptor, &resolver).unwrap();

        let resolver = Arc::new(build_resolver_with(&entries));
        let (_, parallel) =
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 4)
                .await
                .unwrap();

        assert_eq!(parallel, sequential);
        assert_eq!(parallel.dependencies.len(), 2);
        assert!(parallel.dependencies.contains(&locked("bar", "2.0.0", "c2")));
        assert!(parallel.dependencies.contains(&locked("foo", "1.0.0", "c1")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn first_wins_when_same_name_resolves_to_different_coords() {
        let entries = [
            ("foo", "1.0.0", "c1", vec![dep("bar", "2.0.0")]),
            ("bar", "1.0.0", "c3", Vec::new()),
            ("bar", "2.0.0", "c2", Vec::new()),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            dependencies: vec![dep("foo", "1.0.0"), dep("bar", "1.0.0")],
        };
        let resolver = Arc::new(build_resolver_with(&entries));
        let (_, parallel) =
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 4)
                .await
                .unwrap();

        assert!(parallel.dependencies.contains(&locked("bar", "1.0.0", "c3")));
        assert!(parallel.dependencies.contains(&locked("foo", "1.0.0", "c1")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn coordinate_lock_serializes_same_repo() {
        // Three deps to the same coordinate. With per-coord lock, only one
        // can be in flight at a time.
        let entries = [
            ("foo", "1.0.0", "c1", Vec::new()),
            ("foo", "2.0.0", "c2", Vec::new()),
            ("foo", "3.0.0", "c3", Vec::new()),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            dependencies: vec![
                Dependency {
                    name: ModuleName::from("foo-a"),
                    coordinate: coord("foo"),
                    specification: RevisionSpecification {
                        revision: Revision::pinned("1.0.0"),
                        branch: None,
                    },
                    rules: Rules::default(),
                },
                Dependency {
                    name: ModuleName::from("foo-b"),
                    coordinate: coord("foo"),
                    specification: RevisionSpecification {
                        revision: Revision::pinned("2.0.0"),
                        branch: None,
                    },
                    rules: Rules::default(),
                },
                Dependency {
                    name: ModuleName::from("foo-c"),
                    coordinate: coord("foo"),
                    specification: RevisionSpecification {
                        revision: Revision::pinned("3.0.0"),
                        branch: None,
                    },
                    rules: Rules::default(),
                },
            ],
        };

        let mut resolver = build_resolver_with(&entries);
        resolver.delay_ms = 30;
        let in_flight = resolver.in_flight.clone();
        let max_in_flight = resolver.max_in_flight.clone();
        let resolver = Arc::new(resolver);

        let (_, lf) =
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 8)
                .await
                .unwrap();
        assert_eq!(lf.dependencies.len(), 3);
        assert_eq!(in_flight.load(Ordering::SeqCst), 0);
        assert_eq!(
            max_in_flight.load(Ordering::SeqCst),
            1,
            "per-coord lock should serialize same-coord resolves"
        );
    }
}
