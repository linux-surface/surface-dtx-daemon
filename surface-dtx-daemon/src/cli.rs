use clap::{App, Arg};

pub fn app() -> App<'static, 'static> {
    App::new("Surface DTX Daemon")
        .about("Detachment System Daemon for Surface Book 2.")
        .version(clap::crate_version!())
        .author("Maximilian Luz <luzmaximilian@gmail.com>")
        .arg(Arg::with_name("config")
            .short("c")
            .long("config")
            .value_name("FILE")
            .help("Use the specified config file")
            .takes_value(true))
}
