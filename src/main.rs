mod error;

mod cli;

mod config;
use config::Config;

mod device;
use device::{Device, Event};

use std::time::Duration;
use std::convert::TryFrom;

use tokio::prelude::*;
use tokio::runtime::current_thread::Runtime;
use tokio_signal::unix::{Signal, SIGINT, SIGTERM};

use slog::{Logger, debug, info, warn, error};

use crate::error::{Error, Result};


fn logger(config: &Config) -> Logger {
    use slog::{o, Drain};

    let decorator = slog_term::TermDecorator::new()
        .build();

    let drain = slog_term::FullFormat::new(decorator)
        .use_original_order()
        .build()
        .filter_level(config.log.level.into())
        .fuse();

    let drain = std::sync::Mutex::new(drain)
        .fuse();

    slog::Logger::root(drain, o!())
}

fn main() -> Result<()> {
    let matches = cli::app().get_matches();

    let config = match matches.value_of("config") {
        Some(path) => config::Config::load_file(path)?,
        None       => config::Config::load()?,
    };

    let logger = logger(&config);

    let signal = {
        let sigint = Signal::new(SIGINT).flatten_stream();
        let sigterm = Signal::new(SIGTERM).flatten_stream();

        sigint.select(sigterm).into_future()
            .map_err(|(e, _)| Error::from(e))
    };

    // shutdown handler
    let log = logger.clone();
    let signal = signal.map(move |(sig, next)| {
        info!(log, "shutting down...");

        // TODO: actual shutdown code
        let l = log.clone();
        let task = tokio_timer::sleep(Duration::from_millis(5000)).map(move |_| {
            info!(l, "shutdown procedure done");
        });

        let l = log.clone();
        let task = task.map_err(move |e| {
            error!(l, "error while terminating: {}", e);
        });

        // on second signal: terminate, no questions asked
        let l = log.clone();
        let term = next.into_future().then(move |_| -> std::result::Result<(), ()> {
            info!(l, "terminating...");
            std::process::exit(128 + sig.unwrap_or(SIGINT))
        });

        let task = task.select(term)
            .map(|_| ()).map_err(|_| ());

        tokio::runtime::current_thread::spawn(task)
    });

    let device = Device::open()?;

    // event handler
    let log = logger.clone();
    let task = device.events()?.map_err(Error::from).for_each(move |evt| {
        debug!(log, "received event"; "event" => ?evt);

        match Event::try_from(evt) {
            Ok(Event::OpModeChange { mode }) => {
                debug!(log, "op-mode changed: {:?}", mode);                 // TODO
            },
            Ok(Event::ConectionChange { state, arg1: _ }) => {
                debug!(log, "connection-state changed: {:?}", state);       // TODO
            },
            Ok(Event::LatchStateChange { state }) => {
                debug!(log, "latch-state changed: {:?}", state);            // TODO
            },
            Ok(Event::DetachRequest) => {
                debug!(log, "detach requested");                            // TODO
            },
            Ok(Event::DetachError { err }) => {
                debug!(log, "detach error: {}", err);                       // TODO
            },
            Err(evt) => {
                warn!(log, "unhandled event";
                    "type" => evt.typ,  "code" => evt.code,
                    "arg0" => evt.arg0, "arg1" => evt.arg1
                );
            },
        }

        Ok(())
    });

    let task = task.select(signal)
        .map(|_| ()).map_err(|(e, _)| panic!(e));

    debug!(logger, "Starting...");
    let mut rt = Runtime::new()?;
    rt.spawn(task).run().unwrap();

    Ok(())
}
