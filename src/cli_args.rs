use clap::{App, Arg, SubCommand};

#[derive(Debug)]
pub struct CliArgs {
    pub cmd: Cmd,
}

#[derive(Debug)]
pub enum Cmd {
    Fetch(FetchArgs),
    Lock(LockArgs),
}

#[derive(Debug)]
pub struct FetchArgs {
    pub relock: bool,
}

#[derive(Debug)]
pub struct LockArgs {
    
}

pub fn make_app() -> App<'static, 'static> {
    return App::new("protofetch")
        .version("0.0.1")
        .author("Itamar Ravid")
        .arg(Arg::with_name("verbose").short("v"))
        .subcommand(
            SubCommand::with_name("fetch")
                .about("Fetches dependencies")
                .arg(
                    Arg::with_name("relock")
                        .required(false)
                        .long("relock")
                        .takes_value(false)
                        .help("Re-lock dependencies"),
                ),
        )
        .subcommand(SubCommand::with_name("lock").about("Locks dependencies"));
}
