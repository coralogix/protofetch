use clap::Parser;

/// Dependency management tool for Protocol Buffers files.
#[derive(Debug, Parser)]
#[clap(version)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub cmd: Command,
    #[clap(short, long, default_value = "protofetch.toml")]
    /// location of the protofetch configuration toml
    pub module_location: String,
    #[clap(short, long, default_value = "protofetch.lock")]
    /// location of the protofetch lock file
    pub lockfile_location: String,
    #[clap(short, long, default_value = ".protofetch/cache")]
    /// location of the protofetch cache directory
    /// relative path to $HOME directory
    pub cache_directory: String,
    /// name of the output directory for proto_out source files,
    /// this will be used if parameter proto_out_dir is not present in the module toml config
    #[clap(short, long, default_value = "proto_src")]
    pub output_proto_directory: String,
    #[clap(short, long)]
    /// git username in case https is used in config
    pub username: Option<String>,
    #[clap(short, long)]
    /// git password in case https is used in config
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
        #[clap(short, long, default_value = "dependencies")]
        repo_output_directory: String,
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
    /// Cleans generated proto_out sources and lock file
    Clean,
}
