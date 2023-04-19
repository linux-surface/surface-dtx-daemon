use clap::{Arg, Command, ArgAction};

pub fn app() -> Command {
    Command::new("Surface DTX User Daemon")
        .about(clap::crate_description!())
        .version(clap::crate_version!())
        .author(clap::crate_authors!())
        .arg(Arg::new("config")
            .short('c')
            .long("config")
            .value_name("FILE")
            .help("Use the specified config file")
            .value_parser(clap::value_parser!(std::path::PathBuf)))
        .arg(Arg::new("no-log-time")
            .long("no-log-time")
            .help("Do not emit timestamps in log")
            .action(ArgAction::SetTrue))
}
