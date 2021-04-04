#[macro_use]
mod utils;
use utils::JoinHandleExt;

mod cli;

mod config;
use config::Config;

mod logic;

mod service;
use service::Service;

mod tq;


use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;
use dbus_tokio::connection;
use dbus_crossroads::Crossroads;

use futures::prelude::*;

use tokio::signal::unix::{signal, SignalKind};

use tracing::{error, info, trace, warn};


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

    let filter = tracing_subscriber::EnvFilter::from_env("SDTXD_LOG")
        .add_directive(tracing::Level::from(config.log.level).into());

    let fmt = tracing_subscriber::fmt::format::PrettyFields::new()
        .with_ansi(ansi);

    tracing_subscriber::fmt()
        .fmt_fields(fmt)
        .with_env_filter(filter)
        .with_ansi(atty::is(atty::Stream::Stdout))
        .init();

    // warn about unknown config items
    diag.log();

    Ok(config)
}

async fn run() -> Result<()> {
    let config = bootstrap()?;

    // set up signal handling
    trace!(target: "sdtxd", "setting up signal handling");

    let mut sigint = signal(SignalKind::interrupt()).context("Failed to set up signal handling")?;
    let mut sigterm = signal(SignalKind::terminate()).context("Failed to set up signal handling")?;

    let sig = async { tokio::select! {
        _ = sigint.recv()  => "SIGINT",
        _ = sigterm.recv() => "SIGTERM",
    }};

    // prepare devices
    trace!(target: "sdtxd", "preparing devices");

    let event_device = sdtx_tokio::connect().await
        .context("Failed to access DTX device")?;

    let control_device = sdtx_tokio::connect().await
        .context("Failed to access DTX device")?;

    // set up D-Bus connection
    trace!(target: "sdtxd", "connecting to D-Bus");

    let (dbus_rsrc, dbus_conn) = connection::new_system_sync()
        .context("Failed to connect to D-Bus")?;

    let dbus_rsrc = dbus_rsrc.map(|e| Err(e).context("D-Bus connection error"));
    let mut dbus_task = tokio::spawn(dbus_rsrc).guard();

    // set up D-Bus service
    trace!(target: "sdtxd", "setting up D-Bus service");

    let dbus_cr = Arc::new(Mutex::new(Crossroads::new()));

    let serv = Service::new(dbus_conn.clone(), control_device);
    serv.request_name().await?;
    serv.register(&mut dbus_cr.lock().unwrap())?;

    let cr = dbus_cr.clone();
    let token = dbus_conn.start_receive(MatchRule::new_method_call(), Box::new(move |msg, conn| {
        // Crossroads::handle_message() only fails if message is not a method call
        cr.lock().unwrap().handle_message(msg, conn).unwrap();
        true
    }));

    let recv_guard = utils::guard(|| { let _ = dbus_conn.stop_receive(token).unwrap(); });
    let serv_guard = utils::guard(|| { serv.unregister(&mut dbus_cr.lock().unwrap()); });

    // set up task-queue
    trace!(target: "sdtxd", "setting up task queue");

    let (mut queue, queue_tx) = tq::new();
    let mut queue_task = tokio::spawn(async move { queue.run().await }).guard();

    // set up event handler
    trace!(target: "sdtxd", "setting up DTX event handling");

    let adapter = logic::ProcessAdapter::new(config, queue_tx);
    let mut core = logic::Core::new(event_device, adapter);
    let mut event_task = tokio::spawn(async move { core.run().await }).guard();

    // collect main driver tasks
    let tasks = async { tokio::select! {
        result = &mut dbus_task  => result,
        result = &mut event_task => result,
        result = &mut queue_task => result,
    }};

    // run until whatever comes first: error, panic, or shutdown signal
    info!(target: "sdtxd", "running...");

    tokio::select! {
        signame = sig => {
            // first shutdown signal: try to do a clean shutdown and complete
            // the task queue
            info!(target: "sdtxd", "received {}, shutting down...", signame);

            // stop event task: don't handle any new DTX events and drop task
            // queue transmitter to eventually cause the task queue task to
            // complete
            event_task.abort();

            // unregister service
            drop(serv_guard);

            // stop D-Bus message handling
            drop(recv_guard);

            // pepare handling for second shutdown signal
            let sig = async { tokio::select! {
                _ = sigint.recv()  => ("SIGINT",   2),
                _ = sigterm.recv() => ("SIGTERM", 15),
            }};

            // try to run task queue to completion, shut down and exit if
            // second signal received
            tokio::select! {
                (signame, tval) = sig => {
                    warn!(target: "sdtxd", "received {} during shutdown, terminating...", signame);
                    std::process::exit(128 + tval)
                },
                result = queue_task => match result {
                    Ok(res) => res,
                    Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
                    Err(_) => unreachable!("Task unexpectedly canceled"),
                }
            }
        }
        result = tasks => match result {
            Ok(res) => res,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(_) => unreachable!("Task unexpectedly canceled"),
        },
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // run main function and log critical errors
    let result = run().await;
    if let Err(ref err) = result {
        error!(target: "sdtxd", "critical error: {}\n", err);
    }

    // for some reason tokio won't properly shut down, even though every task
    // we spawned should be either canceled or completed by now...
    if let Err(err) = result {
        eprintln!("{:?}", err);
        std::process::exit(1)
    } else {
        std::process::exit(0)
    }
}
