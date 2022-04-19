use clap::Parser;

#[derive(Debug, Parser)]
#[clap(version = "0.0.1")]
pub struct CliArgs {
    #[clap(subcommand)]
    pub cmd: Command,
    #[clap(short, long, default_value = "module.toml")]
    pub module_location: String,
    #[clap(short, long, default_value = "module.lock")]
    pub lockfile_location: String,
    #[clap(short, long, default_value = "proto_src")]
    pub source_directory: String,
    #[clap(short, long, default_value = ".protofetch_cache")]
    pub cache_directory: String,
}

#[derive(Debug, Parser)]
pub enum Command {
    Fetch {
        #[clap(short, long)]
        lock: bool,
    },
    Lock,
    Init {
        #[clap(default_value = ".")]
        directory: String,
        #[clap(short, long)]
        name: Option<String>,
    },
}
