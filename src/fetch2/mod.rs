use std::collections::BTreeMap;

use log::warn;

use crate::{
    fetch::FetchError,
    fetch2::model::{ResolvedDependency, ResolvedModule, ResolvedRootModule},
    model::protofetch::{
        lock::{LockFile, LockedCoordinate, LockedDependency},
        Dependency, Descriptor, ModuleName, RevisionSpecification,
    },
    resolver::ModuleResolver,
};

mod model;

pub fn resolve<R>(
    descriptor: &Descriptor,
    resolver: R,
    _network_jobs: usize,
) -> Result<(ResolvedRootModule, LockFile), FetchError>
where
    R: ModuleResolver + Clone + 'static,
{
    fn resolved_dependencies(dependencies: &[Dependency]) -> Vec<ResolvedDependency> {
        // Resolved dependency edges keep only the edge metadata. The target
        // module's coordinate/spec/commit live in `ResolvedModule`.
        dependencies
            .iter()
            .map(|dependency| ResolvedDependency {
                name: dependency.name.clone(),
                rules: dependency.rules.clone(),
            })
            .collect()
    }

    fn resolve_dependencies<R>(
        dependencies: Vec<Dependency>,
        resolver: &R,
        seen: &mut BTreeMap<ModuleName, (LockedCoordinate, RevisionSpecification)>,
        modules: &mut Vec<ResolvedModule>,
        locked: &mut Vec<LockedDependency>,
    ) -> Result<(), FetchError>
    where
        R: ModuleResolver,
    {
        // Reserve all direct children before resolving any of them. This gives
        // the current module's declarations precedence over transitive deps,
        // while the second phase below still recurses depth-first.
        let mut to_resolve = Vec::new();
        for dependency in dependencies {
            let locked_coordinate = LockedCoordinate::from(&dependency.coordinate);
            match seen.get(&dependency.name) {
                None => {
                    seen.insert(
                        dependency.name.clone(),
                        (locked_coordinate, dependency.specification.clone()),
                    );
                    to_resolve.push(dependency);
                }
                Some((existing_coordinate, existing_specification)) => {
                    if existing_coordinate != &locked_coordinate {
                        warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.coordinate, existing_coordinate, dependency.name
                        );
                    } else if existing_specification != &dependency.specification {
                        warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.specification, existing_specification, dependency.name
                        );
                    }
                }
            }
        }

        for dependency in to_resolve {
            // Identity comes from the dependency edge, not from the descriptor returned by the resolver.
            let result = resolver
                .resolve(
                    &dependency.coordinate,
                    &dependency.specification,
                    None,
                    &dependency.name,
                )
                .map_err(FetchError::Resolver)?;

            modules.push(ResolvedModule {
                name: dependency.name.clone(),
                commit_hash: result.commit_hash.clone(),
                coordinate: dependency.coordinate.clone(),
                specification: dependency.specification.clone(),
                dependencies: resolved_dependencies(&result.descriptor.dependencies),
            });
            locked.push(LockedDependency {
                name: dependency.name.clone(),
                coordinate: LockedCoordinate::from(&dependency.coordinate),
                specification: dependency.specification.clone(),
                commit_hash: result.commit_hash,
            });

            // After sibling names have been reserved, walk each dependency's
            // children immediately, preserving declaration-order DFS behavior.
            resolve_dependencies(
                result.descriptor.dependencies,
                resolver,
                seen,
                modules,
                locked,
            )?;
        }

        Ok(())
    }

    let mut seen = BTreeMap::new();
    let mut modules = Vec::new();
    let mut locked = Vec::new();

    resolve_dependencies(
        descriptor.dependencies.clone(),
        &resolver,
        &mut seen,
        &mut modules,
        &mut locked,
    )?;

    Ok((
        ResolvedRootModule {
            name: descriptor.name.clone(),
            modules,
            dependencies: resolved_dependencies(&descriptor.dependencies),
        },
        LockFile {
            dependencies: locked,
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };

    use anyhow::anyhow;

    use crate::{
        fetch2::{
            model::{ResolvedDependency, ResolvedModule},
            resolve,
        },
        model::protofetch::{
            lock::{LockedCoordinate, LockedDependency},
            AllowPolicies, Coordinate, Dependency, Descriptor, ModuleName, Revision,
            RevisionSpecification, Rules,
        },
        resolver::{CommitAndDescriptor, ModuleResolver},
    };

    struct FakeResolver {
        entries: BTreeMap<Coordinate, BTreeMap<RevisionSpecification, CommitAndDescriptor>>,
    }

    impl ModuleResolver for FakeResolver {
        fn resolve(
            &self,
            coordinate: &Coordinate,
            specification: &RevisionSpecification,
            _: Option<&str>,
            _: &ModuleName,
        ) -> anyhow::Result<CommitAndDescriptor> {
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

    fn resolved_dep(name: &str, rules: Rules) -> ResolvedDependency {
        ResolvedDependency {
            name: ModuleName::from(name),
            rules,
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
        FakeResolver { entries }
    }

    fn module<'a>(modules: &'a [ResolvedModule], name: &str) -> &'a ResolvedModule {
        modules
            .iter()
            .find(|module| module.name == ModuleName::from(name))
            .expect("module must be resolved")
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
        let resolver = Arc::new(build_resolver_with(&entries));
        let (resolved, lockfile) = resolve(&descriptor, resolver, 4).unwrap();

        assert_eq!(resolved.name, ModuleName::from("root"));
        assert_eq!(
            resolved.dependencies,
            vec![resolved_dep("foo", Rules::default())]
        );
        assert_eq!(resolved.modules.len(), 2);
        assert_eq!(module(&resolved.modules, "foo").commit_hash, "c1");
        assert_eq!(module(&resolved.modules, "bar").commit_hash, "c2");
        assert_eq!(
            module(&resolved.modules, "foo").dependencies,
            vec![resolved_dep("bar", Rules::default())]
        );
        assert!(module(&resolved.modules, "bar").dependencies.is_empty());
        assert_eq!(lockfile.dependencies.len(), 2);
        assert!(lockfile
            .dependencies
            .contains(&locked("bar", "2.0.0", "c2")));
        assert!(lockfile
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
        let (_, lockfile) = resolve(&descriptor, resolver, 4).unwrap();

        assert!(lockfile
            .dependencies
            .contains(&locked("bar", "1.0.0", "c3")));
        assert!(lockfile
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
        let (_, lockfile) = resolve(&descriptor, resolver, 4).unwrap();

        assert!(lockfile
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
        let (_, lockfile) = resolve(&descriptor, resolver, 4).unwrap();

        assert!(lockfile
            .dependencies
            .contains(&locked("bar", "1.0.0", "c3")));
        assert!(lockfile
            .dependencies
            .contains(&locked("foo", "1.0.0", "c1")));
    }

    fn with_policies(dep: Dependency, allow: &str) -> Dependency {
        Dependency {
            rules: Rules {
                allow_policies: AllowPolicies::new(BTreeSet::from([allow.parse().unwrap()])),
                ..Default::default()
            },
            ..dep
        }
    }

    fn rules(allow: &str) -> Rules {
        Rules {
            allow_policies: AllowPolicies::new(BTreeSet::from([allow.parse().unwrap()])),
            ..Default::default()
        }
    }

    #[test]
    fn duplicate_dependencies_keep_edge_specific_policies() {
        let entries = [
            (
                "foo",
                "1.0.0",
                "c_foo",
                vec![with_policies(dep("shared", "1.0.0"), "/b.proto")],
            ),
            ("shared", "1.0.0", "c_shared", vec![]),
        ];
        let descriptor = Descriptor {
            name: ModuleName::from("root"),
            description: None,
            proto_out_dir: None,
            dependencies: vec![
                with_policies(dep("shared", "1.0.0"), "/a.proto"),
                dep("foo", "1.0.0"),
            ],
        };
        let resolver = Arc::new(build_resolver_with(&entries));
        let (resolved, _) = resolve(&descriptor, resolver, 4).unwrap();

        assert_eq!(
            resolved.dependencies,
            vec![
                resolved_dep("shared", rules("/a.proto")),
                resolved_dep("foo", Rules::default()),
            ]
        );
        assert_eq!(
            module(&resolved.modules, "foo").dependencies,
            vec![resolved_dep("shared", rules("/b.proto"))]
        );
    }
}
