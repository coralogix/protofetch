use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use log::{debug, info, trace};
use thiserror::Error;

use crate::{
    cache::RepositoryCache,
    model::protofetch::{
        resolved::{ResolvedDependency, ResolvedModule},
        AllowPolicies, DenyPolicies, ModuleName,
    },
};

#[derive(Error, Debug)]
pub enum ProtoError {
    #[error("Bad proto path. {0}")]
    BadPath(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error(transparent)]
    Cache(anyhow::Error),
}

/// Represents a mapping for a proto file between the source repo directory and the desired target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProtoFileMapping {
    from: PathBuf,
    to: PathBuf,
}

/// Proto file canonical representation
/// * full_path: the full path to the proto file
/// * package_path: the package path of the proto file
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProtoFileCanonicalMapping {
    full_path: PathBuf,
    package_path: PathBuf,
}

/// proto_dir: Base path to the directory where the proto files are to be copied to
/// cache_src_dir: Base path to the directory where the dependencies sources are cached
/// lockfile: The lockfile that contains the dependencies to be copied
pub fn copy_proto_files(
    cache: &impl RepositoryCache,
    resolved: &ResolvedModule,
    proto_dir: &Path,
) -> Result<(), ProtoError> {
    info!(
        "Copying proto files from {} descriptor...",
        resolved.module_name
    );
    if !proto_dir.exists() {
        std::fs::create_dir_all(proto_dir)?;
    }

    let deps = collect_all_root_dependencies(resolved);

    for dep in &deps {
        let dep_cache_dir = cache
            .create_worktree(&dep.coordinate, &dep.commit_hash, &dep.name)
            .map_err(ProtoError::Cache)?;
        let sources_to_copy: HashSet<ProtoFileMapping> = if !dep.rules.prune {
            copy_all_proto_files_for_dep(&dep_cache_dir, dep)?
        } else {
            pruned_transitive_dependencies(cache, dep, resolved)?
        };
        let without_denied_files = sources_to_copy
            .into_iter()
            .filter(|m| !DenyPolicies::should_deny_file(&dep.rules.deny_policies, &m.to))
            .collect();
        copy_proto_sources_for_dep(proto_dir, &dep_cache_dir, dep, &without_denied_files)?;
    }
    Ok(())
}

/// Copy all proto files for a dependency to the proto_dir
/// Takes into account content_roots and Allow list rules
fn copy_all_proto_files_for_dep(
    dep_cache_dir: &Path,
    dep: &ResolvedDependency,
) -> Result<HashSet<ProtoFileMapping>, ProtoError> {
    let mut proto_mapping: Vec<ProtoFileMapping> = Vec::new();
    for file in dep_cache_dir.read_dir()? {
        let path = file?.path();
        let proto_files = find_proto_files(path.as_path())?;
        for proto_file_source in proto_files {
            let proto_src = path_strip_prefix(&proto_file_source, dep_cache_dir)?;
            let proto_package_path = zoom_in_content_root(dep, &proto_src)?;
            if !AllowPolicies::should_allow_file(&dep.rules.allow_policies, &proto_package_path) {
                trace!(
                    "Filtering out proto file {} based on allow_policies rules.",
                    &proto_file_source.to_string_lossy()
                );
                continue;
            }
            proto_mapping.push(ProtoFileMapping {
                from: proto_src,
                to: proto_package_path,
            });
        }
    }
    Ok(proto_mapping.into_iter().collect())
}

