use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::Utf8Error,
};

use crate::{
    cache::{CacheError, RepositoryCache},
    model::protofetch::{
        lock::{LockFile, LockedDependency},
        Dependency, DependencyName, Descriptor, RevisionSpecification,
    },
    proto_repository::ProtoRepository,
    resolver::ModuleResolver,
};
use log::{debug, error, info};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache: {0}")]
    Cache(#[from] CacheError),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob: {0}")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Bad output dir {0}")]
    BadOutputDir(String),
    #[error("Error while processing protobuf repository: {0}")]
    ProtoRepoError(#[from] crate::proto_repository::ProtoRepoError),
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
        resolved: &mut BTreeMap<DependencyName, (RevisionSpecification, LockedDependency)>,
        dependencies: &[Dependency],
    ) -> Result<(), FetchError> {
        let mut children = Vec::new();
        for dependency in dependencies {
            match resolved.get(&dependency.name) {
                None => {
                    log::info!("Resolving {:?}", dependency.coordinate);
                    let mut resolved_module = resolver
                        .resolve(
                            &dependency.coordinate,
                            &dependency.specification,
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
                        dependencies,
                        rules: dependency.rules.clone(),
                    };

                    resolved.insert(
                        dependency.name.clone(),
                        (dependency.specification.clone(), locked),
                    );
                    children.append(&mut resolved_module.descriptor.dependencies);
                }
                Some((resolved_specification, resolved)) => {
                    if resolved.coordinate != dependency.coordinate {
                        log::warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.coordinate,
                            resolved.coordinate,
                            &dependency.name.value
                        );
                    } else if resolved_specification != &dependency.specification {
                        log::warn!(
                            "discarded {} in favor of {} for {}",
                            dependency.specification,
                            resolved_specification,
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
        dependencies: resolved
            .into_values()
            .map(|(_, dependency)| dependency)
            .collect(),
    })
}

pub fn fetch_sources<Cache: RepositoryCache>(
    cache: &Cache,
    lockfile: &LockFile,
    cache_src_dir: &Path,
) -> Result<(), FetchError> {
    info!("Fetching dependencies source files...");

    if !cache_src_dir.exists() {
        std::fs::create_dir_all(cache_src_dir)?;
    }

    if cache_src_dir.is_dir() {
        for dep in &lockfile.dependencies {
            //If the dependency is already in the cache, we don't need to fetch it again
            if cache_src_dir
                .join(&dep.name.value)
                .join(PathBuf::from(&dep.commit_hash))
                .exists()
            {
                debug!("Skipping fetching {:?}. Already in cache", dep.name);
                continue;
            }
            let repo = cache.clone_or_update(&dep.coordinate)?;
            let work_tree_res = repo.create_worktrees(
                &lockfile.module_name,
                &dep.name,
                &dep.commit_hash,
                cache_src_dir,
            );
            if let Err(err) = work_tree_res {
                error!("Error while trying to create worktrees {err}. \
                Most likely the worktree sources have been deleted but the worktree metadata has not. \
                Please delete the cache and run protofetch fetch again.")
            }
        }
        Ok(())
    } else {
        Err(FetchError::BadOutputDir(
            cache_src_dir.to_str().unwrap_or("").to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use crate::{
        model::protofetch::{Coordinate, Protocol, Revision, RevisionSpecification, Rules},
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
        Coordinate::from_url(&format!("example.com/org/{}", name), Protocol::Https).unwrap()
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

    fn locked_dependency(name: &str, commit_hash: &str, dependencies: &[&str]) -> LockedDependency {
        LockedDependency {
            name: DependencyName {
                value: name.to_owned(),
            },
            coordinate: coordinate(name),
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
                    locked_dependency("bar", "c2", &[]),
                    locked_dependency("foo", "c1", &["bar"])
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
                    locked_dependency("bar", "c3", &[]),
                    locked_dependency("foo", "c1", &["bar"]),
                ]
            }
        )
    }
}
