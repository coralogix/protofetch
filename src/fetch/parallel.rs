use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    thread,
};

use log::{info, warn};

use crate::{
    fetch::FetchError,
    git::coord_locks::CoordinateLocks,
    model::protofetch::{
        lock::{LockFile, LockedCoordinate, LockedDependency},
        resolved::{ResolvedDependency, ResolvedModule},
        Dependency, Descriptor, ModuleName, RevisionSpecification,
    },
    resolver::{CommitAndDescriptor, ModuleResolver},
    sync::Semaphore,
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

/// Run dependency resolution in parallel, level-by-level BFS over the dep
/// graph. Within one level, sibling deps are resolved concurrently under
/// the network semaphore; between levels we wait for all in-flight tasks
/// before scheduling the next level so the per-name dedup is deterministic
/// and matches the sequential resolver's "first wins + warn" semantics:
/// at each level deps are considered in declaration order (parent's order,
/// then each parent's children in declaration order).
///
/// `network_jobs` caps the number of concurrent resolver invocations.
/// `coord_locks` serializes calls hitting the same on-disk bare repo.
pub fn parallel_resolve<R>(
    descriptor: &Descriptor,
    resolver: Arc<R>,
    coord_locks: CoordinateLocks,
    network_jobs: usize,
) -> Result<(ResolvedModule, LockFile), FetchError>
where
    R: ModuleResolver + ?Sized + 'static,
{
    let net_sem = Semaphore::new(network_jobs.max(1));

    // Per-name dedup: tracks the (coord, spec) we first saw for a name so we
    // can match the sequential implementation's "first wins + warn" semantics.
    let mut seen: HashMap<ModuleName, (LockedCoordinate, RevisionSpecification)> = HashMap::new();
    let mut results: BTreeMap<ModuleName, (LockedDependency, ResolvedDependency)> = BTreeMap::new();

    fn consider_dependency(
        dep: Dependency,
        seen: &mut HashMap<ModuleName, (LockedCoordinate, RevisionSpecification)>,
    ) -> Option<Dependency> {
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
    }

    let mut level: Vec<Dependency> = descriptor.dependencies.clone();

    while !level.is_empty() {
        // Dedup the upcoming level in declaration order before scheduling so
        // the lockfile output is deterministic regardless of completion order.
        let mut to_schedule: Vec<(usize, Dependency)> = Vec::new();
        for dep in level.drain(..) {
            if let Some(dep) = consider_dependency(dep, &mut seen) {
                to_schedule.push((to_schedule.len(), dep));
            }
        }

        // Run this level's tasks concurrently, but wait for all of them
        // before moving on. `thread::scope` joins every spawned thread when
        // the closure returns, so no Arc gymnastics are needed.
        let completed: Vec<(usize, Dependency, CommitAndDescriptor)> = thread::scope(|s| {
            let mut handles = Vec::with_capacity(to_schedule.len());
            for (idx, dep) in to_schedule {
                let net_sem = &net_sem;
                let coord_lock = coord_locks.lock_for(&dep.coordinate);
                let resolver = resolver.clone();
                handles.push(s.spawn(
                    move || -> Result<(usize, Dependency, CommitAndDescriptor), FetchError> {
                        let _permit = net_sem.acquire();
                        let _g = coord_lock.lock().expect("coord lock poisoned");
                        info!("Resolving {}", dep.coordinate);
                        let result = resolver
                            .resolve(&dep.coordinate, &dep.specification, None, &dep.name)
                            .map_err(FetchError::Resolver)?;
                        Ok((idx, dep, result))
                    },
                ));
            }
            let mut out = Vec::with_capacity(handles.len());
            for h in handles {
                out.push(h.join().map_err(|_| {
                    FetchError::Resolver(anyhow::anyhow!("worker thread panicked"))
                })??);
            }
            Ok::<_, FetchError>(out)
        })?;

        // Sort by schedule index so the next level's children appear in
        // declaration order (parent1's children first, parent2's next, etc.).
        let mut completed = completed;
        completed.sort_by_key(|(i, _, _)| *i);

        for (_, dep, cd) in completed {
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

            level.extend(dep_descriptor.dependencies);
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
        let mut entries: BTreeMap<
            Coordinate,
            BTreeMap<RevisionSpecification, CommitAndDescriptor>,
        > = BTreeMap::new();
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

    #[test]
    fn matches_sequential_diamond_graph() {
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
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 4).unwrap();

        assert_eq!(parallel, sequential);
        assert_eq!(parallel.dependencies.len(), 2);
        assert!(parallel
            .dependencies
            .contains(&locked("bar", "2.0.0", "c2")));
        assert!(parallel
            .dependencies
            .contains(&locked("foo", "1.0.0", "c1")));
    }

    #[test]
    fn first_wins_when_same_name_resolves_to_different_coords() {
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
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 4).unwrap();

        assert!(parallel
            .dependencies
            .contains(&locked("bar", "1.0.0", "c3")));
        assert!(parallel
            .dependencies
            .contains(&locked("foo", "1.0.0", "c1")));
    }

    #[test]
    fn transitive_conflicts_dedupe_in_declaration_order() {
        // Regression test: two parents at level 0 each pull a child named
        // "shared" but at different coords. Whichever parent appears first in
        // the descriptor must win, regardless of which task completes first.
        // (Without level-by-level scheduling, the late-completer's child
        // could register first in `seen`, producing a non-deterministic
        // lockfile and breaking `--locked` mode against transitive deps.)
        let entries = [
            (
                "fast_parent",
                "1.0.0",
                "p_fast",
                vec![dep("shared", "from_fast")],
            ),
            (
                "slow_parent",
                "1.0.0",
                "p_slow",
                vec![dep("shared", "from_slow")],
            ),
            ("shared", "from_fast", "h_fast", Vec::new()),
            ("shared", "from_slow", "h_slow", Vec::new()),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            // slow_parent declared FIRST — its transitive child must win.
            dependencies: vec![dep("slow_parent", "1.0.0"), dep("fast_parent", "1.0.0")],
        };

        // Run many times; deterministic dedup must always pick slow_parent's
        // child even though scheduling order can flap under load.
        for _ in 0..30 {
            let resolver = Arc::new(build_resolver_with(&entries));
            let (_, lf) =
                parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 4).unwrap();
            let shared = lf
                .dependencies
                .iter()
                .find(|d| d.name == ModuleName::from("shared"))
                .expect("shared in lockfile");
            assert_eq!(shared.commit_hash, "h_slow");
        }
    }

    #[test]
    fn coordinate_lock_serializes_same_repo() {
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
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 8).unwrap();
        assert_eq!(lf.dependencies.len(), 3);
        assert_eq!(in_flight.load(Ordering::SeqCst), 0);
        assert_eq!(
            max_in_flight.load(Ordering::SeqCst),
            1,
            "per-coord lock should serialize same-coord resolves"
        );
    }
}
