use clap::{AppSettings, Clap};

#[derive(Debug, Clap)]
#[clap(version = "0.0.1")]
#[clap(setting = AppSettings::ColoredHelp)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub cmd: Command,
    #[clap(short, long, default_value = "./module.toml")]
    pub module_location: String,
    #[clap(short, long, default_value = "./module.lock")]
    pub lockfile_location: String,
    #[clap(short, long, default_value = "./proto_src")]
    pub source_directory: String,
    #[clap(short, long, default_value = "./.protofetch_cache")]
    pub cache_directory: String,
}

#[derive(Debug, Clap)]
pub enum Command {
    Fetch {
        #[clap(short, long)]
        lock: bool,
    },
    Lock,
}
