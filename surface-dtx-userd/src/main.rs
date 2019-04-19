mod error;
use error::CliResult;

mod cli;


fn main() -> CliResult {
    let matches = cli::app().get_matches();

    Ok(())
}
