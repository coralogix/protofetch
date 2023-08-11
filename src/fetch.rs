use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    str::Utf8Error,
};

use crate::{
    cache::{CacheError, RepositoryCache},
    model::protofetch::{
        Coordinate, Dependency, DependencyName, Descriptor, LockFile, LockedDependency, Revision,
        RevisionSpecification, Rules,
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
type ValueWithRevision = (
    Rules,
    Coordinate,
    Box<dyn ProtoRepository>,
    RevisionSpecification,
    Vec<DependencyName>,
);

pub fn lock<Cache: RepositoryCache>(
    descriptor: &Descriptor,
    cache: &Cache,
) -> Result<LockFile, FetchError> {
    type Value = (
        Rules,
        Coordinate,
        Box<dyn ProtoRepository>,
        Vec<DependencyName>,
    );

    fn go<Cache: RepositoryCache>(
        cache: &Cache,
        dep_map: &mut HashMap<DependencyName, Vec<RevisionSpecification>>,
        repo_map: &mut HashMap<DependencyName, Value>,
        dependencies: &[Dependency],
        parent: Option<&DependencyName>,
    ) -> Result<(), FetchError> {
        for dependency in dependencies {
            log::info!("Resolving {:?}", dependency.coordinate);

            dep_map
                .entry(dependency.name.clone())
                .and_modify(|vec| vec.push(dependency.specification.clone()))
                .or_insert_with(|| vec![dependency.specification.clone()]);

            let repo = cache.clone_or_update(&dependency.coordinate)?;
            let descriptor =
                repo.extract_descriptor(&dependency.name, &dependency.specification)?;

            repo_map.entry(dependency.name.clone()).or_insert((
                dependency.rules.clone(),
                dependency.coordinate.clone(),
                repo,
                vec![],
            ));

            if let Some(p) = parent {
                repo_map
                    .entry(p.clone())
                    .and_modify(|(_r, _c, _p, deps)| deps.push(dependency.name.clone()));
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
    let mut dep_map: HashMap<DependencyName, Vec<RevisionSpecification>> = HashMap::new();
    let mut repo_map: HashMap<DependencyName, Value> = HashMap::new();

    go(
        cache,
        &mut dep_map,
        &mut repo_map,
        &descriptor.dependencies,
        None,
    )?;

    let no_conflicts = resolve_conflicts(dep_map);

    let with_revision: HashMap<DependencyName, ValueWithRevision> = no_conflicts
        .into_iter()
        .filter_map(|(coordinate, revision)| {
            repo_map
                .remove(&coordinate)
                .map(|(rules, dep_name, repo, deps)| {
                    (coordinate, (rules, dep_name, repo, revision, deps))
                })
        })
        .collect();

    let locked_dependencies = locked_dependencies(&with_revision)?;

    Ok(LockFile::new(
        descriptor.name.clone(),
        descriptor.proto_out_dir.clone(),
        locked_dependencies,
    ))
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
    dep_map: HashMap<DependencyName, Vec<RevisionSpecification>>,
) -> HashMap<DependencyName, RevisionSpecification> {
    dep_map
        .into_iter()
        .filter_map(|(name, mut specs)| match specs.len() {
            0 => None,
            1 => Some((name, specs.remove(0))),
            _ => {
                specs.sort();
                let mut result = RevisionSpecification::default();
                for spec in specs.into_iter().rev() {
                    let RevisionSpecification {
                        revision: spec_revision,
                        branch: spec_branch,
                    } = spec;
                    if let Revision::Pinned { revision } = &spec_revision {
                        match &result.revision {
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
                                result.revision = spec_revision;
                            }
                        }
                    }
                    if let Some(branch) = &spec_branch {
                        match &result.branch {
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
                            None => result.branch = spec_branch,
                        }
                    }
                }
                Some((name, result))
            }
        })
        .collect()
}

fn locked_dependencies(
    dep_map: &HashMap<DependencyName, ValueWithRevision>,
) -> Result<BTreeSet<LockedDependency>, FetchError> {
    let mut locked_deps: BTreeSet<LockedDependency> = BTreeSet::new();
    for (name, (rules, coordinate, repository, specification, deps)) in dep_map {
        log::info!("Locking {:?} at {:?}", coordinate, specification);

        let commit_hash = repository.resolve_commit_hash(specification)?;
        let locked_dep = LockedDependency {
            name: name.clone(),
            commit_hash,
            coordinate: coordinate.clone(),
            dependencies: BTreeSet::from_iter(deps.clone()),
            rules: rules.clone(),
        };

        locked_deps.insert(locked_dep);
    }
    Ok(locked_deps)
}

#[cfg(test)]
mod tests {
    use crate::model::protofetch::Revision;

    use super::*;

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
                    coordinate: Coordinate {
                        forge: "github.com".to_string(),
                        organization: "org".to_string(),
                        repository: "repo".to_string(),
                        protocol: Protocol::Https,
                    },
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
                    coordinate: Coordinate {
                        forge: "github.com".to_string(),
                        organization: "org".to_string(),
                        repository: "repo".to_string(),
                        protocol: Protocol::Https,
                    },
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
                    coordinate: Coordinate {
                        forge: "github.com".to_string(),
                        organization: "org".to_string(),
                        repository: "repo".to_string(),
                        protocol: Protocol::Https,
                    },
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
        let mut input = HashMap::new();
        let mut result = HashMap::new();
        let name = DependencyName::new("foo".to_string());
        input.insert(
            name.clone(),
            vec![
                RevisionSpecification {
                    revision: Revision::pinned("1.0.0"),
                    branch: Some("master".to_owned()),
                },
                RevisionSpecification {
                    revision: Revision::pinned("3.0.0"),
                    branch: None,
                },
                RevisionSpecification {
                    revision: Revision::pinned("2.0.0"),
                    branch: Some("main".to_owned()),
                },
            ],
        );
        result.insert(
            name,
            RevisionSpecification {
                revision: Revision::pinned("3.0.0"),
                branch: Some("main".to_owned()),
            },
        );
        assert_eq!(resolve_conflicts(input), result)
    }
}
