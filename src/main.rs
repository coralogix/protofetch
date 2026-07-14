use std::error::Error;

use clap::Parser;
use env_logger::Target;

use log::warn;
use protofetch::{DependencyUpdate, LockMode, LockUpdateMode, Protofetch};

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
    /// Maximum number of in-flight network jobs (resolve + fetch). Overrides
    /// PROTOFETCH_JOBS / config.toml. Defaults to 16.
    #[clap(long)]
    pub jobs: Option<usize>,
    /// Maximum number of in-flight disk jobs (worktree + copy). Overrides
    /// PROTOFETCH_COPY_JOBS / config.toml. Defaults to max(4, num_cpus / 2).
    #[clap(long)]
    pub copy_jobs: Option<usize>,
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
    Update {
        /// Dependencies to update
        #[clap(value_name = "DEP")]
        deps: Vec<String>,
        /// Update the selected dependency to this exact commit
        #[clap(long)]
        precise: Option<String>,
    },
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

            write!(buf, "{style}{}{style:#}", level)?;

            if at_least_debug_log {
                write!(
                    buf,
                    " [{}:{}]",
                    record.file().unwrap_or("unknown"),
                    record.line().unwrap_or(0),
                )?;
            }
            writeln!(buf, " {}", record.args())
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
    if let Some(jobs) = cli_args.jobs {
        if jobs == 0 {
            return Err("--jobs must be at least 1".into());
        }
        protofetch = protofetch.jobs(jobs);
    }
    if let Some(copy_jobs) = cli_args.copy_jobs {
        if copy_jobs == 0 {
            return Err("--copy-jobs must be at least 1".into());
        }
        protofetch = protofetch.copy_jobs(copy_jobs);
    }

    match cli_args.cmd {
        Command::Fetch { locked, force_lock } => {
            let lock_mode = if force_lock {
                warn!("Specifying --force-lock is deprecated, please use \"protofetch update\" instead");
                LockMode::Recreate
            } else if locked {
                LockMode::Locked
            } else {
                LockMode::Update
            };

            protofetch.try_build()?.fetch(lock_mode)
        }
        Command::Lock => protofetch.try_build()?.update(LockUpdateMode::Reconcile),
        Command::Update { deps, precise } => {
            if precise.is_some() && deps.len() != 1 {
                return Err("--precise requires exactly one DEP".into());
            }

            if deps.is_empty() {
                protofetch.try_build()?.update(LockUpdateMode::Full)
            } else {
                let updates = match precise {
                    Some(commit_hash) => vec![DependencyUpdate::Precise {
                        name: deps.into_iter().next().expect("validated one DEP"),
                        commit_hash,
                    }],
                    None => deps
                        .into_iter()
                        .map(|name| DependencyUpdate::Latest { name })
                        .collect(),
                };

                protofetch
                    .try_build()?
                    .update(LockUpdateMode::ReconcileAndUpdate(updates))
            }
        }
        Command::Init { directory, name } => protofetch.root(directory).try_build()?.init(name),
        Command::Migrate { directory, name } => protofetch
            .root(&directory)
            .try_build()?
            .migrate(name, directory),
        Command::Clean => protofetch.try_build()?.clean(),
        Command::ClearCache => protofetch.try_build()?.clear_cache(),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{CliArgs, Command};

    #[test]
    fn update_accepts_positional_specs_and_precise() {
        let args =
            CliArgs::try_parse_from(["protofetch", "update", "repo1", "--precise", "abc123"])
                .unwrap();

        match args.cmd {
            Command::Update {
                deps: specs,
                precise,
            } => {
                assert_eq!(specs, vec!["repo1"]);
                assert_eq!(precise.as_deref(), Some("abc123"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn update_accepts_multiple_positional_specs() {
        let args = CliArgs::try_parse_from(["protofetch", "update", "repo1", "repo2"]).unwrap();

        match args.cmd {
            Command::Update {
                deps: specs,
                precise,
            } => {
                assert_eq!(specs, vec!["repo1", "repo2"]);
                assert_eq!(precise, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
