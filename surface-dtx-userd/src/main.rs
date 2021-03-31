mod cli;
mod config;
mod logic;
mod notify;
mod utils;

use crate::config::Config;

use anyhow::{Context, Result};
use tokio::signal::unix::{SignalKind, signal};

use tracing::{debug, error, info, warn};


fn bootstrap() -> Result<Config> {
    // handle command line input
    let matches = cli::app().get_matches();

    // set up config
    let (config, diag) = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    // set up logger
    let ansi = atty::is(atty::Stream::Stdout);

    let filter = tracing_subscriber::EnvFilter::from_env("SDTX_USERD_LOG")
        .add_directive(tracing::Level::from(config.log.level).into());

    let fmt = tracing_subscriber::fmt::format::PrettyFields::new()
        .with_ansi(ansi);

    tracing_subscriber::fmt()
        .fmt_fields(fmt)
        .with_env_filter(filter)
        .with_ansi(atty::is(atty::Stream::Stdout))
        .init();

    // warn about unknown config items
    let span = tracing::info_span!("config", file=?diag.path);
    span.in_scope(|| {
        debug!("configuration loaded");
        for item in diag.unknowns {
            warn!(item = %item, "unknown config item")
        }
    });

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

        info!("received {}, shutting down...", cause);
    };

    // set up main logic task
    let main = logic::run();

    // wait for error or shutdown signal
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
        error!("critical error: {}\n", err);
    }

    result
}
