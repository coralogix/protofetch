mod cache;
mod cli_args;
mod fetch;
mod model;
mod proto_repository;

use std::error::Error;
use std::path::{Path, PathBuf};

use clap::Clap;
use cli_args::CliArgs;
use fetch::{fetch, lock};

use model::{Descriptor, LockFile};

use crate::cache::ProtofetchCache;

fn run() -> Result<(), Box<dyn Error>> {
    let cli_args: CliArgs = CliArgs::parse();

    let cache = ProtofetchCache::new(PathBuf::from(&cli_args.cache_directory))?;
    let out_dir = Path::new(&cli_args.source_directory);
    let module_path = Path::new(&cli_args.module_location);
    let lockfile_path = Path::new(&cli_args.lockfile_location);

    match cli_args.cmd {
        cli_args::Command::Fetch { lock } => {
            do_fetch(lock, &cache, module_path, lockfile_path, out_dir)
        }
        cli_args::Command::Lock => {
            do_lock(&cache, module_path, lockfile_path)?;

            Ok(())
        }
    }
}

fn do_fetch(
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

    fetch(cache, &lockfile, out_dir)?;

    Ok(())
}

fn do_lock(
    cache: &ProtofetchCache,
    module_path: &Path,
    lockfile_path: &Path,
) -> Result<LockFile, Box<dyn Error>> {
    let module_descriptor = Descriptor::from_file(module_path)?;
    let lockfile = lock(
        module_descriptor.name,
        cache,
        &module_descriptor.dependencies,
    )?;

    log::debug!("Generated lockfile: {:?}", lockfile);

    std::fs::write(lockfile_path, toml::to_string_pretty(&lockfile)?)?;

    log::info!("Wrote lockfile to {}", lockfile_path.to_string_lossy());

    Ok(lockfile)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Err(e) = run() {
        log::error!("{}", e)
    }
}
