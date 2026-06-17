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

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use anyhow::anyhow;

    use crate::{
        fetch::{parallel::parallel_resolve, tests::resolve},
        git::coord_locks::CoordinateLocks,
        model::protofetch::{
            lock::{LockedCoordinate, LockedDependency},
            AllowPolicies, Coordinate, DenyPolicies, Dependency, Descriptor, ModuleName, Revision,
            RevisionSpecification, Rules,
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
    fn parent_override_wins() {
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
    fn first_wins_even_if_different_level() {
        let entries = [
            ("leaf", "1.0.0", "leaf1", Vec::new()),
            ("leaf", "2.0.0", "leaf2", Vec::new()),
            ("path_a_1", "1.0.0", "c1", vec![dep("path_a_2", "1.0.0")]),
            ("path_a_2", "1.0.0", "c2", vec![dep("leaf", "1.0.0")]),
            ("path_b_1", "1.0.0", "c3", vec![dep("leaf", "2.0.0")]),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            dependencies: vec![dep("path_a_1", "1.0.0"), dep("path_b_1", "1.0.0")],
        };
        let resolver = Arc::new(build_resolver_with(&entries));
        let (_, parallel) =
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 4).unwrap();

        assert!(parallel
            .dependencies
            .contains(&locked("leaf", "1.0.0", "leaf1")));
    }

    #[test]
    fn circular_dependency() {
        let entries = [
            ("foo", "1.0.0", "c1", vec![dep("bar", "1.0.0")]),
            ("bar", "1.0.0", "c3", vec![dep("foo", "2.0.0")]),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            dependencies: vec![dep("foo", "1.0.0")],
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

    fn with_policies(dep: Dependency, allow: &str, deny: &str) -> Dependency {
        Dependency {
            rules: Rules {
                allow_policies: AllowPolicies::new(BTreeSet::from([allow.parse().unwrap()])),
                deny_policies: DenyPolicies::new(BTreeSet::from([deny.parse().unwrap()])),
                ..Default::default()
            },
            ..dep
        }
    }

    #[test]
    fn policies_are_merged_across_duplicate_dependencies() {
        // Dependency graph:
        //   root -> shared@1.0.0 (allow=[/a.proto], deny=[/x.proto])  <- root declares it
        //   root -> foo@1.0.0
        //   foo  -> shared@1.0.0 (allow=[/b.proto], deny=[/y.proto])  <- foo declares it
        //
        // `shared` appears twice at the same coordinate+revision but with different
        // policies. The resolver should merge them into the union of both sets.
        let entries = [
            (
                "foo",
                "1.0.0",
                "c_foo",
                vec![with_policies(
                    dep("shared", "1.0.0"),
                    "/b.proto",
                    "/y.proto",
                )],
            ),
            ("shared", "1.0.0", "c_shared", vec![]),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            // root declares shared first (with /a.proto / /x.proto), then foo
            // transitively re-introduces shared with /b.proto / /y.proto.
            dependencies: vec![
                with_policies(dep("shared", "1.0.0"), "/a.proto", "/x.proto"),
                dep("foo", "1.0.0"),
            ],
        };
        let resolver = Arc::new(build_resolver_with(&entries));
        let (resolved, _) =
            parallel_resolve(&descriptor, resolver, CoordinateLocks::default(), 4).unwrap();

        let shared = resolved
            .dependencies
            .iter()
            .find(|d| d.name == ModuleName::from("shared"))
            .expect("shared must be in resolved deps");

        // rules must contain both (allow, deny) pairs in declaration order:
        // first from root's direct declaration, then from foo's transitive one.
        assert_eq!(
            shared.rules,
            vec![
                Rules {
                    allow_policies: AllowPolicies::new(BTreeSet::from(["/a.proto"
                        .parse()
                        .unwrap()])),
                    deny_policies: DenyPolicies::new(BTreeSet::from(["/x.proto".parse().unwrap()])),
                    ..Default::default()
                },
                Rules {
                    allow_policies: AllowPolicies::new(BTreeSet::from(["/b.proto"
                        .parse()
                        .unwrap()])),
                    deny_policies: DenyPolicies::new(BTreeSet::from(["/y.proto".parse().unwrap()])),
                    ..Default::default()
                },
            ],
            "rules should contain full Rules from all occurrences"
        );
    }
}
