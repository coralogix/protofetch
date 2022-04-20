use clap::Parser;

/// Dependency management tool for Protocol Buffers files.
#[derive(Debug, Parser)]
#[clap(version = "0.0.1")]
pub struct CliArgs {
    #[clap(subcommand)]
    pub cmd: Command,
    #[clap(short, long, default_value = "protofetch.toml")]
    pub module_location: String,
    #[clap(short, long, default_value = "protofetch.lock")]
    pub lockfile_location: String,
    #[clap(short, long, default_value = "proto_src")]
    pub source_directory: String,
    #[clap(short, long, default_value = ".protofetch_cache")]
    pub cache_directory: String,
}

#[derive(Debug, Parser)]
pub enum Command {
    ///Fetches protodep dependencies defined in the toml configuration file
    Fetch {
        #[clap(short, long)]
        lock: bool,
    },
    ///Creates a lock file based on toml configuration file
    Lock,
    ///Creates an init protofetch setup in provided directory and name
    Init {
        #[clap(default_value = ".")]
        directory: String,
        #[clap(short, long)]
        name: Option<String>,
    },
    ///Migrates a protodep toml file to a protofetch format
    Migrate {
        #[clap(default_value = ".")]
        directory: String,
        #[clap(short, long)]
        name: Option<String>,
    },
}