/// Returns an HashSet of ProtoFileMapping to the proto files that `dep` depends on. It recursively
/// iterates all the dependencies of `dep` and its transitive dependencies based on imports
/// until no new dependencies are found.
fn pruned_transitive_dependencies(
    cache: &impl RepositoryCache,
    dep: &ResolvedDependency,
    lockfile: &ResolvedModule,
) -> Result<HashSet<ProtoFileMapping>, ProtoError> {
    fn process_mapping_file(
        cache: &impl RepositoryCache,
        mapping: ProtoFileCanonicalMapping,
        dep: &ResolvedDependency,
        lockfile: &ResolvedModule,
        visited: &mut HashSet<PathBuf>,
        deps: &mut HashSet<ProtoFileCanonicalMapping>,
    ) -> Result<(), ProtoError> {
        visited.insert(mapping.package_path.clone());
        let file_deps = extract_proto_dependencies_from_file(mapping.full_path.as_path())?;
        let mut dependencies = collect_transitive_dependencies(dep, lockfile);
        dependencies.push(dep.clone());
        let mut new_mappings = canonical_mapping_for_proto_files(cache, &file_deps, &dependencies)?;
        trace!("Adding {:?}.", &new_mappings);
        new_mappings.push(mapping);
        deps.extend(new_mappings.clone());
        Ok(())
    }

    /// Recursively loop through all the file dependencies based on imports
    /// Looks in own repository and in transitive dependencies.
    fn inner_loop(
        cache: &impl RepositoryCache,
        dep: &ResolvedDependency,
        lockfile: &ResolvedModule,
        visited: &mut HashSet<PathBuf>,
        found_proto_deps: &mut HashSet<ProtoFileCanonicalMapping>,
    ) -> Result<(), ProtoError> {
        let dep_dir = cache
            .create_worktree(&dep.coordinate, &dep.commit_hash, &dep.name)
            .map_err(ProtoError::Cache)?;
        for dir in dep_dir.read_dir()? {
            let proto_files = find_proto_files(&dir?.path())?;
            let filtered_mapping = filtered_proto_files(proto_files, &dep_dir, dep, false)
                .into_iter()
                .collect();
            let file_dependencies: HashSet<ProtoFileCanonicalMapping> = found_proto_deps
                .intersection(&filtered_mapping)
                .cloned()
                .collect();
            let file_dependencies_not_visited: HashSet<ProtoFileCanonicalMapping> =
                file_dependencies
                    .into_iter()
                    .filter(|p| !visited.contains(&p.package_path))
                    .collect();
            for mapping in file_dependencies_not_visited {
                process_mapping_file(cache, mapping, dep, lockfile, visited, found_proto_deps)?;
                inner_loop(cache, dep, lockfile, visited, found_proto_deps)?;
            }
        }
        Ok(())
    }

    let mut found_proto_deps: HashSet<ProtoFileCanonicalMapping> = HashSet::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut visited_dep: HashSet<ModuleName> = HashSet::new();
    debug!("Extracting proto files for {}", &dep.name);

    let dep_dir = cache
        .create_worktree(&dep.coordinate, &dep.commit_hash, &dep.name)
        .map_err(ProtoError::Cache)?;
    for dir in dep_dir.read_dir()? {
        let proto_files = find_proto_files(&dir?.path())?;
        let filtered_mapping = filtered_proto_files(proto_files, &dep_dir, dep, true);
        trace!("Filtered size {:?}.", &filtered_mapping.len());
        for mapping in filtered_mapping {
            process_mapping_file(
                cache,
                mapping,
                dep,
                lockfile,
                &mut visited,
                &mut found_proto_deps,
            )?;
            inner_loop(cache, dep, lockfile, &mut visited, &mut found_proto_deps)?;
        }
    }

    // Select proto files for the transitive dependencies of this dependency
    let t_deps = collect_transitive_dependencies(dep, lockfile);
    for t_dep in t_deps {
        trace!(
            "Extracting transitive proto dependencies from {} for dependency {} ",
            &t_dep.name,
            &dep.name
        );
        visited_dep.insert(t_dep.name.clone());
        inner_loop(cache, &t_dep, lockfile, &mut visited, &mut found_proto_deps)?;
    }
    debug!(
        "Found {:?} proto files for dependency {}",
        found_proto_deps.len(),
        dep.name
    );
    Ok(found_proto_deps
        .into_iter()
        .map(|p| ProtoFileMapping {
            from: p.full_path,
            to: p.package_path,
        })
        .collect())
}

fn copy_proto_sources_for_dep(
    proto_dir: &Path,
    dep_cache_dir: &Path,
    dep: &ResolvedDependency,
    sources_to_copy: &HashSet<ProtoFileMapping>,
) -> Result<(), ProtoError> {
    debug!(
        "Copying {:?} proto files for dependency {}",
        sources_to_copy.len(),
        dep.name
    );
    for mapping in sources_to_copy {
        trace!(
            "Copying proto file from {} to {}",
            &mapping.from.to_string_lossy(),
            &mapping.to.to_string_lossy()
        );
        let proto_file_source = dep_cache_dir.join(&mapping.from);
        let proto_file_out = proto_dir.join(&mapping.to);
        let prefix = proto_file_out.parent().ok_or_else(|| {
            ProtoError::BadPath(format!(
                "Bad parent dest file for {}",
                &proto_file_out.to_string_lossy()
            ))
        })?;
        std::fs::create_dir_all(prefix)?;
        std::fs::copy(proto_file_source, proto_file_out.as_path())?;
    }
    Ok(())
}

