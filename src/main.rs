use std::error::Error;

use clap::Parser;
use env_logger::Target;

use log::warn;
use protofetch::{LockMode, Protofetch};

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
}

#[derive(Debug, Parser)]
pub enum Command {
    /// Fetches protodep dependencies defined in the toml configuration file
    Fetch {
        /// reqiure dependencies to match the lock file
        #[clap(long)]
        locked: bool,
        /// forces re-creation of lock file
        #[clap(short, long, hide(true))]
        force_lock: bool,
    },
    /// Creates a lock file based on toml configuration file
    Lock,
    /// Updates the lock file
    Update,
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
        .format(move |buf, record| {
            use std::io::Write;

            let at_least_debug_log = log::log_enabled!(log::Level::Debug);
            let level = record.level();
            let style = buf.default_level_style(level);

            if at_least_debug_log {
                writeln!(
                    buf,
                    "{} [{}:{}] {}",
                    style.value(level),
                    record.file().unwrap_or("unknown"),
                    record.line().unwrap_or(0),
                    record.args()
                )
            } else {
                writeln!(buf, "{} {}", style.value(level), record.args())
            }
        })
        .init();

    if let Err(e) = run() {
        log::error!("{}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli_args: CliArgs = CliArgs::parse();

    let mut protofetch = Protofetch::builder()
        .module_file_name(&cli_args.module_location)
        .lock_file_name(&cli_args.lockfile_location);

    if let Some(output_directory_name) = &cli_args.output_proto_directory {
        protofetch = protofetch.output_directory_name(output_directory_name)
    }
    if let Some(cache_directory) = &cli_args.cache_directory {
        protofetch = protofetch.cache_directory(cache_directory);
    }

    match cli_args.cmd {
        Command::Fetch { locked, force_lock } => {
            let lock_mode = if force_lock {
                warn!("Specifying --force-lock is deprecated, please use \"protofetch update\" instead.");
                LockMode::Recreate
            } else if locked {
                LockMode::Locked
            } else {
                LockMode::Update
            };

            protofetch.try_build()?.fetch(lock_mode)
        }
        Command::Lock => protofetch.try_build()?.lock(LockMode::Update),
        Command::Update => protofetch.try_build()?.lock(LockMode::Recreate),
        Command::Init { directory, name } => protofetch.root(directory).try_build()?.init(name),
        Command::Migrate { directory, name } => protofetch
            .root(&directory)
            .try_build()?
            .migrate(name, directory),
        Command::Clean => protofetch.try_build()?.clean(),
        Command::ClearCache => protofetch.try_build()?.clear_cache(),
    }
}
