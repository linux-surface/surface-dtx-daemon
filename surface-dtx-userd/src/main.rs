mod cli;
mod config;
mod logic;
mod notify;
mod utils;

use crate::config::Config;

use anyhow::{Context, Result};
use slog::{Logger, crit, info, o};
use tokio::signal::unix::{SignalKind, signal};


fn build_logger(config: &Config) -> Logger {
    use slog::Drain;

    let decorator = slog_term::TermDecorator::new().build();

    let drain = slog_term::FullFormat::new(decorator)
        .use_original_order()
        .build()
        .filter_level(config.log.level.into())
        .fuse();

    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}

fn bootstrap() -> Result<(Logger, Config)> {
    // handle command line input
    let matches = cli::app().get_matches();

    // set up config
    let config = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    // set up logger
    let logger = build_logger(&config);

    Ok((logger, config))
}

async fn run(logger: Logger, _config: Config) -> Result<()> {
    // set up signal handling for shutdown
    let mut sigint = signal(SignalKind::interrupt()).context("Failed to set up signal handling")?;
    let mut sigterm = signal(SignalKind::terminate()).context("Failed to set up signal handling")?;

    let log = logger.clone();
    let sig = async move {
        let cause = tokio::select! {
            _ = sigint.recv()  => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        };

        info!(log, "received {}, shutting down...", cause);
    };

    // set up main logic task
    let main = logic::run(logger);

    // wait for error or shutdown signal
    tokio::select! {
        _   = sig  => Ok(()),
        res = main => res,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // no logger so we can't log errors here
    let (logger, config) = bootstrap()?;

    // run main function and log critical errors
    let result = run(logger.clone(), config).await;
    if let Err(ref err) = result {
        crit!(logger, "Critical error: {}\n", err);
    }

    result
}
