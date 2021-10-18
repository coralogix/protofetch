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

    let cache = ProtofetchCache::new(PathBuf::from("./.protofetch_cache"))?;
    let out_dir = Path::new("./proto_src");

    match cli_args.cmd {
        cli_args::Command::Fetch { lock } => do_fetch(lock, &cache, out_dir),
        cli_args::Command::Lock => do_lock(&cache).map(|_| ()),
    }
}

fn do_fetch(lock: bool, cache: &ProtofetchCache, out_dir: &Path) -> Result<(), Box<dyn Error>> {
    let lockfile = if lock {
        do_lock(cache)?
    } else {
        // read from file
        LockFile::from_file(Path::new("protofetch.lock"))?
    };

    fetch(cache, &lockfile, out_dir)?;

    Ok(())
}

fn do_lock(cache: &ProtofetchCache) -> Result<LockFile, Box<dyn Error>> {
    let module_descriptor = Descriptor::from_file(Path::new("module.toml"))?;
    let lockfile = lock(
        module_descriptor.name,
        cache,
        &module_descriptor.dependencies,
    )?;

    log::info!("Generated lockfile: {:?}", lockfile);

    Ok(lockfile)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Err(e) = run() {
        log::error!("{}", e)
    }
}
