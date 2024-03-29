mod cli;
mod config;
mod logic;
mod utils;

use std::{path::PathBuf, io::IsTerminal};

use crate::config::Config;

use anyhow::{Context, Result};
use tokio::signal::unix::{SignalKind, signal};

use tracing::{error, info};


fn bootstrap() -> Result<Config> {
    // handle command line input
    let matches = cli::app().get_matches();

    // set up config
    let (config, diag) = match matches.get_one::<PathBuf>("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    // set up logger
    let filter = tracing_subscriber::EnvFilter::from_env("SDTXU_LOG")
        .add_directive(tracing::Level::from(config.log.level).into());

    let fmt = tracing_subscriber::fmt::format::PrettyFields::new();

    let subscriber = tracing_subscriber::fmt()
        .fmt_fields(fmt)
        .with_env_filter(filter)
        .with_ansi(std::io::stdout().is_terminal());

    if matches.get_flag("no-log-time") {
        subscriber.without_time().init();
    } else {
        subscriber.init();
    }

    // warn about unknown config items
    diag.log();

    Ok(config)
}

async fn run() -> Result<()> {
    let _config = bootstrap()?;

    // set up signal handling for shutdown
    let mut sigint = signal(SignalKind::interrupt()).context("Failed to set up signal handling")?;
    let mut sigterm = signal(SignalKind::terminate()).context("Failed to set up signal handling")?;

    let sig = async move {
        let cause = tokio::select! {
            _ = sigint.recv()  => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        };

        info!(target: "sdtxu", "received {}, shutting down...", cause);
    };

    // set up main logic task
    let main = logic::run();

    // wait for error or shutdown signal
    info!(target: "sdtxu", "running...");

    tokio::select! {
        _   = sig  => Ok(()),
        res = main => res,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // run main function and log critical errors
    let result = run().await;
    if let Err(ref err) = result {
        error!(target: "sdtxu", "critical error: {}\n", err);
    }

    result
}
