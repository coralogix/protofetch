use crate::model::protofetch::{AllowPolicies, DenyPolicies, LockFile, LockedDependency};
use derive_new::new;
use log::{debug, info, trace};
use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtoError {
    #[error("Bad proto path. {0}")]
    BadPath(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

/// Represents a mapping for a proto file between the source repo directory and the desired target.
#[derive(new, Debug, Clone, PartialEq, Eq, Hash)]
struct ProtoFileMapping {
    from: PathBuf,
    to: PathBuf,
}

/// Proto file canonical representation
/// * full_path: the full path to the proto file
/// * package_path: the package path of the proto file
#[derive(new, Debug, Clone, PartialEq, Eq, Hash)]
struct ProtoFileCanonicalMapping {
    full_path: PathBuf,
    package_path: PathBuf,
}

/// proto_dir: Base path to the directory where the proto files are to be copied to
/// cache_src_dir: Base path to the directory where the dependencies sources are cached
/// lockfile: The lockfile that contains the dependencies to be copied
pub fn copy_proto_files(
    proto_dir: &Path,
    cache_src_dir: &Path,
    lockfile: &LockFile,
) -> Result<(), ProtoError> {
    info!(
        "Copying proto files from {} descriptor...",
        lockfile.module_name
    );
    if !proto_dir.exists() {
        std::fs::create_dir_all(proto_dir)?;
    }

    let deps = collect_all_root_dependencies(lockfile);

    for dep in &deps {
        let dep_cache_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
        let sources_to_copy: HashSet<ProtoFileMapping> = if !dep.rules.prune {
            copy_all_proto_files_for_dep(&dep_cache_dir, dep)?
        } else {
            pruned_transitive_dependencies(cache_src_dir, dep, lockfile)?
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
    dep: &LockedDependency,
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
            proto_mapping.push(ProtoFileMapping::new(proto_src, proto_package_path));
        }
    }
    Ok(proto_mapping.into_iter().collect())
}

/// Returns an HashSet of ProtoFileMapping to the proto files that `dep` depends on. It recursively
/// iterates all the dependencies of `dep` and its transitive dependencies based on imports
/// until no new dependencies are found.
fn pruned_transitive_dependencies(
    cache_src_dir: &Path,
    dep: &LockedDependency,
    lockfile: &LockFile,
) -> Result<HashSet<ProtoFileMapping>, ProtoError> {
    fn process_mapping_file(
        mapping: ProtoFileCanonicalMapping,
        cache_src_dir: &Path,
        dep: &LockedDependency,
        lockfile: &LockFile,
        visited: &mut HashSet<PathBuf>,
        deps: &mut HashSet<ProtoFileCanonicalMapping>,
    ) -> Result<(), ProtoError> {
        visited.insert(mapping.package_path.clone());
        let file_deps = extract_proto_dependencies_from_file(mapping.full_path.as_path())?;
        let mut dependencies = collect_transitive_dependencies(dep, lockfile);
        dependencies.push(dep.clone());
        let mut new_mappings =
            canonical_mapping_for_proto_files(&file_deps, cache_src_dir, &dependencies)?;
        trace!("Adding {:?}.", &new_mappings);
        new_mappings.push(mapping);
        deps.extend(new_mappings.clone());
        Ok(())
    }

    /// Recursively loop through all the file dependencies based on imports
    /// Looks in own repository and in transitive dependencies.
    fn inner_loop(
        cache_src_dir: &Path,
        dep: &LockedDependency,
        lockfile: &LockFile,
        visited: &mut HashSet<PathBuf>,
        found_proto_deps: &mut HashSet<ProtoFileCanonicalMapping>,
    ) -> Result<(), ProtoError> {
        let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
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
                process_mapping_file(
                    mapping,
                    cache_src_dir,
                    dep,
                    lockfile,
                    visited,
                    found_proto_deps,
                )?;
                inner_loop(cache_src_dir, dep, lockfile, visited, found_proto_deps)?;
            }
        }
        Ok(())
    }

    let mut found_proto_deps: HashSet<ProtoFileCanonicalMapping> = HashSet::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut visited_dep: HashSet<LockedDependency> = HashSet::new();
    debug!("Extracting proto files for {}", &dep.name.value);

    let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
    for dir in dep_dir.read_dir()? {
        let proto_files = find_proto_files(&dir?.path())?;
        let filtered_mapping = filtered_proto_files(proto_files, &dep_dir, dep, true);
        trace!("Filtered size {:?}.", &filtered_mapping.len());
        for mapping in filtered_mapping {
            process_mapping_file(
                mapping,
                cache_src_dir,
                dep,
                lockfile,
                &mut visited,
                &mut found_proto_deps,
            )?;
            inner_loop(
                cache_src_dir,
                dep,
                lockfile,
                &mut visited,
                &mut found_proto_deps,
            )?;
        }
    }

    // Select proto files for the transitive dependencies of this dependency
    let t_deps: Vec<LockedDependency> = collect_transitive_dependencies(dep, lockfile);
    for t_dep in t_deps {
        trace!(
            "Extracting transitive proto dependencies from {} for dependency {} ",
            &t_dep.name.value,
            &dep.name.value
        );
        visited_dep.insert(t_dep.clone());
        inner_loop(
            cache_src_dir,
            &t_dep,
            lockfile,
            &mut visited,
            &mut found_proto_deps,
        )?;
    }
    debug!(
        "Found {:?} proto files for dependency {}",
        found_proto_deps.len(),
        dep.name.value
    );
    Ok(found_proto_deps
        .into_iter()
        .map(|p| ProtoFileMapping::new(p.full_path, p.package_path))
        .collect())
}