/// Extracts the dependencies from a proto file
fn extract_proto_dependencies_from_file(file: &Path) -> Result<Vec<PathBuf>, ProtoError> {
    let mut dependencies = Vec::new();
    let mut reader = BufReader::new(File::open(file)?);
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if line.starts_with("import ") {
            if let Some(dependency) = line.split_whitespace().nth(1) {
                let dependency = dependency.to_string().replace([';', '\"'], "");
                dependencies.push(PathBuf::from(dependency));
            }
        }
        line.clear();
    }
    Ok(dependencies)
}

/// Find proto files in a directory
fn find_proto_files(dir: &Path) -> Result<Vec<PathBuf>, ProtoError> {
    let mut files: Vec<PathBuf> = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let rec_call = find_proto_files(&path)?;
                files.append(&mut rec_call.clone());
            } else if let Some(extension) = path.extension() {
                if extension == "proto" {
                    files.push(path);
                }
            }
        }
    }
    Ok(files)
}

///From a dep and a lockfile, returns the transitive dependencies of the dep
fn collect_transitive_dependencies(
    dep: &ResolvedDependency,
    lockfile: &ResolvedModule,
) -> Vec<ResolvedDependency> {
    lockfile
        .dependencies
        .clone()
        .into_iter()
        .filter(|x| dep.dependencies.contains(&x.name) || x.rules.transitive)
        .collect::<Vec<_>>()
}

/// Collects all root dependencies based on pruning rules and transitive dependencies
/// This still has a limitation. At the moment.
/// If a dependency is flagged as transitive it will only be included in transitive fetching which uses pruning.
fn collect_all_root_dependencies(resolved: &ResolvedModule) -> Vec<ResolvedDependency> {
    let mut deps = Vec::new();

    for dep in &resolved.dependencies {
        let pruned = resolved
            .dependencies
            .iter()
            .any(|iter_dep| iter_dep.dependencies.contains(&dep.name) && iter_dep.rules.prune);

        let non_pruned = resolved
            .dependencies
            .iter()
            .any(|iter_dep| iter_dep.dependencies.contains(&dep.name) && !iter_dep.rules.prune);

        if (!pruned && !dep.rules.transitive) || non_pruned {
            deps.push(dep.clone());
        }
    }
    deps
}

