use log::{debug, info};

use crate::{
    api::LockMode,
    fetch,
    git::cache::ProtofetchGitCache,
    model::{
        protodep::ProtodepDescriptor,
        protofetch::{lock::LockFile, resolved::ResolvedModule, Descriptor, ModuleName},
    },
    proto,
    resolver::LockFileModuleResolver,
};
use std::{
    error::Error,
    path::{Path, PathBuf},
};

const DEFAULT_OUTPUT_DIRECTORY_NAME: &str = "proto_src";

/// Handler to fetch command
pub fn do_fetch(
    lock_mode: LockMode,
    cache: &ProtofetchGitCache,
    root: &Path,
    module_file_name: &Path,
    lock_file_name: &Path,
    output_directory_name: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let module_descriptor = load_module_descriptor(root, module_file_name)?;

    let resolved = do_lock(lock_mode, cache, root, module_file_name, lock_file_name)?;

    let output_directory_name = output_directory_name
        .or_else(|| module_descriptor.proto_out_dir.as_ref().map(Path::new))
        .unwrap_or(Path::new(DEFAULT_OUTPUT_DIRECTORY_NAME));
    fetch::fetch_sources(cache, &resolved.dependencies)?;

    //Copy proto_out files to actual target
    proto::copy_proto_files(cache, &resolved, &root.join(output_directory_name))?;

    Ok(())
}

/// Handler to lock command
/// Loads dependency descriptor from protofetch toml or protodep toml
/// Generates a lock file based on the protofetch.toml
pub fn do_lock(
    lock_mode: LockMode,
    cache: &ProtofetchGitCache,
    root: &Path,
    module_file_name: &Path,
    lock_file_name: &Path,
) -> Result<ResolvedModule, Box<dyn Error>> {
    let module_descriptor = load_module_descriptor(root, module_file_name)?;

    let lock_file_path = root.join(lock_file_name);

    let (old_lock, (resolved, lockfile)) = match (lock_mode, lock_file_path.exists()) {
        (LockMode::Locked, false) => return Err("Lock file does not exist".into()),

        (LockMode::Locked, true) => {
            let old_lock = LockFile::from_file(&lock_file_path)?;
            let resolver = LockFileModuleResolver::new(cache, &old_lock, true);
            debug!("Verifying lockfile...");
            let resolved = fetch::resolve(&module_descriptor, &resolver)?;
            (Some(old_lock), resolved)
        }

        (LockMode::Update, false) => {
            debug!("Generating lockfile...");
            (None, fetch::resolve(&module_descriptor, &cache)?)
        }

        (LockMode::Update, true) => {
            let old_lock = LockFile::from_file(&lock_file_path)?;
            let resolver = LockFileModuleResolver::new(cache, &old_lock, false);
            debug!("Updating lockfile...");
            let resolved = fetch::resolve(&module_descriptor, &resolver)?;
            (Some(old_lock), resolved)
        }

        (LockMode::Recreate, _) => {
            debug!("Generating lockfile...");
            (None, fetch::resolve(&module_descriptor, &cache)?)
        }
    };

    debug!("Generated lockfile: {:?}", lockfile);

    if old_lock.is_some_and(|old_lock| old_lock == lockfile) {
        debug!("Lockfile is up to date");
    } else {
        let lock_file_path = root.join(lock_file_name);
        std::fs::write(&lock_file_path, lockfile.to_string()?)?;
        info!("Wrote lockfile to {}", lock_file_path.display());
    }

    Ok(resolved)
}

/// Handler to init command
pub fn do_init(
    root: &Path,
    name: Option<String>,
    module_file_name: &Path,
) -> Result<(), Box<dyn Error>> {
    let name = build_module_name(name, root)?;
    let descriptor = {
        Descriptor {
            name,
            description: None,
            proto_out_dir: None,
            dependencies: vec![],
        }
    };
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
    let descriptor = ProtodepDescriptor::from_file(&source_directory_path.join("protodep.toml"))
        .and_then(|d| d.into_proto_fetch())?;

    let name = build_module_name(name, root)?;
    let descriptor_with_name = Descriptor { name, ..descriptor };
    create_module_dir(descriptor_with_name, &root.join(module_file_name), false)?;

    std::fs::remove_file(source_directory_path.join("protodep.toml"))?;
    std::fs::remove_file(source_directory_path.join("protodep.lock"))?;

    Ok(())
}

pub fn do_clean(
    root: &Path,
    module_file_name: &Path,
    lock_file_name: &Path,
    output_directory_name: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let module_descriptor = load_module_descriptor(root, module_file_name)?;

    let lock_file_path = root.join(lock_file_name);

    let output_directory_name = output_directory_name
        .or_else(|| module_descriptor.proto_out_dir.as_ref().map(Path::new))
        .unwrap_or(Path::new(DEFAULT_OUTPUT_DIRECTORY_NAME));
    let output_directory_path = root.join(output_directory_name);

    info!(
        "Cleaning protofetch proto_out source files folder {}.",
        output_directory_path.display()
    );
    let output1 = std::fs::remove_dir_all(&output_directory_path);
    let output2 = std::fs::remove_file(&lock_file_path);

    for (output, path) in [(output1, output_directory_path), (output2, lock_file_path)] {
        match output {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                info!("{} is already removed, nothing to do", path.display());
                Ok(())
            }
            otherwise => otherwise,
        }?;
    }

    Ok(())
}

fn load_module_descriptor(
    root: &Path,
    module_file_name: &Path,
) -> Result<Descriptor, Box<dyn Error>> {
    let module_descriptor = Descriptor::from_file(&root.join(module_file_name)).or_else(|_| {
        ProtodepDescriptor::from_file(&root.join("protodep.toml"))
            .and_then(|d| d.into_proto_fetch())
    })?;

    Ok(module_descriptor)
}

/// Name if present otherwise attempt to extract from directory
fn build_module_name(name: Option<String>, path: &Path) -> Result<ModuleName, Box<dyn Error>> {
    match name {
        Some(name) => Ok(ModuleName::from(name)),
        None => match path.canonicalize()?.file_name() {
            Some(dir) => Ok(ModuleName::from(dir.to_string_lossy().to_string())),
            None => {
                Err("Module name not given and could not convert location to directory name".into())
            }
        },
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
