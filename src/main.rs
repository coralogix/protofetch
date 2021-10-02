mod cache;
mod cli_args;
mod config_file;
mod fetch;
mod model;

use std::path::{Path, PathBuf};
use std::error::Error;

use cli_args::{make_app, CliArgs, Cmd, FetchArgs, LockArgs};
use config_file::ProtofetchConfig;
use fetch::{lock, FetchError};

use model::Descriptor;

use crate::cache::ProtofetchCache;

fn run() -> Result<(), Box<dyn Error>> {
    let app_matches = make_app().get_matches();
    let cmd: Option<CliArgs> = match app_matches.subcommand() {
        ("fetch", Some(sub_m)) => {
            let should_lock = sub_m.is_present("relock");
            Some(CliArgs {
                cmd: Cmd::Fetch(FetchArgs {
                    relock: should_lock,
                }),
            })
        }
        ("lock", Some(sub_m)) => Some(CliArgs {
            cmd: Cmd::Lock(LockArgs {}),
        }),
        _ => None,
    };

    let module_descriptor = Descriptor::from_file(Path::new("module.toml"))?;
    let cache = ProtofetchCache::new(PathBuf::from("./.protofetch_cache"))?;
    let lockfile = lock(&cache, &module_descriptor.dependencies)?;

    println!("{:?}", lockfile);

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        println!("{}", e);
    }
}