/// This is removing the prefix which is needed to actually load file to extract protos from imports
fn filtered_proto_files(
    proto_files: Vec<PathBuf>,
    dep_dir: &Path,
    dep: &ResolvedDependency,
    should_filter: bool,
) -> Vec<ProtoFileCanonicalMapping> {
    proto_files
        .into_iter()
        .filter_map(|p| {
            let path = path_strip_prefix(&p, dep_dir).ok()?;
            let zoom = zoom_in_content_root(dep, &path).ok()?;
            if AllowPolicies::should_allow_file(&dep.rules.allow_policies, &zoom) || !should_filter
            {
                Some(ProtoFileCanonicalMapping {
                    full_path: p,
                    package_path: zoom,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Takes a slice of proto files, cache source directory and a slice of dependencies associated with these files
/// and builds the full proto file paths from the package path returning a ProtoFileCanonicalMapping.
/// This is used to be able to later on copy the files from the source directory to the user defined output directory.
fn canonical_mapping_for_proto_files(
    cache: &impl RepositoryCache,
    proto_files: &[PathBuf],
    deps: &[ResolvedDependency],
) -> Result<Vec<ProtoFileCanonicalMapping>, ProtoError> {
    let r: Result<Vec<ProtoFileCanonicalMapping>, ProtoError> = proto_files
        .iter()
        .map(|p| {
            let zoom_out = zoom_out_content_root(cache, deps, p)?;
            Ok(ProtoFileCanonicalMapping {
                full_path: zoom_out,
                package_path: p.to_path_buf(),
            })
        })
        .collect::<Result<Vec<_>, _>>();
    r
}

/// Remove content_root part of path if found
fn zoom_in_content_root(
    dep: &ResolvedDependency,
    proto_file_source: &Path,
) -> Result<PathBuf, ProtoError> {
    let mut proto_src = proto_file_source.to_path_buf();
    if !dep.rules.content_roots.is_empty() {
        let root = dep
            .rules
            .content_roots
            .iter()
            .find(|c_root| proto_file_source.starts_with(&c_root.value));
        if let Some(c_root) = root {
            trace!(
                "[Zoom in] Found valid content root {} for {}.",
                c_root.value.to_string_lossy(),
                proto_file_source.to_string_lossy()
            );
            proto_src = path_strip_prefix(proto_file_source, &c_root.value)?;
        }
    }
    Ok(proto_src)
}

fn zoom_out_content_root(
    cache: &impl RepositoryCache,
    deps: &[ResolvedDependency],
    proto_file_source: &Path,
) -> Result<PathBuf, ProtoError> {
    let mut proto_src = proto_file_source.to_path_buf();
    for dep in deps {
        let dep_dir = cache
            .create_worktree(&dep.coordinate, &dep.commit_hash, &dep.name)
            .map_err(ProtoError::Cache)?;
        for dir in dep_dir.read_dir()? {
            let proto_files = find_proto_files(&dir?.path())?;
            if let Some(path) = proto_files
                .into_iter()
                .find(|p| p.ends_with(proto_file_source))
            {
                trace!(
                    "[Zoom out] Found path root {} for {}.",
                    path.to_string_lossy(),
                    proto_file_source.to_string_lossy()
                );
                proto_src = path;
            }
        }
    }
    Ok(proto_src)
}

fn path_strip_prefix(path: &Path, prefix: &Path) -> Result<PathBuf, ProtoError> {
    path.strip_prefix(prefix)
        .map_err(|_err| {
            {
                ProtoError::BadPath(format!(
                    "Could not create proto source file path in {}. Wrong base dir {}",
                    path.to_string_lossy(),
                    prefix.to_string_lossy()
                ))
            }
        })
        .map(|s| s.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashSet},
        path::{Path, PathBuf},
    };

    use crate::model::protofetch::{ContentRoot, Coordinate, RevisionSpecification, Rules};

    use super::*;

    use pretty_assertions::assert_eq;

    struct FakeCache {
        root: PathBuf,
    }

    impl RepositoryCache for FakeCache {
        fn fetch(&self, _: &Coordinate, _: &RevisionSpecification, _: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn create_worktree(
            &self,
            _: &Coordinate,
            commit_hash: &str,
            name: &ModuleName,
        ) -> anyhow::Result<PathBuf> {
            Ok(self.root.join(name.as_str()).join(commit_hash))
        }
    }

    #[test]
    fn content_root_dependencies() {
        let cache_dir = project_root::get_project_root()
            .unwrap()
            .join(Path::new("resources/cache/dep3/hash3"));
        let lock_file = ResolvedDependency {
            name: ModuleName::new("dep3".to_string()),
            commit_hash: "hash3".to_string(),
            coordinate: Coordinate::from_url("example.com/org/dep3").unwrap(),
            specification: RevisionSpecification::default(),
            dependencies: BTreeSet::new(),
            rules: Rules {
                content_roots: BTreeSet::from([ContentRoot::from_string("root")]),
                ..Default::default()
            },
        };
        let expected_dep_1: HashSet<PathBuf> = vec![
            PathBuf::from("proto/example.proto"),
            PathBuf::from("proto/root.proto"),
        ]
        .into_iter()
        .collect();

        let result: HashSet<PathBuf> = copy_all_proto_files_for_dep(&cache_dir, &lock_file)
            .unwrap()
            .into_iter()
            .map(|p| p.to)
            .collect();

        assert_eq!(result, expected_dep_1);
    }

    #[test]
    fn pruned_dependencies() {
        let cache_dir = project_root::get_project_root()
            .unwrap()
            .join("resources/cache");
        let lock_file = ResolvedModule {
            module_name: ModuleName::from("test"),
            dependencies: vec![
                ResolvedDependency {
                    name: ModuleName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep1").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::from([ModuleName::new("dep2".to_string())]),
                    rules: Rules {
                        prune: true,
                        allow_policies: AllowPolicies::new(BTreeSet::from([
                            "/proto/example.proto".parse().unwrap(),
                        ])),
                        ..Default::default()
                    },
                },
                ResolvedDependency {
                    name: ModuleName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep2").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
            ],
        };
        let expected_dep_1: HashSet<PathBuf> = vec![
            PathBuf::from("proto/example.proto"),
            PathBuf::from("proto/example2.proto"),
            PathBuf::from("proto/example3.proto"),
            PathBuf::from("proto/example5.proto"),
            PathBuf::from("scalapb/scalapb.proto"),
            PathBuf::from("google/protobuf/descriptor.proto"),
            PathBuf::from("google/protobuf/struct.proto"),
        ]
        .into_iter()
        .collect();

        let pruned1: HashSet<PathBuf> = pruned_transitive_dependencies(
            &FakeCache { root: cache_dir },
            lock_file.dependencies.first().unwrap(),
            &lock_file,
        )
        .unwrap()
        .into_iter()
        .map(|p| p.to)
        .collect();

        assert_eq!(pruned1, expected_dep_1);
    }

    #[test]
    fn extract_dependencies_test() {
        let path = project_root::get_project_root()
            .unwrap()
            .join(Path::new("resources/proto_out/example2.proto"));
        let dependencies = extract_proto_dependencies_from_file(&path).unwrap();
        assert_eq!(dependencies.len(), 3);
        assert_eq!(dependencies[0].to_string_lossy(), "scalapb/scalapb.proto");
        assert_eq!(
            dependencies[1].to_string_lossy(),
            "google/protobuf/descriptor.proto"
        );
        assert_eq!(
            dependencies[2].to_string_lossy(),
            "google/protobuf/struct.proto"
        );
    }

    #[test]
    fn collect_transitive_dependencies_test() {
        let lock_file = ResolvedModule {
            module_name: ModuleName::from("test"),
            dependencies: vec![
                ResolvedDependency {
                    name: ModuleName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep1").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::from([
                        ModuleName::new("dep2".to_string()),
                        ModuleName::new("dep3".to_string()),
                    ]),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep2").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep3".to_string()),
                    commit_hash: "hash3".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep3").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep4".to_string()),
                    commit_hash: "hash4".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep4").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules {
                        transitive: true,
                        ..Default::default()
                    },
                },
            ],
        };

        let mut it = lock_file.dependencies.iter();
        let result = collect_transitive_dependencies(it.next().unwrap(), &lock_file);
        assert_eq!(result.len(), 3);
        assert!(result.contains(it.next().unwrap()));
        assert!(result.contains(it.next().unwrap()));
        assert!(result.contains(it.next().unwrap()));
    }

    #[test]
    fn collect_all_root_dependencies_() {
        let lock_file = ResolvedModule {
            module_name: ModuleName::from("test"),
            dependencies: vec![
                ResolvedDependency {
                    name: ModuleName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep1").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep2").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep3".to_string()),
                    commit_hash: "hash3".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep3").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
            ],
        };

        let result = collect_all_root_dependencies(&lock_file);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn collect_all_root_dependencies_filtered() {
        let lock_file = ResolvedModule {
            module_name: ModuleName::from("test"),
            dependencies: vec![
                ResolvedDependency {
                    name: ModuleName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep1").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::from([ModuleName::new("dep2".to_string())]),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep2").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep3".to_string()),
                    commit_hash: "hash3".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep3").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::from([
                        ModuleName::new("dep2".to_string()),
                        ModuleName::new("dep5".to_string()),
                    ]),
                    rules: Rules {
                        prune: true,
                        transitive: false,
                        ..Default::default()
                    },
                },
                ResolvedDependency {
                    name: ModuleName::new("dep4".to_string()),
                    commit_hash: "hash4".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep4").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                ResolvedDependency {
                    name: ModuleName::new("dep5".to_string()),
                    commit_hash: "hash5".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep5").unwrap(),
                    specification: RevisionSpecification::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules {
                        prune: false,
                        transitive: true,
                        ..Default::default()
                    },
                },
            ],
        };

        let result = collect_all_root_dependencies(&lock_file);
        assert_eq!(result.len(), 4);
    }
}
