use crate::{
    cache::ProtofetchCache,
    fetch,
    model::{
        protodep::ProtodepDescriptor,
        protofetch::{Descriptor, LockFile},
    },
};
use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
};

/// Handler to fetch command
pub fn do_fetch(
    lock: bool,
    cache: &ProtofetchCache,
    conf_path: &Path,
    lockfile_path: &Path,
    out_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    let lockfile = if lock {
        do_lock(cache, conf_path, lockfile_path)?
    } else {
        // read from file
        LockFile::from_file(lockfile_path)?
    };

    fetch::fetch(cache, &lockfile, out_dir)?;

    Ok(())
}

/// Handler to lock command
/// Loads dependency descriptor from protofetch toml or protodep toml
/// Generates a lock file based on the protofetch.toml
pub fn do_lock(
    cache: &ProtofetchCache,
    conf_path: &Path,
    lockfile_path: &Path,
) -> Result<LockFile, Box<dyn Error>> {
    let dir = env::current_dir()?.canonicalize()?;
    let conf_path = dir.join(conf_path);
    let protodep_toml_path = dir.join(Path::new("protodep.toml"));

    let module_descriptor = Descriptor::from_file(conf_path.as_path()).or_else(|_| {
        ProtodepDescriptor::from_file(protodep_toml_path.as_path()).and_then(|d| d.to_proto_fetch())
    })?;

    let lockfile = fetch::lock(&module_descriptor, cache)?;

    log::debug!("Generated lockfile: {:?}", lockfile);

    std::fs::write(lockfile_path, toml::to_string_pretty(&lockfile)?)?;

    log::info!("Wrote lockfile to {}", lockfile_path.to_string_lossy());

    Ok(lockfile)
}

/// Handler to init command
pub fn do_init(
    directory: &str,
    name: Option<&str>,
    module_filename: &str,
) -> Result<(), Box<dyn Error>> {
    let canonical_path = Path::new(directory).canonicalize()?;
    let actual_name = build_module_name(name, &canonical_path)?;
    let descriptor = Descriptor::new(actual_name, None, None, vec![]);
    let module_filename_path = canonical_path.join(module_filename);
    create_module_dir(descriptor, &module_filename_path, false)
}

///Migrate from protodep to protofetch
/// 1 - Reads protodep.toml
/// 2 - Translates descriptor
/// 3 - Writes protofetch.toml
/// 4 - Deletes protodep.toml
pub fn do_migrate(
    directory: &str,
    name: Option<&str>,
    module_filename: &str,
) -> Result<(), Box<dyn Error>> {
    //protodep default file
    let protodep_toml_path = Path::new("./protodep.toml");
    let protodep_lock_path = Path::new("./protodep.lock");
    let descriptor =
        ProtodepDescriptor::from_file(protodep_toml_path).and_then(|d| d.to_proto_fetch())?;
    let canonical_path = Path::new(directory).canonicalize()?;
    let actual_name = build_module_name(name, &canonical_path)
        .expect("Expected a way to build a valid module name");
    let descriptor_with_name = Descriptor {
        name: actual_name,
        ..descriptor
    };
    let module_filename_path = canonical_path.join(module_filename);
    create_module_dir(descriptor_with_name, &module_filename_path, false)?;
    std::fs::remove_file(protodep_toml_path)?;
    std::fs::remove_file(protodep_lock_path)?;
    Ok(())
}

/// Name if present otherwise attempt to extract from directory
fn build_module_name(name: Option<&str>, path: &Path) -> Result<String, Box<dyn Error>> {
    match name {
        Some(name) => Ok(name.to_string()),
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
            toml::to_string_pretty(&descriptor.to_toml())?,
        )?;
        Ok(())
    } else if ow {
        std::fs::remove_file(module_filename_path)?;
        std::fs::write(
            module_filename_path,
            toml::to_string_pretty(&descriptor.to_toml())?,
        )?;
        Ok(())
    } else {
        Err(format!(
            "File already exists: {}",
            module_filename_path.to_string_lossy()
        )
        .into())
    }
}
