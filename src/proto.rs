use crate::model::protofetch::{Coordinate, DependencyName, LockFile, LockedDependency};
use std::{
    collections::{HashMap, HashSet},
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

/// proto_out_dir: Base path to the directory where the proto files are to be copied to
/// cache_src_dir: Base path to the directory where the dependencies sources are cached
/// lockfile: The lockfile that contains the dependencies to be copied
pub fn copy_proto_files(
    proto_out_dir: &Path,
    cache_src_dir: &Path,
    lockfile: &LockFile,
) -> Result<(), ProtoError> {
    info!(
        "Copying proto files from {} descriptor...",
        lockfile.module_name
    );
    if !proto_out_dir.exists() {
        std::fs::create_dir_all(proto_out_dir)?;
    }

    for dep in &lockfile.dependencies {
        debug!("Copying proto files for dependency {}", dep.name.value);
        let dep_cache_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
        copy_all_proto_files_for_dep(proto_out_dir, &dep_cache_dir, dep)?;
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
                "Copying proto file {}",
                &proto_file_source.to_string_lossy()
            );
            let proto_src = proto_file_source
                .strip_prefix(&dep_cache_dir)
                .map_err(|_err| {
                    ProtoError::BadPath(format!(
                        "Could not create proto source file path in {}. Wrong base dir {}",
                        proto_file_source.to_string_lossy(),
                        dep_cache_dir.to_string_lossy()
                    ))
                })?;

            if !dep.rules.content_roots.is_empty() {
                let root = dep
                    .rules
                    .content_roots
                    .iter()
                    .find(|c_root| proto_src.starts_with(Path::new(c_root)));
                if let Some(root) = root {
                    let proto_src = proto_src.strip_prefix(Path::new(&root)).unwrap();
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
    lockfile: &LockFile,
) -> Result<(), ProtoError> {
    let pruned_dep: HashMap<DependencyName, HashSet<PathBuf>> =
        pruned_dependencies(cache_src_dir, lockfile)?;

    for dep in &lockfile.dependencies {
        if let Some(dep_paths) = pruned_dep.get(&dep.name) {
            debug!("Copying proto files for dependency {}", dep.name.value);
            let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
            for path in dep_paths {
                trace!("Copying proto file {}", &path.to_string_lossy());
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
        }
    }
    Ok(())
}

fn pruned_dependencies(
    cache_src_dir: &Path,
    lockfile: &LockFile,
) -> Result<HashMap<DependencyName, HashSet<PathBuf>>, ProtoError> {
    let mut pruned_deps: HashMap<DependencyName, Vec<PathBuf>> = HashMap::new();
    let extracted_deps = extract_dependencies(cache_src_dir, lockfile)?;
    for dep in &lockfile.dependencies {
        trace!("Pruning dependencies for {}", &dep.name.value);
        let mut to_prune: Vec<PathBuf> = Vec::new();
        for t_dep in &dep.dependencies {
            to_prune.append(&mut extracted_deps.get(t_dep).unwrap_or(&vec![]).clone());
        }
        to_prune.append(&mut extracted_deps.get(&dep.name).unwrap_or(&vec![]).clone());
        pruned_deps
            .entry(dep.name.clone())
            .or_insert_with(|| to_prune.clone());
    }
    let mut pruned_deps_dedup: HashMap<DependencyName, HashSet<PathBuf>> = HashMap::new();
    for (dep_name, pruned_dep) in pruned_deps {
        pruned_deps_dedup.insert(dep_name, pruned_dep.into_iter().collect());
    }
    Ok(pruned_deps_dedup)
}

fn extract_dependencies(
    cache_src_dir: &Path,
    lockfile: &LockFile,
) -> Result<HashMap<DependencyName, Vec<PathBuf>>, ProtoError> {
    let mut deps: HashMap<DependencyName, Vec<PathBuf>> = HashMap::new();
    for dep in &lockfile.dependencies {
        trace!("Extracting proto dependencies {}", &dep.name.value);
        let dep_dir = cache_src_dir.join(&dep.name.value).join(&dep.commit_hash);
        for file in dep_dir.read_dir()? {
            let proto_files = find_proto_files(&file?.path())?;
            for proto_file_source in proto_files {
                let file_deps = extract_dependencies_for_file(proto_file_source.as_path())?;
                //TODO:efficient way of deduplicating vec
                deps.entry(dep.name.clone())
                    .and_modify(|deps| deps.append(&mut file_deps.clone()))
                    .or_insert_with(|| file_deps);
            }
        }
    }
    Ok(deps)
}

/// Extracts the dependencies from a proto file
fn extract_dependencies_for_file(file: &Path) -> Result<Vec<PathBuf>, ProtoError> {
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
                name: DependencyName {
                    value: "dep1".to_string(),
                },
                commit_hash: "hash1".to_string(),
                coordinate: Coordinate::default(),
                dependencies: vec![DependencyName {
                    value: "dep2".to_string(),
                }],
                rules: Default::default(),
            },
            LockedDependency {
                name: DependencyName {
                    value: "dep2".to_string(),
                },
                commit_hash: "hash2".to_string(),
                coordinate: Coordinate::default(),
                dependencies: vec![],
                rules: Default::default(),
            },
        ],
    };
    let mut expected: HashMap<DependencyName, HashSet<PathBuf>> = HashMap::new();
    expected.insert(
        DependencyName::new("dep1".to_string()),
        vec![
            PathBuf::from("proto/example2.proto"),
            PathBuf::from("scalapb/scalapb.proto"),
            PathBuf::from("google/protobuf/descriptor.proto"),
            PathBuf::from("google/protobuf/struct.proto"),
        ]
        .into_iter()
        .collect(),
    );
    expected.insert(
        DependencyName::new("dep2".to_string()),
        vec![
            PathBuf::from("scalapb/scalapb.proto"),
            PathBuf::from("google/protobuf/descriptor.proto"),
            PathBuf::from("google/protobuf/struct.proto"),
        ]
        .into_iter()
        .collect(),
    );
    let pruned = pruned_dependencies(&cache_dir, &lock_file).unwrap();
    assert_eq!(pruned, expected);
}

#[test]
fn extract_dependencies_test() {
    let path = project_root::get_project_root()
        .unwrap()
        .join(Path::new("resources/proto/example2.proto"));
    let dependencies = extract_dependencies_for_file(&path).unwrap();
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
