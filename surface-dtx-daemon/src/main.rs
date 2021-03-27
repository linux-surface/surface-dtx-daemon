mod cli;

mod config;
use config::Config;

mod logic;
use logic::EventHandler;

mod service;
use service::Service;

mod tq;
use tq::TaskQueue;

mod utils;
use utils::JoinHandleExt;


use std::sync::{Arc, Mutex};

use anyhow::{Context, Error, Result};

use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;
use dbus_tokio::connection;
use dbus_crossroads::Crossroads;

use futures::prelude::*;

use slog::{crit, debug, info, o, warn, Logger};

use tokio::signal::unix::{signal, SignalKind};


type Task = tq::Task<Error>;


fn build_logger(config: &Config) -> Logger {
    use slog::Drain;

    let decorator = slog_term::TermDecorator::new()
        .build();

    let drain = slog_term::FullFormat::new(decorator)
        .use_original_order()
        .build()
        .filter_level(config.log.level.into())
        .fuse();

    let drain = std::sync::Mutex::new(drain)
        .fuse();

    Logger::root(drain, o!())
}

fn bootstrap() -> Result<(Logger, Config)> {
    // handle command line input
    let matches = cli::app().get_matches();

    // set up config
    let (config, diag) = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    // set up logger
    let logger = build_logger(&config);

    // warn about unknown config items
    for item in diag.unknowns {
        warn!(logger, "Unknown config item"; "item" => item, "file" => ?diag.path)
    }

    Ok((logger, config))
}

async fn run(logger: Logger, config: Config) -> Result<()> {
    // set up signal handling
    let mut sigint = signal(SignalKind::interrupt()).context("Failed to set up signal handling")?;
    let mut sigterm = signal(SignalKind::terminate()).context("Failed to set up signal handling")?;

    let sig = async { tokio::select! {
        _ = sigint.recv()  => "SIGINT",
        _ = sigterm.recv() => "SIGTERM",
    }};

    // prepare devices
    let event_device = sdtx_tokio::connect().await
        .context("Failed to access DTX device")?;

    let control_device = sdtx_tokio::connect().await
        .context("Failed to access DTX device")?;

    // set up D-Bus connection
    let (dbus_rsrc, dbus_conn) = connection::new_system_sync()
        .context("Failed to connect to D-Bus")?;

    let dbus_rsrc = dbus_rsrc.map(|e| Err(e).context("D-Bus connection error"));
    let mut dbus_task = tokio::spawn(dbus_rsrc).guard();

    // set up D-Bus service
    let dbus_cr = Arc::new(Mutex::new(Crossroads::new()));

    let serv = Service::new(&logger, &dbus_conn, control_device);
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
    let (mut queue, queue_tx) = TaskQueue::new();
    let mut queue_task = tokio::spawn(async move { queue.run().await }).guard();

    // set up event handler
    let mut event_handler = EventHandler::new(&logger, config, &serv, event_device, queue_tx);
    let mut event_task = tokio::spawn(async move { event_handler.run().await }).guard();

    // collect main driver tasks
    let tasks = async { tokio::select! {
        result = &mut dbus_task  => result,
        result = &mut event_task => result,
        result = &mut queue_task => result,
    }};

    debug!(logger, "running...");

    // run until whatever comes first: error, panic, or shutdown signal
    tokio::select! {
        signame = sig => {
            // first shutdown signal: try to do a clean shutdown and complete
            // the task queue
            info!(logger, "received {}, shutting down...", signame);

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
                    warn!(logger, "received {} during shutdown, terminating...", signame);
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
    // no logger so we can't log errors here
    let (logger, config) = bootstrap()?;

    // run main function and log critical errors
    let result = run(logger.clone(), config).await;
    if let Err(ref err) = result {
        crit!(logger, "Critical error: {}\n", err);
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
