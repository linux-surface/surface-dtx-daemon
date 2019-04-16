mod error;

mod cli;

mod config;
use config::Config;

mod device;
use device::{Device, Event, RawEvent, OpMode, LatchState, ConnectionState};

use std::time::Duration;
use std::convert::TryFrom;
use std::rc::Rc;

use tokio::prelude::*;
use tokio::runtime::current_thread::Runtime;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_signal::unix::{Signal, SIGINT, SIGTERM};

use slog::{Logger, trace, debug, info, warn, error, o};

use crate::error::{Error, Result};


fn logger(config: &Config) -> Logger {
    use slog::{Drain};

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

    // set-up task-queue for external processes
    let (queue_tx, queue_rx) = tokio::sync::mpsc::channel(32);

    // event handler
    let device = Device::open()?;
    let event_task = setup_event_task(logger.clone(), device, queue_tx)?;

    // process queue handler
    let process_task = setup_process_task(queue_rx).shared();
    let shutdown_task = process_task.clone().map(|_| ());

    // shutdown handler
    let signal = setup_shutdown_signal(logger.clone(), shutdown_task);
    let event_task = event_task.select(signal);

    // only critical errors will reach this point
    let event_task = event_task.map(|_| ()).map_err(|(e, _)| {
        panic!(format!("{}", e))
    });

    let process_task = process_task.map(|_| ()).map_err(|e| {
        panic!(format!("{}", e))
    });

    debug!(logger, "running...");
    Runtime::new()?
        .spawn(process_task)
        .spawn(event_task)
        .run().unwrap();

    Ok(())
}


fn setup_shutdown_signal<F>(log: Logger, shutdown_task: F) -> impl Future<Item=(), Error=Error>
where
    F: Future + 'static,
    <F as Future>::Error: std::fmt::Display,
{
    let signal = {
        let sigint = Signal::new(SIGINT).flatten_stream();
        let sigterm = Signal::new(SIGTERM).flatten_stream();

        sigint.select(sigterm).into_future()
            .map_err(|(e, _)| Error::from(e))
    };

    signal.map(move |(sig, next)| {
        info!(log, "shutting down...");

        // actual shutdown code provided via shutdown_task: wait for completion
        let l = log.clone();
        let task = shutdown_task.map(|_| ()).map_err(move |e| {
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
    })
}

fn setup_event_task(log: Logger, device: Device, queue_tx: Sender<DtxProcess>)
    -> Result<impl Future<Item=(), Error=Error>>
{
    let events = device.events()?.map_err(Error::from);

    let mut handler = EventHandler::new(log, Rc::new(device), queue_tx);
    let task = events.for_each(move |evt| {
        handler.handle(evt)
    });

    Ok(task)
}

fn setup_process_task(queue_rx: Receiver<DtxProcess>)
    -> impl Future<Item=(), Error=Error>
{
    queue_rx.map_err(|e| panic!(e)).for_each(|task| {
        task
    })
}


type DtxProcess = Box<dyn Future<Item=(), Error=Error>>;


struct EventHandler {
    log: Logger,
    device: Rc<Device>,
    queue_tx: Sender<DtxProcess>,
}

impl EventHandler {
    fn new(log: Logger, device: Rc<Device>, queue_tx: Sender<DtxProcess>) -> Self {
        EventHandler { log, device, queue_tx }
    }


    fn handle(&mut self, evt: RawEvent) -> Result<()> {
        trace!(self.log, "received event"; "event" => ?evt);

        match Event::try_from(evt) {
            Ok(Event::OpModeChange { mode }) => {
                self.on_opmode_change(mode);
            },
            Ok(Event::ConectionChange { state, arg1: _ }) => {
                self.on_connection_change(state);
            },
            Ok(Event::LatchStateChange { state }) => {
                self.on_latch_state_change(state);
            },
            Ok(Event::DetachRequest) => {
                self.on_detach_request()
            },
            Ok(Event::DetachError { err }) => {
                self.on_detach_error(err)
            },
            Err(evt) => {
                warn!(self.log, "unhandled event";
                    "type" => evt.typ,  "code" => evt.code,
                    "arg0" => evt.arg0, "arg1" => evt.arg1
                );
            },
        }

        Ok(())
    }


    fn on_opmode_change(&mut self, mode: OpMode) {
        debug!(self.log, "op-mode changed: {:?}", mode);                // TODO
    }

    fn on_connection_change(&mut self, state: ConnectionState) {
        debug!(self.log, "connection changed: {:?}", state);            // TODO
    }

    fn on_latch_state_change(&mut self, state: LatchState) {
        debug!(self.log, "latch-state changed: {:?}", state);           // TODO
    }

    fn on_detach_request(&mut self) {
        debug!(self.log, "detach requested");                           // TODO

        let log = self.log.clone();
        let dev = self.device.clone();

        let task = tokio_timer::sleep(Duration::from_secs(5)).map_err(|e| panic!(e));
        let task = task.and_then(move |_| {
            debug!(log, "detachment process completed");
            dev.commands().latch_open()
        });

        self.queue_proc_task(Box::new(task));
    }

    fn on_detach_error(&mut self, err: u8) {
        debug!(self.log, "detach error: {}", err);                      // TODO
    }


    fn queue_proc_task(&mut self, task: DtxProcess) {
        let res = self.queue_tx.try_send(Box::new(task));
        if let Err(e) = res {
            if e.is_full() {
                warn!(self.log, "process queue is full, dropping task")
            } else {
                unreachable!("process queue closed")
            }
        }
    }
}