fn copy_proto_sources_for_dep(
    proto_dir: &Path,
    dep_cache_dir: &Path,
    dep: &LockedDependency,
    sources_to_copy: &HashSet<ProtoFileMapping>,
) -> Result<(), ProtoError> {
    debug!(
        "Copying {:?} proto files for dependency {}",
        sources_to_copy.len(),
        dep.name.value
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
    dep: &LockedDependency,
    lockfile: &LockFile,
) -> Vec<LockedDependency> {
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
fn collect_all_root_dependencies(lockfile: &LockFile) -> HashSet<LockedDependency> {
    let mut deps = HashSet::new();

    for dep in &lockfile.dependencies {
        let pruned = lockfile
            .dependencies
            .iter()
            .any(|iter_dep| iter_dep.dependencies.contains(&dep.name) && iter_dep.rules.prune);

        let non_pruned = lockfile
            .dependencies
            .iter()
            .any(|iter_dep| iter_dep.dependencies.contains(&dep.name) && !iter_dep.rules.prune);

        if (!pruned && !dep.rules.transitive) || non_pruned {
            deps.insert(dep.clone());
        }
    }
    deps
}

/// This is removing the prefix which is needed to actually load file to extract protos from imports
fn filtered_proto_files(
    proto_files: Vec<PathBuf>,
    dep_dir: &Path,
    dep: &LockedDependency,
    should_filter: bool,
) -> Vec<ProtoFileCanonicalMapping> {
    proto_files
        .into_iter()
        .filter_map(|p| {
            let path = path_strip_prefix(&p, dep_dir).ok()?;
            let zoom = zoom_in_content_root(dep, &path).ok()?;
            if AllowPolicies::should_allow_file(&dep.rules.allow_policies, &zoom) || !should_filter
            {
                Some(ProtoFileCanonicalMapping::new(p, zoom))
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
    proto_files: &[PathBuf],
    cache_src_dir: &Path,
    deps: &[LockedDependency],
) -> Result<Vec<ProtoFileCanonicalMapping>, ProtoError> {
    let r: Result<Vec<ProtoFileCanonicalMapping>, ProtoError> = proto_files
        .iter()
        .map(|p| {
            let zoom_out = zoom_out_content_root(cache_src_dir, deps, p)?;
            Ok(ProtoFileCanonicalMapping::new(zoom_out, p.to_path_buf()))
        })
        .collect::<Result<Vec<_>, _>>();
    r
}

/// Remove content_root part of path if found
fn zoom_in_content_root(
    dep: &LockedDependency,
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
    cache_src_dir: &Path,
    deps: &[LockedDependency],
    proto_file_source: &Path,
) -> Result<PathBuf, ProtoError> {
    let mut proto_src = proto_file_source.to_path_buf();
    for dep in deps {
        let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
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
mod test {
    use crate::{model::protofetch::*, proto::*};
    use std::{
        collections::{BTreeSet, HashSet},
        path::{Path, PathBuf},
    };
    use test_log::test;

    #[test]
    fn content_root_dependencies_test() {
        let cache_dir = project_root::get_project_root()
            .unwrap()
            .join(Path::new("resources/cache/dep3/hash3"));
        let lock_file = LockedDependency {
            name: DependencyName::new("dep3".to_string()),
            commit_hash: "hash3".to_string(),
            coordinate: Coordinate::default(),
            dependencies: BTreeSet::new(),
            rules: Rules::new(
                false,
                false,
                BTreeSet::from([ContentRoot::from_string("root")]),
                AllowPolicies::default(),
                DenyPolicies::default(),
            ),
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
    fn pruned_dependencies_test() {
        let cache_dir = project_root::get_project_root()
            .unwrap()
            .join(Path::new("resources/cache"));
        let lock_file = LockFile {
            module_name: "test".to_string(),
            proto_out_dir: None,
            dependencies: BTreeSet::from([
                LockedDependency {
                    name: DependencyName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::from([DependencyName::new("dep2".to_string())]),
                    rules: Rules::new(
                        true,
                        false,
                        BTreeSet::new(),
                        AllowPolicies::new(BTreeSet::from([FilePolicy::try_from_str(
                            "/proto/example.proto",
                        )
                        .unwrap()])),
                        DenyPolicies::default(),
                    ),
                },
                LockedDependency {
                    name: DependencyName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
            ]),
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
            &cache_dir,
            lock_file.dependencies.iter().next().unwrap(),
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
        let lock_file = LockFile {
            module_name: "test".to_string(),
            proto_out_dir: None,
            dependencies: BTreeSet::from([
                LockedDependency {
                    name: DependencyName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::from([
                        DependencyName::new("dep2".to_string()),
                        DependencyName::new("dep3".to_string()),
                    ]),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep3".to_string()),
                    commit_hash: "hash3".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep4".to_string()),
                    commit_hash: "hash4".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::new(
                        false,
                        true,
                        BTreeSet::new(),
                        AllowPolicies::default(),
                        DenyPolicies::default(),
                    ),
                },
            ]),
        };

        let mut it = lock_file.dependencies.iter();
        let result = collect_transitive_dependencies(it.next().unwrap(), &lock_file);
        assert_eq!(result.len(), 3);
        assert!(result.contains(it.next().unwrap()));
        assert!(result.contains(it.next().unwrap()));
        assert!(result.contains(it.next().unwrap()));
    }

    #[test]
    fn collect_all_root_dependencies_test() {
        let lock_file = LockFile {
            module_name: "test".to_string(),
            proto_out_dir: None,
            dependencies: BTreeSet::from([
                LockedDependency {
                    name: DependencyName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep3".to_string()),
                    commit_hash: "hash3".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
            ]),
        };

        let result = collect_all_root_dependencies(&lock_file);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn collect_all_root_dependencies_test_filtered() {
        let lock_file = LockFile {
            module_name: "test".to_string(),
            proto_out_dir: None,
            dependencies: BTreeSet::from([
                LockedDependency {
                    name: DependencyName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::from([DependencyName::new("dep2".to_string())]),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep3".to_string()),
                    commit_hash: "hash3".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::from([
                        DependencyName::new("dep2".to_string()),
                        DependencyName::new("dep5".to_string()),
                    ]),
                    rules: Rules {
                        prune: true,
                        transitive: false,
                        ..Default::default()
                    },
                },
                LockedDependency {
                    name: DependencyName::new("dep4".to_string()),
                    commit_hash: "hash4".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
                LockedDependency {
                    name: DependencyName::new("dep5".to_string()),
                    commit_hash: "hash5".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules {
                        prune: false,
                        transitive: true,
                        ..Default::default()
                    },
                },
            ]),
        };

        let result = collect_all_root_dependencies(&lock_file);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn generate_valid_lock_no_dep() {
        let expected = r#"module_name = "test"

[[dependencies]]
commit_hash = "hash2"

[dependencies.coordinate]
forge = ""
organization = ""
protocol = "ssh"
repository = ""

[dependencies.name]
value = "dep2"

[dependencies.rules]
prune = false
transitive = false
"#;
        let lock_file = LockFile {
            module_name: "test".to_string(),
            proto_out_dir: None,
            dependencies: BTreeSet::from([LockedDependency {
                name: DependencyName::new("dep2".to_string()),
                commit_hash: "hash2".to_string(),
                coordinate: Coordinate::default(),
                dependencies: BTreeSet::new(),
                rules: Rules::default(),
            }]),
        };
        let value_toml = toml::Value::try_from(lock_file).unwrap();
        let string_fmt = toml::to_string_pretty(&value_toml).unwrap();
        assert_eq!(string_fmt, expected);
    }

    #[test]
    fn parse_valid_lock_no_dep() {
        let lock_file = LockFile {
            module_name: "test".to_string(),
            proto_out_dir: None,
            dependencies: BTreeSet::from([LockedDependency {
                name: DependencyName::new("dep2".to_string()),
                commit_hash: "hash2".to_string(),
                coordinate: Coordinate::default(),
                dependencies: BTreeSet::new(),
                rules: Rules::default(),
            }]),
        };
        let value_toml = toml::Value::try_from(&lock_file).unwrap();
        let string_fmt = toml::to_string_pretty(&value_toml).unwrap();
        let new_lock_file = toml::from_str::<LockFile>(&string_fmt).unwrap();

        assert_eq!(new_lock_file, lock_file);
    }
}
