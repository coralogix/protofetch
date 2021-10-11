use clap::{AppSettings, Clap};

#[derive(Debug, Clap)]
#[clap(version = "0.0.1")]
#[clap(setting = AppSettings::ColoredHelp)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Clap)]
pub enum Command {
    Fetch {
        #[clap(short, long)]
        lock: bool,
    },
    Lock,
}
