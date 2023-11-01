use std::{collections::BTreeMap, str::Utf8Error};

use crate::{
    cache::RepositoryCache,
    model::protofetch::{
        lock::{LockFile, LockedDependency},
        resolved::{ResolvedDependency, ResolvedModule},
        Dependency, DependencyName, Descriptor,
    },
    resolver::{CommitAndDescriptor, ModuleResolver},
};
use log::{error, info};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache: {0}")]
    Cache(anyhow::Error),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob: {0}")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Error while processing protobuf repository: {0}")]
    ProtoRepoError(#[from] crate::git::repository::ProtoRepoError),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error(transparent)]
    Resolver(anyhow::Error),
}

pub fn resolve(
    descriptor: &Descriptor,
    resolver: &impl ModuleResolver,
) -> Result<(ResolvedModule, LockFile), FetchError> {
    fn go(
        resolver: &impl ModuleResolver,
        results: &mut BTreeMap<DependencyName, (LockedDependency, ResolvedDependency)>,
        dependencies: &[Dependency],
    ) -> Result<(), FetchError> {
        let mut children = Vec::new();
        for dependency in dependencies {
            match results.get(&dependency.name) {
                None => {
                    log::info!("Resolving {}", dependency.coordinate);
                    let CommitAndDescriptor {
                        commit_hash,
                        mut descriptor,
                    } = resolver
                        .resolve(
                            &dependency.coordinate,
                            &dependency.specification,
                            None,
                            &dependency.name,
                        )
                        .map_err(FetchError::Resolver)?;

                    let locked = LockedDependency {
                        name: dependency.name.clone(),
                        commit_hash: commit_hash.clone(),
                        coordinate: dependency.coordinate.clone(),
                        specification: dependency.specification.clone(),
                    };

                    let resolved = ResolvedDependency {
                        name: dependency.name.clone(),
                        commit_hash,
                        coordinate: dependency.coordinate.clone(),
                        specification: dependency.specification.clone(),
                        rules: dependency.rules.clone(),
                        dependencies: descriptor
                            .dependencies
                            .iter()
                            .map(|d| d.name.clone())
                            .collect(),
                    };

                    results.insert(dependency.name.clone(), (locked, resolved));
                    children.append(&mut descriptor.dependencies);
                }
                Some((already_locked, _)) => {
                    if already_locked.coordinate != dependency.coordinate {
                        log::warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.coordinate,
                            already_locked.coordinate,
                            &dependency.name.value
                        );
                    } else if already_locked.specification != dependency.specification {
                        log::warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.specification,
                            already_locked.specification,
                            &dependency.name.value
                        );
                    }
                }
            }
        }

        if !children.is_empty() {
            go(resolver, results, &children)?;
        }

        Ok(())
    }

    let mut results = BTreeMap::new();

    go(resolver, &mut results, &descriptor.dependencies)?;

    let (locked, resolved) = results.into_values().unzip();

    let resolved = ResolvedModule {
        module_name: descriptor.name.clone(),
        dependencies: resolved,
    };

    let lockfile = LockFile {
        module_name: descriptor.name.clone(),
        dependencies: locked,
    };

    Ok((resolved, lockfile))
}

