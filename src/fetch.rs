use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    str::Utf8Error,
};

use crate::{
    cache::{CacheError, RepositoryCache},
    model::protofetch::{
        lock::{LockFile, LockedCoordinateRevisionSpecification, LockedDependency},
        Coordinate, Dependency, DependencyName, Descriptor, Revision, RevisionSpecification, Rules,
    },
    proto_repository::ProtoRepository,
};
use log::{debug, error, info};
use std::iter::FromIterator;
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
}

type CoordinateRevisionSpecification = (Coordinate, RevisionSpecification);

type ValueWithRevision = (
    Rules,
    Box<dyn ProtoRepository>,
    CoordinateRevisionSpecification,
    Vec<CoordinateRevisionSpecification>,
    Vec<DependencyName>,
);

pub fn lock<Cache: RepositoryCache>(
    descriptor: &Descriptor,
    cache: &Cache,
) -> Result<LockFile, FetchError> {
    type Value = (Rules, Box<dyn ProtoRepository>, Vec<DependencyName>);

    fn go<Cache: RepositoryCache>(
        cache: &Cache,
        dep_map: &mut BTreeMap<DependencyName, Vec<CoordinateRevisionSpecification>>,
        repo_map: &mut BTreeMap<DependencyName, Value>,
        dependencies: &[Dependency],
        parent: Option<&DependencyName>,
    ) -> Result<(), FetchError> {
        for dependency in dependencies {
            log::info!("Resolving {:?}", dependency.coordinate);

            dep_map.entry(dependency.name.clone()).or_default().push((
                dependency.coordinate.clone(),
                dependency.specification.clone(),
            ));

            let repo = cache.clone_or_update(&dependency.coordinate)?;
            let descriptor =
                repo.extract_descriptor(&dependency.name, &dependency.specification)?;

            repo_map.entry(dependency.name.clone()).or_insert((
                dependency.rules.clone(),
                repo,
                vec![],
            ));

            if let Some(p) = parent {
                repo_map
                    .entry(p.clone())
                    .and_modify(|(_, _, deps)| deps.push(dependency.name.clone()));
            }
            go(
                cache,
                dep_map,
                repo_map,
                &descriptor.dependencies,
                Some(&dependency.name),
            )?;
        }

        Ok(())
    }
    let mut dep_map: BTreeMap<DependencyName, Vec<CoordinateRevisionSpecification>> =
        BTreeMap::new();
    let mut repo_map: BTreeMap<DependencyName, Value> = BTreeMap::new();

    go(
        cache,
        &mut dep_map,
        &mut repo_map,
        &descriptor.dependencies,
        None,
    )?;

    let resolved = resolve_conflicts(&dep_map);

    let with_revision: BTreeMap<DependencyName, ValueWithRevision> = resolved
        .into_iter()
        .filter_map(|(dep_name, coord_spec)| {
            let specifications = dep_map
                .remove(&dep_name)
                .expect("no unknown dependency names after conflict resolution");
            repo_map.remove(&dep_name).map(|(rules, repo, deps)| {
                (dep_name, (rules, repo, coord_spec, specifications, deps))
            })
        })
        .collect();

    let locked_dependencies = locked_dependencies(with_revision)?;

    Ok(LockFile {
        module_name: descriptor.name.clone(),
        proto_out_dir: descriptor.proto_out_dir.clone(),
        dependencies: locked_dependencies,
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

//TODO: Make sure we get the last version. Getting the biggest string is extremely error prone.
//      Use semver
fn resolve_conflicts(
    dep_map: &BTreeMap<DependencyName, Vec<CoordinateRevisionSpecification>>,
) -> BTreeMap<DependencyName, CoordinateRevisionSpecification> {
    dep_map
        .iter()
        .filter_map(|(name, coord_specs)| {
            let mut coord_specs = coord_specs.iter().collect::<Vec<_>>();
            // Stable sort by specification
            coord_specs.sort_by(|l, r| l.1.cmp(&r.1).reverse());
            coord_specs
                .split_first()
                .map(|(&head, tail)| {
                    tail.iter().fold(
                        head.clone(),
                        |(result_coord, mut result_spec), (coord, spec)| {
                            if coord != &result_coord {
                                log::warn!(
                                    "discarded {} in favor of {} for {}",
                                    coord,
                                    result_coord,
                                    name.value
                                );
                            } else {
                                let RevisionSpecification {
                                    revision: spec_revision,
                                    branch: spec_branch,
                                } = spec;
                                if let Revision::Pinned { revision } = &spec_revision {
                                    match &result_spec.revision {
                                        Revision::Pinned {
                                            revision: result_revision,
                                        } => {
                                            if result_revision != revision {
                                                log::warn!(
                                                    "discarded revision {} in favor of {} for {}",
                                                    revision,
                                                    result_revision,
                                                    name.value
                                                )
                                            }
                                        }
                                        Revision::Arbitrary => {
                                            result_spec.revision = spec_revision.to_owned();
                                        }
                                    }
                                }
                                if let Some(branch) = &spec_branch {
                                    match &result_spec.branch {
                                        Some(result_branch) => {
                                            if result_branch != branch {
                                                log::warn!(
                                                    "discarded branch {} in favor of {} for {}",
                                                    branch,
                                                    result_branch,
                                                    name.value
                                                )
                                            }
                                        }
                                        None => result_spec.branch = spec_branch.to_owned(),
                                    }
                                }
                            }
                            (result_coord, result_spec)
                        },
                    )
                })
                .map(|result| (name.to_owned(), result))
        })
        .collect()
}

fn locked_dependencies(
    dep_map: BTreeMap<DependencyName, ValueWithRevision>,
) -> Result<Vec<LockedDependency>, FetchError> {
    let mut locked_deps = Vec::new();
    for (name, (rules, repository, (coordinate, specification), specifications, deps)) in dep_map {
        log::info!("Locking {:?} at {:?}", coordinate, specification);

        let commit_hash = repository.resolve_commit_hash(&specification)?;
        let locked_dep = LockedDependency {
            name: name.clone(),
            commit_hash,
            coordinate: coordinate.clone(),
            specifications: specifications
                .into_iter()
                .map(
                    |(coord, specification)| LockedCoordinateRevisionSpecification {
                        coordinate: Some(coord).filter(|x| x != &coordinate),
                        specification,
                    },
                )
                .collect(),
            dependencies: BTreeSet::from_iter(deps.clone()),
            rules: rules.clone(),
        };

        locked_deps.push(locked_dep);
    }
    Ok(locked_deps)
}

#[cfg(test)]
mod tests {
    use std::iter;

    use crate::model::protofetch::{Protocol, Revision};

    use super::*;

    use pretty_assertions::assert_eq;

    #[test]
    fn lock_from_descriptor_always_the_same() {
        use crate::{
            cache::MockRepositoryCache,
            model::protofetch::{Protocol, *},
            proto_repository::MockProtoRepository,
        };
        let mut mock_repo_cache = MockRepositoryCache::new();
        let desc = Descriptor {
            name: "test_file".to_string(),
            description: None,
            proto_out_dir: Some("./path/to/proto_out".to_string()),
            dependencies: vec![
                Dependency {
                    name: DependencyName::new("dependency1".to_string()),
                    coordinate: Coordinate::from_url("github.com/org/repo", Protocol::Https)
                        .unwrap(),
                    specification: RevisionSpecification {
                        revision: Revision::Pinned {
                            revision: "1.0.0".to_string(),
                        },
                        branch: None,
                    },
                    rules: Default::default(),
                },
                Dependency {
                    name: DependencyName::new("dependency2".to_string()),
                    coordinate: Coordinate::from_url("github.com/org/repo", Protocol::Https)
                        .unwrap(),
                    specification: RevisionSpecification {
                        revision: Revision::Pinned {
                            revision: "2.0.0".to_string(),
                        },
                        branch: None,
                    },
                    rules: Rules {
                        prune: true,
                        content_roots: BTreeSet::from([ContentRoot::from_string("src")]),
                        transitive: false,
                        allow_policies: AllowPolicies::new(BTreeSet::from([
                            FilePolicy::new(
                                PolicyKind::File,
                                PathBuf::from("/foo/proto/file.proto"),
                            ),
                            FilePolicy::new(PolicyKind::Prefix, PathBuf::from("/foo/other")),
                            FilePolicy::new(PolicyKind::SubPath, PathBuf::from("/some/path")),
                        ])),
                        deny_policies: DenyPolicies::new(BTreeSet::from([
                            FilePolicy::new(
                                PolicyKind::File,
                                PathBuf::from("/foo1/proto/file.proto"),
                            ),
                            FilePolicy::new(PolicyKind::Prefix, PathBuf::from("/foo1/other")),
                            FilePolicy::new(PolicyKind::SubPath, PathBuf::from("/some1/path")),
                        ])),
                    },
                },
                Dependency {
                    name: DependencyName::new("dependency3".to_string()),
                    coordinate: Coordinate::from_url("github.com/org/repo", Protocol::Https)
                        .unwrap(),
                    specification: RevisionSpecification {
                        revision: Revision::Pinned {
                            revision: "3.0.0".to_string(),
                        },
                        branch: None,
                    },
                    rules: Default::default(),
                },
            ],
        };

        mock_repo_cache.expect_clone_or_update().returning(|_| {
            let mut mock_repo = MockProtoRepository::new();
            mock_repo.expect_extract_descriptor().returning(
                |dep_name: &DependencyName, _: &RevisionSpecification| {
                    Ok(Descriptor {
                        name: dep_name.value.clone(),
                        description: None,
                        proto_out_dir: None,
                        dependencies: vec![],
                    })
                },
            );

            mock_repo
                .expect_resolve_commit_hash()
                .returning(|_| Ok("asjdlaksdjlaksjd".to_string()));
            Ok(Box::new(mock_repo))
        });

        let result = lock(&desc, &mock_repo_cache).unwrap();
        let value_toml = toml::Value::try_from(&result).unwrap();
        let string_file = toml::to_string_pretty(&value_toml).unwrap();

        for _n in 1..100 {
            let new_lock = lock(&desc, &mock_repo_cache).unwrap();
            let value_toml1 = toml::Value::try_from(&new_lock).unwrap();
            let sting_new_file = toml::to_string_pretty(&value_toml1).unwrap();

            assert_eq!(new_lock, result);
            assert_eq!(string_file, sting_new_file)
        }
    }

    #[test]
    fn resolve_conflict_picks_latest_revision_and_branch() {
        let coord1 = Coordinate::from_url("example.com/org/dep1", Protocol::Https).unwrap();
        let name = DependencyName::new("foo".to_string());
        let input = BTreeMap::from_iter(iter::once((
            name.clone(),
            vec![
                (
                    coord1.clone(),
                    RevisionSpecification {
                        revision: Revision::pinned("1.0.0"),
                        branch: Some("master".to_owned()),
                    },
                ),
                (
                    coord1.clone(),
                    RevisionSpecification {
                        revision: Revision::pinned("3.0.0"),
                        branch: None,
                    },
                ),
                (
                    coord1.clone(),
                    RevisionSpecification {
                        revision: Revision::pinned("2.0.0"),
                        branch: Some("main".to_owned()),
                    },
                ),
            ],
        )));
        let mut resolved = resolve_conflicts(&input);
        let resolved_coord_spec = resolved.remove(&name).expect("name is resolved");
        assert_eq!(
            resolved_coord_spec,
            (
                coord1,
                RevisionSpecification {
                    revision: Revision::pinned("3.0.0"),
                    branch: Some("main".to_owned()),
                },
            )
        );
    }

    #[test]
    fn resolve_conflict_picks_first_coordinate() {
        let coord1 = Coordinate::from_url("example.com/org/dep1", Protocol::Https).unwrap();
        let coord2 = Coordinate::from_url("example.com/org/dep2", Protocol::Https).unwrap();
        let coord3 = Coordinate::from_url("example.com/org/dep3", Protocol::Https).unwrap();
        let name = DependencyName::new("foo".to_string());
        let main = RevisionSpecification {
            revision: Revision::Arbitrary,
            branch: Some("main".to_owned()),
        };
        let input = BTreeMap::from_iter(iter::once((
            name.clone(),
            vec![
                (coord2.clone(), main.clone()),
                (coord1, main.clone()),
                (coord3, main.clone()),
            ],
        )));
        let mut resolved = resolve_conflicts(&input);
        let resolved_coord_spec = resolved.remove(&name).expect("name is resolved");
        assert_eq!(resolved_coord_spec, (coord2, main))
    }
}
