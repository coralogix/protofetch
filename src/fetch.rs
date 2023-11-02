use std::{collections::BTreeMap, str::Utf8Error};

use crate::{
    cache::RepositoryCache,
    model::protofetch::{
        lock::{LockFile, LockedDependency},
        Dependency, DependencyName, Descriptor,
    },
    resolver::ModuleResolver,
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

pub fn lock(
    descriptor: &Descriptor,
    resolver: &impl ModuleResolver,
) -> Result<LockFile, FetchError> {
    fn go(
        resolver: &impl ModuleResolver,
        resolved: &mut BTreeMap<DependencyName, LockedDependency>,
        dependencies: &[Dependency],
    ) -> Result<(), FetchError> {
        let mut children = Vec::new();
        for dependency in dependencies {
            match resolved.get(&dependency.name) {
                None => {
                    log::info!("Resolving {}", dependency.coordinate);
                    let mut resolved_module = resolver
                        .resolve(
                            &dependency.coordinate,
                            &dependency.specification,
                            None,
                            &dependency.name,
                        )
                        .map_err(FetchError::Resolver)?;
                    let dependencies = resolved_module
                        .descriptor
                        .dependencies
                        .iter()
                        .map(|dep| dep.name.clone())
                        .collect();

                    let locked = LockedDependency {
                        name: dependency.name.clone(),
                        commit_hash: resolved_module.commit_hash,
                        coordinate: dependency.coordinate.clone(),
                        specification: dependency.specification.clone(),
                        dependencies,
                        rules: dependency.rules.clone(),
                    };

                    resolved.insert(dependency.name.clone(), locked);
                    children.append(&mut resolved_module.descriptor.dependencies);
                }
                Some(resolved) => {
                    if resolved.coordinate != dependency.coordinate {
                        log::warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.coordinate,
                            resolved.coordinate,
                            &dependency.name.value
                        );
                    } else if resolved.specification != dependency.specification {
                        log::warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.specification,
                            resolved.specification,
                            &dependency.name.value
                        );
                    }
                }
            }
        }

        if !children.is_empty() {
            go(resolver, resolved, &children)?;
        }

        Ok(())
    }

    let mut resolved = BTreeMap::new();

    go(resolver, &mut resolved, &descriptor.dependencies)?;

    Ok(LockFile {
        module_name: descriptor.name.clone(),
        dependencies: resolved.into_values().collect(),
    })
}

pub fn fetch_sources(cache: &impl RepositoryCache, lockfile: &LockFile) -> Result<(), FetchError> {
    info!("Fetching dependencies source files...");
    for dep in &lockfile.dependencies {
        cache
            .fetch(&dep.coordinate, &dep.specification, &dep.commit_hash)
            .map_err(FetchError::Cache)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use crate::{
        model::protofetch::{Coordinate, Revision, RevisionSpecification, Rules},
        resolver::ResolvedModule,
    };

    use super::*;

    use pretty_assertions::assert_eq;

    #[derive(Default)]
    struct FakeModuleResolver {
        entries: BTreeMap<Coordinate, BTreeMap<RevisionSpecification, ResolvedModule>>,
    }

    impl FakeModuleResolver {
        fn push(&mut self, name: &str, revision: &str, commit_hash: &str, descriptor: Descriptor) {
            self.entries.entry(coordinate(name)).or_default().insert(
                RevisionSpecification {
                    revision: Revision::pinned(revision),
                    branch: None,
                },
                ResolvedModule {
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
        ) -> anyhow::Result<ResolvedModule> {
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

    fn locked_dependency(
        name: &str,
        revision: &str,
        commit_hash: &str,
        dependencies: &[&str],
    ) -> LockedDependency {
        LockedDependency {
            name: DependencyName {
                value: name.to_owned(),
            },
            coordinate: coordinate(name),
            specification: RevisionSpecification {
                revision: Revision::pinned(revision),
                branch: None,
            },
            rules: Rules::default(),
            commit_hash: commit_hash.to_owned(),
            dependencies: dependencies
                .iter()
                .map(|s| DependencyName {
                    value: s.to_string(),
                })
                .collect(),
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

        let lock_file = lock(
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
            lock_file,
            LockFile {
                module_name: "root".to_owned(),
                dependencies: vec![
                    locked_dependency("bar", "2.0.0", "c2", &[]),
                    locked_dependency("foo", "1.0.0", "c1", &["bar"])
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

        let lock_file = lock(
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
            lock_file,
            LockFile {
                module_name: "root".to_owned(),
                dependencies: vec![
                    locked_dependency("bar", "1.0.0", "c3", &[]),
                    locked_dependency("foo", "1.0.0", "c1", &["bar"]),
                ]
            }
        )
    }
}
