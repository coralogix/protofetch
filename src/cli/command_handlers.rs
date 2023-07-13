use log::info;

use crate::{
    cache::ProtofetchGitCache,
    fetch,
    model::{
        protodep::ProtodepDescriptor,
        protofetch::{Descriptor, LockFile},
    },
    proto,
};
use std::{
    error::Error,
    path::{Path, PathBuf},
};

/// Handler to fetch command
pub fn do_fetch(
    force_lock: bool,
    cache: &ProtofetchGitCache,
    root: &Path,
    module_file_name: &Path,
    lock_file_name: &Path,
    cache_dependencies_directory_name: &Path,
    default_proto_output_directory_name: &Path,
) -> Result<(), Box<dyn Error>> {
    let lock_file_path = root.join(lock_file_name);
    let lockfile = if force_lock || !lock_file_path.exists() {
        do_lock(cache, root, module_file_name, lock_file_name)?
    } else {
        LockFile::from_file(&lock_file_path)?
    };
    let cache_dependencies_directory_path = cache.location.join(cache_dependencies_directory_name);
    let proto_output_directory_name = lockfile
        .proto_out_dir
        .as_ref()
        .map(Path::new)
        .unwrap_or(default_proto_output_directory_name);
    let proto_output_directory_path = root.join(proto_output_directory_name);
    fetch::fetch_sources(cache, &lockfile, &cache_dependencies_directory_path)?;
    //Copy proto_out files to actual target
    proto::copy_proto_files(
        &proto_output_directory_path,
        &cache_dependencies_directory_path,
        &lockfile,
    )?;
    Ok(())
}

/// Handler to lock command
/// Loads dependency descriptor from protofetch toml or protodep toml
/// Generates a lock file based on the protofetch.toml
pub fn do_lock(
    cache: &ProtofetchGitCache,
    root: &Path,
    module_file_name: &Path,
    lock_file_name: &Path,
) -> Result<LockFile, Box<dyn Error>> {
    log::debug!("Generating lockfile...");
    let root = root.canonicalize()?;
    let module_file_path = root.join(module_file_name);
    let lock_file_path = root.join(lock_file_name);
    let protodep_toml_path = root.join(Path::new("protodep.toml"));

    let module_descriptor = Descriptor::from_file(module_file_path.as_path()).or_else(|_| {
        ProtodepDescriptor::from_file(protodep_toml_path.as_path())
            .and_then(|d| d.into_proto_fetch())
    })?;

    let lockfile = fetch::lock(&module_descriptor, cache)?;

    log::debug!("Generated lockfile: {:?}", lockfile);
    let value_toml = toml::Value::try_from(&lockfile)?;
    std::fs::write(&lock_file_path, toml::to_string_pretty(&value_toml)?)?;

    log::info!("Wrote lockfile to {}", lock_file_path.display());

    Ok(lockfile)
}

/// Handler to init command
pub fn do_init(
    root: &Path,
    name: Option<String>,
    module_file_name: &Path,
) -> Result<(), Box<dyn Error>> {
    let root = root.canonicalize()?;
    let name = build_module_name(name, &root)?;
    let descriptor = Descriptor::new(name, None, None, vec![]);
    let module_file_path = root.join(module_file_name);
    create_module_dir(descriptor, &module_file_path, false)
}

///Migrate from protodep to protofetch
/// 1 - Reads protodep.toml
/// 2 - Translates descriptor
/// 3 - Writes protofetch.toml
/// 4 - Deletes protodep.toml
pub fn do_migrate(
    root: &Path,
    name: Option<String>,
    module_file_name: &Path,
    source_directory_path: &Path,
) -> Result<(), Box<dyn Error>> {
    //protodep default file
    let protodep_toml_path = source_directory_path.join("protodep.toml");
    let protodep_lock_path = source_directory_path.join("protodep.lock");
    let descriptor =
        ProtodepDescriptor::from_file(&protodep_toml_path).and_then(|d| d.into_proto_fetch())?;
    let root = Path::new(root).canonicalize()?;
    let name = build_module_name(name, &root)?;
    let descriptor_with_name = Descriptor { name, ..descriptor };
    let module_file_path = root.join(module_file_name);
    create_module_dir(descriptor_with_name, &module_file_path, false)?;
    std::fs::remove_file(protodep_toml_path)?;
    std::fs::remove_file(protodep_lock_path)?;
    Ok(())
}

pub fn do_clean(
    root: &Path,
    lock_file_name: &Path,
    default_output_directory_name: &Path,
) -> Result<(), Box<dyn Error>> {
    let lock_file_path = root.join(lock_file_name);
    if lock_file_path.exists() {
        let lockfile = LockFile::from_file(&lock_file_path)?;
        let proto_out_directory_name = lockfile
            .proto_out_dir
            .as_ref()
            .map(Path::new)
            .unwrap_or(default_output_directory_name);
        let proto_out_directory_path = root.join(proto_out_directory_name);
        info!(
            "Cleaning protofetch proto_out source files folder {}.",
            proto_out_directory_path.display()
        );
        std::fs::remove_dir_all(proto_out_directory_path)?;
        std::fs::remove_file(lock_file_path)?;
        Ok(())
    } else {
        Ok(())
    }
}

pub fn do_clear_cache(cache: &ProtofetchGitCache) -> Result<(), Box<dyn Error>> {
    if cache.location.exists() {
        info!(
            "Clearing protofetch repository cache {}.",
            &cache.location.display()
        );
        std::fs::remove_dir_all(&cache.location)?;
        Ok(())
    } else {
        Ok(())
    }
}

/// Name if present otherwise attempt to extract from directory
fn build_module_name(name: Option<String>, path: &Path) -> Result<String, Box<dyn Error>> {
    match name {
        Some(name) => Ok(name),
        None => {
            let filename = path.file_name();

            match filename {
                Some(dir) => Ok(dir.to_string_lossy().to_string()),
                None => Err(
                    "Module name not given and could not convert location to directory name".into(),
                ),
            }
        }
    }
}

fn create_module_dir(
    descriptor: Descriptor,
    module_filename_path: &PathBuf,
    ow: bool,
) -> Result<(), Box<dyn Error>> {
    if !module_filename_path.exists() {
        std::fs::write(
            module_filename_path,
            toml::to_string_pretty(&descriptor.into_toml())?,
        )?;
        Ok(())
    } else if ow {
        std::fs::remove_file(module_filename_path)?;
        std::fs::write(
            module_filename_path,
            toml::to_string_pretty(&descriptor.into_toml())?,
        )?;
        Ok(())
    } else {
        Err(format!("File already exists: {}", module_filename_path.display()).into())
    }
}
