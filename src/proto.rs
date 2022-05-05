use crate::model::protofetch::{LockFile, LockedDependency};
use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtoError {
    #[error("Bad proto_out path. {0}")]
    BadPath(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

/// proto_out_dir: Base path to the directory where the proto_out files are to be copied to
/// cache_src_dir: Base path to the directory where the dependencies sources are cached
/// lockfile: The lockfile that contains the dependencies to be copied
pub fn copy_proto_files(
    proto_out_dir: &Path,
    cache_src_dir: &Path,
    lockfile: &LockFile,
) -> Result<(), ProtoError> {
    info!(
        "Copying proto_out files from {} descriptor...",
        lockfile.module_name
    );
    if !proto_out_dir.exists() {
        std::fs::create_dir_all(proto_out_dir)?;
    }

    for dep in &lockfile.dependencies {
        debug!("Copying proto_out files for dependency {}", dep.name.value);
        let dep_cache_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
        if dep.rules.prune {
            copy_strict_pruned_dependencies(proto_out_dir, &dep_cache_dir, dep, lockfile)?;
        } else {
            copy_all_proto_files_for_dep(proto_out_dir, &dep_cache_dir, dep)?;
        }
    }
    Ok(())
}

fn copy_all_proto_files_for_dep(
    proto_out_dir: &Path,
    dep_cache_dir: &Path,
    dep: &LockedDependency,
) -> Result<(), ProtoError> {
    for file in dep_cache_dir.read_dir()? {
        let path = file?.path();
        let proto_files = find_proto_files(path.as_path())?;
        for proto_file_source in proto_files {
            trace!(
                "Copying proto_out file {}",
                &proto_file_source.to_string_lossy()
            );
            let proto_src = path_strip_prefix(&proto_file_source, &dep_cache_dir)?;

            if !dep.rules.content_roots.is_empty() {
                let root = dep
                    .rules
                    .content_roots
                    .iter()
                    .find(|c_root| proto_src.starts_with(Path::new(c_root)));
                if let Some(root) = root {
                    let proto_src = path_strip_prefix(&proto_src, Path::new(&root))?;
                    let proto_out_dist = proto_out_dir.join(&proto_src);
                    let prefix = proto_out_dist.parent().ok_or_else(|| {
                        ProtoError::BadPath(format!(
                            "Bad parent dest file for {}",
                            &proto_out_dist.to_string_lossy()
                        ))
                    })?;
                    std::fs::create_dir_all(prefix)?;
                    std::fs::copy(proto_file_source.as_path(), proto_out_dist.as_path())?;
                }
            } else {
                let proto_out_dist = proto_out_dir.join(&proto_src);
                let prefix = proto_out_dist.parent().ok_or_else(|| {
                    ProtoError::BadPath(format!(
                        "Bad parent dest file for {}",
                        &proto_out_dist.to_string_lossy()
                    ))
                })?;
                std::fs::create_dir_all(prefix)?;
                std::fs::copy(proto_file_source.as_path(), proto_out_dist.as_path())?;
            }
        }
    }
    Ok(())
}

fn copy_strict_pruned_dependencies(
    proto_out_dir: &Path,
    cache_src_dir: &Path,
    dep: &LockedDependency,
    lockfile: &LockFile,
) -> Result<(), ProtoError> {
    let pruned_dep: HashSet<PathBuf> =
        pruned_transitive_dependencies(cache_src_dir, dep, lockfile)?;

    debug!("Copying proto_out files for dependency {}", dep.name.value);
    let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
    for path in pruned_dep {
        trace!("Copying proto_out file {}", &path.to_string_lossy());
        let proto_file_out = proto_out_dir.join(&path);
        let proto_file_source = dep_dir.join(&path);
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

/// Returns an HashSet of paths to the proto_out files that `dep` depends on. It recursively
/// iterates all the dependencies of `dep` based on imports until no new dependencies are found.
fn pruned_transitive_dependencies(
    cache_src_dir: &Path,
    dep: &LockedDependency,
    lockfile: &LockFile,
) -> Result<HashSet<PathBuf>, ProtoError> {
    fn go(
        cache_src_dir: &Path,
        dep: &LockedDependency,
        lockfile: &LockFile,
        visited: &mut HashSet<PathBuf>,
        deps: &mut HashSet<PathBuf>,
    ) -> Result<(), ProtoError> {
        trace!("Cycle started");
        let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
        for dir in dep_dir.read_dir()? {
            let proto_files = find_proto_files(&dir?.path())?
                .iter()
                .map(|p| path_strip_prefix(p, &dep_dir))
                .collect::<Result<HashSet<_>, _>>()?;

            let deps_clone = deps.clone();
            trace!("Deps has {:?} and found {:?}.", deps, proto_files);
            let intersected : HashSet<PathBuf>  = deps_clone.intersection(&proto_files).cloned().collect();
            let intersected : HashSet<PathBuf> = intersected.difference(visited).cloned().collect();
            trace!("Intersected {:?}.",&intersected);
            for proto_file_source in intersected {
                visited.insert(proto_file_source.to_path_buf());
                let file_deps = extract_proto_dependencies_from_file(&dep_dir.join(proto_file_source))?;
                trace!("Adding {:?}.",&file_deps);
                deps.extend(file_deps.clone());
                go(cache_src_dir, dep, lockfile, visited,deps)?;
            }
        }
        let t_deps = collect_transitive_dependencies(dep, lockfile);
        for dep in t_deps {
            go(cache_src_dir, &dep, lockfile, visited, deps)?;
        }
        Ok(())
    }

    let mut deps: HashSet<PathBuf> = HashSet::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();

    trace!(
        "Extracting transitive proto_out dependencies {}",
        &dep.name.value
    );

    let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
    for dir in dep_dir.read_dir()? {
        let proto_files = find_proto_files(&dir?.path())?;
        for proto_file_source in proto_files {
            visited.insert(proto_file_source.clone());
            let file_deps = extract_proto_dependencies_from_file(proto_file_source.as_path())?;
            deps.extend(file_deps.clone());
        }
    }
    let t_deps: Vec<LockedDependency> = collect_transitive_dependencies(dep, lockfile);
    for dep in t_deps {
        go(cache_src_dir, &dep, lockfile, &mut visited, &mut deps)?;
    }
    Ok(deps)
}

/// Extracts the dependencies from a proto_out file
fn extract_proto_dependencies_from_file(file: &Path) -> Result<Vec<PathBuf>, ProtoError> {
    let mut dependencies = Vec::new();
    let mut reader = BufReader::new(File::open(file)?);
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if line.starts_with("import ") {
            if let Some(dependency) = line.split_whitespace().nth(1) {
                let dependency = dependency.to_string().replace(';', "").replace('\"', "");
                dependencies.push(PathBuf::from(dependency));
            }
        }
        line.clear();
    }
    Ok(dependencies)
}

/// Find proto_out files in a directory
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
                if extension == "proto_out" {
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
        .filter(|x| dep.dependencies.contains(&x.name))
        .collect::<Vec<_>>()
}

fn path_strip_prefix(path: &Path, prefix: &Path) -> Result<PathBuf, ProtoError> {
    path.strip_prefix(prefix)
        .map_err(|_err| {
            {
                ProtoError::BadPath(format!(
                    "Could not create proto_out source file path in {}. Wrong base dir {}",
                    path.to_string_lossy(),
                    prefix.to_string_lossy()
                ))
            }
        })
        .map(|s| s.to_path_buf())
}

use crate::model::protofetch::{Coordinate, DependencyName, Rules};
use git2::SubmoduleUpdate::Default;
use test_log::test;

#[test]
fn pruned_dependencies_test() {
    let cache_dir = project_root::get_project_root()
        .unwrap()
        .join(Path::new("resources/cache"));
    let lock_file = LockFile {
        module_name: "test".to_string(),
        proto_out_dir: None,
        dependencies: vec![
            LockedDependency {
                name: DependencyName::new("dep1".to_string()),
                commit_hash: "hash1".to_string(),
                coordinate: Coordinate::default(),
                dependencies: vec![DependencyName::new("dep2".to_string())],
                rules: Rules::new(true, vec![]),
            },
            LockedDependency {
                name: DependencyName::new("dep2".to_string()),
                commit_hash: "hash2".to_string(),
                coordinate: Coordinate::default(),
                dependencies: vec![],
                rules: Rules::new(true, vec![]),
            },
        ],
    };
    let expected_dep_1: HashSet<PathBuf> = vec![
        PathBuf::from("proto_out/example2.proto_out"),
        PathBuf::from("proto_out/example3.proto_out"),
        PathBuf::from("scalapb/scalapb.proto_out"),
        PathBuf::from("google/protobuf/descriptor.proto_out"),
        PathBuf::from("google/protobuf/struct.proto_out"),
    ]
    .into_iter()
    .collect();
    let expected_dep_2: HashSet<PathBuf> = vec![
        PathBuf::from("proto_out/example3.proto_out"),
        PathBuf::from("scalapb/scalapb.proto_out"),
        PathBuf::from("google/protobuf/descriptor.proto_out"),
        PathBuf::from("google/protobuf/struct.proto_out"),
    ]
    .into_iter()
    .collect();

    let pruned1 =
        pruned_transitive_dependencies(&cache_dir, &lock_file.dependencies[0], &lock_file).unwrap();
    let pruned2 =
        pruned_transitive_dependencies(&cache_dir, &lock_file.dependencies[1], &lock_file).unwrap();

    assert_eq!(pruned1, expected_dep_1);
    assert_eq!(pruned2, expected_dep_2);
}

#[test]
fn extract_dependencies_test() {
    let path = project_root::get_project_root()
        .unwrap()
        .join(Path::new("resources/proto/example2.proto"));
    let dependencies = extract_proto_dependencies_from_file(&path).unwrap();
    assert_eq!(dependencies.len(), 3);
    assert_eq!(dependencies[0].to_string_lossy(), "scalapb/scalapb.proto_out");
    assert_eq!(
        dependencies[1].to_string_lossy(),
        "google/protobuf/descriptor.proto_out"
    );
    assert_eq!(
        dependencies[2].to_string_lossy(),
        "google/protobuf/struct.proto_out"
    );
}

#[test]
fn collect_transitive_dependencies_test() {
    let lock_file = LockFile {
        module_name: "test".to_string(),
        proto_out_dir: None,
        dependencies: vec![
            LockedDependency {
                name: DependencyName::new("dep1".to_string()),
                commit_hash: "hash1".to_string(),
                coordinate: Coordinate::default(),
                dependencies: vec![
                    DependencyName::new("dep2".to_string()),
                    DependencyName::new("dep3".to_string()),
                ],
                rules: Rules::new(true, vec![]),
            },
            LockedDependency {
                name: DependencyName::new("dep2".to_string()),
                commit_hash: "hash2".to_string(),
                coordinate: Coordinate::default(),
                dependencies: vec![],
                rules: Rules::new(true, vec![]),
            },
            LockedDependency {
                name: DependencyName::new("dep3".to_string()),
                commit_hash: "hash3".to_string(),
                coordinate: Coordinate::default(),
                dependencies: vec![],
                rules: Rules::new(true, vec![]),
            },
        ],
    };

    let result = collect_transitive_dependencies(&lock_file.dependencies[0], &lock_file);
    assert_eq!(result.len(), 2);
    assert!(result.contains(&lock_file.dependencies[1]));
    assert!(result.contains(&lock_file.dependencies[2]));
}
