use std::error::Error;

use clap::Parser;
use env_logger::Target;

use log::warn;
use protofetch::Protofetch;

/// Dependency management tool for Protocol Buffers files.
#[derive(Debug, Parser)]
#[clap(version)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub cmd: Command,
    #[clap(short, long, default_value = "protofetch.toml")]
    /// Name of the protofetch configuration toml file
    pub module_location: String,
    #[clap(short, long, default_value = "protofetch.lock")]
    /// Name of the protofetch lock file
    pub lockfile_location: String,
    #[clap(short, long)]
    /// Location of the protofetch cache directory [default: platform-specific]
    pub cache_directory: Option<String>,
    /// Name of the output directory for proto source files,
    /// this will override proto_out_dir from the module toml config
    #[clap(short, long)]
    pub output_proto_directory: Option<String>,
    #[clap(short, long)]
    /// Git username in case https is used in config
    pub username: Option<String>,
    #[clap(short, long)]
    /// Git password in case https is used in config
    pub password: Option<String>,
}

#[derive(Debug, Parser)]
pub enum Command {
    /// Fetches protodep dependencies defined in the toml configuration file
    Fetch {
        #[clap(short, long)]
        /// forces re-creation of lock file
        force_lock: bool,
        /// name of the dependencies repo checkout directory
        /// this is a relative path within cache folder
        #[clap(short, long, hide(true))]
        repo_output_directory: Option<String>,
    },
    /// Creates a lock file based on toml configuration file
    Lock,
    /// Creates an init protofetch setup in provided directory and name
    Init {
        #[clap(default_value = ".")]
        directory: String,
        #[clap(short, long)]
        name: Option<String>,
    },
    /// Migrates a protodep toml file to a protofetch format
    Migrate {
        #[clap(default_value = ".")]
        directory: String,
        #[clap(short, long)]
        name: Option<String>,
    },
    /// Cleans generated proto sources and lock file
    Clean,
    /// Clears cached dependencies.
    /// This will remove all cached dependencies and metadata hence making the next fetch operation slower.
    ClearCache,
}

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
    let mut protofetch = Protofetch::builder()
        .module_file_name(&cli_args.module_location)
        .lock_file_name(&cli_args.lockfile_location)
        .http_credentials(cli_args.username, cli_args.password);

    if let Some(output_directory_name) = &cli_args.output_proto_directory {
        protofetch = protofetch.output_directory_name(output_directory_name)
    }
    if let Some(cache_directory) = &cli_args.cache_directory {
        protofetch = protofetch.cache_directory(cache_directory);
    }

    match cli_args.cmd {
        Command::Fetch {
            force_lock,
            repo_output_directory,
        } => {
            #[allow(deprecated)]
            if let Some(repo_output_directory) = repo_output_directory {
                warn!("Specifying --repo-output-directory is deprecated, if you need it please open a GitHub issue describing your use-case.");
                protofetch = protofetch.cache_dependencies_directory_name(repo_output_directory);
            }

            protofetch.try_build()?.fetch(force_lock)
        }
        Command::Lock => protofetch.try_build()?.lock(),
        Command::Init { directory, name } => protofetch.root(directory).try_build()?.init(name),
        Command::Migrate { directory, name } => protofetch
            .root(&directory)
            .try_build()?
            .migrate(name, directory),
        Command::Clean => protofetch.try_build()?.clean(),
        Command::ClearCache => protofetch.try_build()?.clear_cache(),
    }
}