pub fn fetch_sources(
    cache: &impl RepositoryCache,
    dependencies: &[ResolvedDependency],
) -> Result<(), FetchError> {
    info!("Fetching dependencies source files...");
    for dependency in dependencies {
        cache
            .fetch(
                &dependency.coordinate,
                &dependency.specification,
                &dependency.commit_hash,
            )
            .map_err(FetchError::Cache)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use crate::{
        model::protofetch::{Coordinate, Revision, RevisionSpecification, Rules},
        resolver::CommitAndDescriptor,
    };

    use super::*;

    use pretty_assertions::assert_eq;

    #[derive(Default)]
    struct FakeModuleResolver {
        entries: BTreeMap<Coordinate, BTreeMap<RevisionSpecification, CommitAndDescriptor>>,
    }

    impl FakeModuleResolver {
        fn push(&mut self, name: &str, revision: &str, commit_hash: &str, descriptor: Descriptor) {
            self.entries.entry(coordinate(name)).or_default().insert(
                RevisionSpecification {
                    revision: Revision::pinned(revision),
                    branch: None,
                },
                CommitAndDescriptor {
                    commit_hash: commit_hash.to_string(),
                    descriptor,
                },
            );
        }
    }

    impl ModuleResolver for FakeModuleResolver {
        fn resolve(
            &self,
            coordinate: &Coordinate,
            specification: &RevisionSpecification,
            _: Option<&str>,
            _: &DependencyName,
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

    fn coordinate(name: &str) -> Coordinate {
        Coordinate::from_url(&format!("example.com/org/{}", name)).unwrap()
    }

    fn dependency(name: &str, revision: &str) -> Dependency {
        Dependency {
            name: DependencyName {
                value: name.to_owned(),
            },
            coordinate: coordinate(name),
            specification: RevisionSpecification {
                revision: Revision::pinned(revision),
                branch: None,
            },
            rules: Rules::default(),
        }
    }

    fn locked_dependency(name: &str, revision: &str, commit_hash: &str) -> LockedDependency {
        LockedDependency {
            name: DependencyName {
                value: name.to_owned(),
            },
            coordinate: coordinate(name),
            specification: RevisionSpecification {
                revision: Revision::pinned(revision),
                branch: None,
            },
            commit_hash: commit_hash.to_owned(),
        }
    }

    #[test]
    fn resolve_transitive() {
        let mut resolver = FakeModuleResolver::default();
        resolver.push(
            "foo",
            "1.0.0",
            "c1",
            Descriptor {
                name: "foo".to_owned(),
                description: None,
                proto_out_dir: None,
                dependencies: vec![dependency("bar", "2.0.0")],
            },
        );

        resolver.push(
            "bar",
            "2.0.0",
            "c2",
            Descriptor {
                name: "bar".to_owned(),
                description: None,
                proto_out_dir: None,
                dependencies: Vec::new(),
            },
        );

        let (_, lockfile) = resolve(
            &Descriptor {
                name: "root".to_owned(),
                description: None,
                proto_out_dir: None,
                dependencies: vec![dependency("foo", "1.0.0")],
            },
            &resolver,
        )
        .unwrap();

        assert_eq!(
            lockfile,
            LockFile {
                module_name: "root".to_owned(),
                dependencies: vec![
                    locked_dependency("bar", "2.0.0", "c2"),
                    locked_dependency("foo", "1.0.0", "c1")
                ]
            }
        )
    }

    #[test]
    fn resolve_transitive_root_priority() {
        let mut resolver = FakeModuleResolver::default();
        resolver.push(
            "foo",
            "1.0.0",
            "c1",
            Descriptor {
                name: "foo".to_owned(),
                description: None,
                proto_out_dir: None,
                dependencies: vec![dependency("bar", "2.0.0")],
            },
        );

        resolver.push(
            "bar",
            "1.0.0",
            "c3",
            Descriptor {
                name: "bar".to_owned(),
                description: None,
                proto_out_dir: None,
                dependencies: Vec::new(),
            },
        );
        resolver.push(
            "bar",
            "2.0.0",
            "c2",
            Descriptor {
                name: "bar".to_owned(),
                description: None,
                proto_out_dir: None,
                dependencies: Vec::new(),
            },
        );

        let (_, lockfile) = resolve(
            &Descriptor {
                name: "root".to_owned(),
                description: None,
                proto_out_dir: None,
                dependencies: vec![dependency("foo", "1.0.0"), dependency("bar", "1.0.0")],
            },
            &resolver,
        )
        .unwrap();

        assert_eq!(
            lockfile,
            LockFile {
                module_name: "root".to_owned(),
                dependencies: vec![
                    locked_dependency("bar", "1.0.0", "c3"),
                    locked_dependency("foo", "1.0.0", "c1"),
                ]
            }
        )
    }
}
