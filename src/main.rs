mod cli_args;
mod fetch;
mod config_file;
mod cache;
mod model;

use cli_args::{make_app, CliArgs, Cmd, FetchArgs, LockArgs};
use config_file::ProtofetchConfig;
use fetch::{FetchError, checkout};

fn main() {
    let app_matches = make_app().get_matches();
    let cmd: Option<CliArgs> = match app_matches.subcommand() {
        ("fetch", Some(sub_m)) => {
            let should_lock = sub_m.is_present("relock");
            Some(CliArgs {
                cmd: Cmd::Fetch(FetchArgs { relock: should_lock }),
            })
        }
        ("lock", Some(sub_m)) => Some(CliArgs {
            cmd: Cmd::Lock(LockArgs {}),
        }),
        _ => None,
    };

    let conf_file = ProtofetchConfig::load().unwrap();

    println!("Hello, world! {:?}, {:?}", cmd, conf_file);

    do_fetch(conf_file);
}

fn do_fetch(conf: ProtofetchConfig) -> Result<(), FetchError> {
    println!("DepEntries: {:?}", conf.dep_entries);

    for (name, dep_entry) in conf.dep_entries {
        let repo = checkout(&name, &dep_entry.url, &conf.out_dir)?;

        println!("Repo checked out {:?} at {:?}", dep_entry, repo.repo_path);
    }

    Ok(())
}
