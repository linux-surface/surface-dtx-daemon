mod error;
use error::CliResult;

mod cli;

mod config;
use config::Config;


fn main() -> CliResult {
    let matches = cli::app().get_matches();

    let config = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    Ok(())
}
