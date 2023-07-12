use std::error::Error;

use clap::Parser;
use env_logger::Target;

use protofetch::{cli, cli::args::CliArgs, Protofetch};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(Target::Stdout)
        .init();

    if let Err(e) = run() {
        log::error!("{}", e)
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli_args: CliArgs = CliArgs::parse();

    #[allow(deprecated)]
    let protofetch = Protofetch::builder()
        .module_file_name(&cli_args.module_location)
        .lock_file_name(&cli_args.lockfile_location)
        .cache_directory(&cli_args.cache_directory)
        .default_output_directory_name(&cli_args.output_proto_directory)
        .http_credentials(cli_args.username, cli_args.password);

    match cli_args.cmd {
        cli::args::Command::Fetch {
            force_lock,
            repo_output_directory,
        } =>
        {
            #[allow(deprecated)]
            protofetch
                .cache_dependencies_directory_name(repo_output_directory)
                .try_build()?
                .fetch(force_lock)
        }
        cli::args::Command::Lock => protofetch.try_build()?.lock(),
        cli::args::Command::Init { directory, name } => {
            protofetch.root(directory).try_build()?.init(name)
        }
        cli::args::Command::Migrate { directory, name } => {
            protofetch.root(directory).try_build()?.migrate(name)
        }
        cli::args::Command::Clean => protofetch.try_build()?.clean(),
        cli::args::Command::ClearCache => protofetch.try_build()?.clear_cache(),
    }
}
