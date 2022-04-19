use std::error::Error;
use std::path::Path;
use crate::cache::ProtofetchCache;
use crate::fetch;
use crate::model::protofetch::{Descriptor, LockFile};

pub fn do_fetch(
    lock: bool,
    cache: &ProtofetchCache,
    module_path: &Path,
    lockfile_path: &Path,
    out_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    let lockfile = if lock {
        do_lock(cache, module_path, lockfile_path)?
    } else {
        // read from file
        LockFile::from_file(lockfile_path)?
    };

    fetch::fetch(cache, &lockfile, out_dir)?;

    Ok(())
}

pub fn do_lock(
    cache: &ProtofetchCache,
    module_path: &Path,
    lockfile_path: &Path,
) -> Result<LockFile, Box<dyn Error>> {
    let module_descriptor = Descriptor::from_file(module_path)?;
    let lockfile = fetch::lock(
        module_descriptor.name,
        cache,
        &module_descriptor.dependencies,
    )?;

    log::debug!("Generated lockfile: {:?}", lockfile);

    std::fs::write(lockfile_path, toml::to_string_pretty(&lockfile)?)?;

    log::info!("Wrote lockfile to {}", lockfile_path.to_string_lossy());

    Ok(lockfile)
}

pub fn do_init(
    directory: &str,
    name: Option<&str>,
    module_filename: &str,
) -> Result<(), Box<dyn Error>> {
    let canonical_path = Path::new(directory).canonicalize()?;
    let actual_name = match name {
        Some(name) => name.to_string(),
        None => {
            let filename = canonical_path.file_name();

            match filename {
                Some(dir) => dir.to_string_lossy().to_string(),
                None => {
                    return Err(
                        "Module name not given and could not convert location to directory name"
                            .into(),
                    );
                }
            }
        }
    };

    let descriptor = Descriptor::new(actual_name, None, vec![]);
    let module_filename_path = canonical_path.join(module_filename);

    if !module_filename_path.exists() {
        std::fs::write(module_filename_path, toml::to_string_pretty(&descriptor)?)?;
        Ok(())
    } else {
        Err(format!(
            "File already exists: {}",
            module_filename_path.to_string_lossy().to_string()
        )
            .into())
    }
}
