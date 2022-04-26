use std::{
    error::Error,
    path::{Path, PathBuf},
};

use clap::Parser;

use protofetch::{
    cache::ProtofetchCache,
    cli,
    cli::{args::CliArgs, command_handlers},
};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Err(e) = run() {
        log::error!("{}", e)
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli_args: CliArgs = CliArgs::parse();

    let cache = ProtofetchCache::new(PathBuf::from(&cli_args.cache_directory))?;
    let module_path = Path::new(&cli_args.module_location);
    let lockfile_path = Path::new(&cli_args.lockfile_location);
    let proto_output_directory = Path::new(&cli_args.proto_output_directory);

    match cli_args.cmd {
        cli::args::Command::Fetch {
            force_lock,
            source_output_directory,
        } => {
            let dependencies_out_dir = Path::new(&source_output_directory);
            let proto_output_directory = Path::new(&proto_output_directory);

            command_handlers::do_fetch(
                force_lock,
                &cache,
                module_path,
                lockfile_path,
                dependencies_out_dir,
                proto_output_directory,
            )
        }
        cli::args::Command::Lock => {
            command_handlers::do_lock(&cache, module_path, lockfile_path)?;
            Ok(())
        }
        cli::args::Command::Init { directory, name } => {
            command_handlers::do_init(&directory, name.as_deref(), &cli_args.module_location)
        }
        cli::args::Command::Migrate { directory, name } => {
            command_handlers::do_migrate(&directory, name.as_deref(), &cli_args.module_location)
        }
        cli::args::Command::Clean => {
            command_handlers::do_clean(lockfile_path, proto_output_directory)
        }
    }
}
