mod cli;

mod config;
use config::Config;

mod service;
use service::{DetachState, Service};

mod tq;
use tq::TaskQueue;

mod utils;
use utils::JoinHandleExt;


use std::convert::TryFrom;
use std::ffi::OsStr;
use std::future::Future;
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Error, Result};

use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;
use dbus_tokio::connection;
use dbus_crossroads::Crossroads;

use futures::prelude::*;

use sdtx::Event;
use sdtx::event::{BaseState, CancelReason, DeviceMode, LatchStatus};

use slog::{crit, debug, error, info, o, trace, warn, Logger};

use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;


type ControlDevice = sdtx::Device<std::fs::File>;
type EventDevice = sdtx_tokio::Device;

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
    let config = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    // set up logger
    let logger = build_logger(&config);

    Ok((logger, config))
}

async fn run(logger: Logger, config: Config) -> Result<()> {
    let event_device = sdtx_tokio::connect().await
        .context("Failed to access DTX device")?;

    let control_device = Arc::new(sdtx::connect()
        .context("Failed to access DTX device")?);

    // set-up task-queue for external processes
    let (mut queue, queue_tx) = TaskQueue::new();

    // set up D-Bus connection
    let (dbus_rsrc, dbus_conn) = connection::new_system_sync()
        .context("Failed to connect to D-Bus")?;

    let dbus_rsrc = dbus_rsrc.map(|e| Err(e).context("D-Bus connection error"));
    let dbus_task = tokio::spawn(dbus_rsrc).guard();

    // set up D-Bus service
    let mut dbus_cr = Crossroads::new();

    let serv = Service::new(&logger, &dbus_conn, &control_device);
    serv.request_name().await?;
    serv.register(&mut dbus_cr)?;

    dbus_conn.start_receive(MatchRule::new_method_call(), Box::new(move |msg, conn| {
        // Crossroads::handle_message() only fails if message is not a method call
        dbus_cr.handle_message(msg, conn).unwrap();
        true
    }));

    // event handler
    let mut event_handler = EventHandler::new(&logger, config, &serv, event_device, queue_tx);
    let event_task = event_handler.run();

    // This implementation is structured around two main tasks: process_task
    // and main_task. main_task drives all processing, while process_task is
    // the subtask managing the task queue. When the first shutdown signal has
    // been received, main_task stops all processing and waits for the shutdown
    // driver to complete. The shutdown driver will then continue to run
    // process_task, either to completion or until the second signal has been
    // received. This way, under normal operation, all events received befor
    // the first shutdown signal will be properly handled.

    // process queue handler and init stuff
    let process_task = async move {
        // make sure the device-mode in the service is up to date
        let mode = control_device.get_device_mode()
            .context("DTX device error")?;

        serv.set_device_mode(mode);

        queue.run().await
    };

    // set up shutdown so that process_task is driven to completion
    let log = logger.clone();
    let process_task = process_task.map(move |result| {
        if let Err(e) = result {
            panic_with_critical_error(&log, &e);
        }
    });

    let process_task = process_task.shared();

    // shutdown handler and main task
    let shutdown_signal = shutdown_signal(logger.clone(), process_task.clone());

    debug!(logger, "running...");
    let result = tokio::select! {
        res = shutdown_signal => res.map(Some),
        res = event_task => res.map(|_| None),
        _ = process_task => Ok(None),
        res = dbus_task => match res {
            Ok(res) => res,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(_) => unreachable!("Task unexpectedly canceled"),
        },
    };

    // wait for shutdown driver to complete
    match result {
        Ok(Some(shutdown_driver)) => shutdown_driver.await.context("Runtime error"),
        x => x.map(|_| ()),
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


async fn shutdown_signal<F>(log: Logger, shutdown_task: F) -> Result<JoinHandle<()>>
where
    F: Future<Output=()> + 'static + Send,
{
    let mut sigint = signal(SignalKind::interrupt()).context("Failed to set up signal handling")?;
    let mut sigterm = signal(SignalKind::terminate()).context("Failed to set up signal handling")?;

    // wait for first signal
    let sig = tokio::select! {
        _ = sigint.recv()  => "SIGINT",
        _ = sigterm.recv() => "SIGTERM",
    };

    info!(log, "received {}, shutting down...", sig);

    // schedule driver for completion
    let driver = async move {
        let (sig, tval) = tokio::select! {
            _ = sigint.recv()  => ("SIGINT",   2),  // = value of SIGINT
            _ = sigterm.recv() => ("SIGTERM", 15),  // = value of SIGTERM
            _ = shutdown_task  => ("OK",       0),
        };

        if tval != 0 {
            warn!(log, "received {} during shutdown, terminating...", sig);
            std::process::exit(128 + tval)
        }
    };

    Ok(tokio::spawn(driver))
}


struct EventHandler {
    log: Logger,
    config: Config,
    service: Arc<Service>,
    device: Arc<EventDevice>,
    state: Arc<Mutex<State>>,
    task_queue_tx: Sender<Task>,
    ignore_request: u32,
}

impl EventHandler {
    fn new(log: &Logger, config: Config, service: &Arc<Service>, device: EventDevice,
           task_queue_tx: Sender<Task>)
        -> Self
    {
        EventHandler {
            log: log.clone(),
            config,
            service: service.clone(),
            task_queue_tx,
            device: Arc::new(device),
            state: Arc::new(Mutex::new(State::Normal)),
            ignore_request: 0,
        }
    }

    async fn run(&mut self) -> Result<()> {
        let mut evdev = EventDevice::from(self.device.file().try_clone().await?);

        let mut events = evdev.events_async()
            .context("DTX device error")?;

        while let Some(evt) = events.next().await {
            self.handle(evt.context("DTX device error")?)?;
        }

        Ok(())
    }

    fn handle(&mut self, evt: Event) -> Result<()> {
        trace!(self.log, "received event"; "event" => ?evt);

        match evt {
            Event::DeviceMode { mode } => {
                self.on_device_mode_change(mode);
            },
            Event::BaseConnection { state, .. } => {
                self.on_connection_change(state);
            },
            Event::LatchStatus { status } => {
                self.on_latch_state_change(status);
            },
            Event::Request => {
                self.on_detach_request()?;
            },
            Event::Cancel { reason } => {
                self.on_detach_error(reason);
            },
            Event::Unknown { code, data } => {
                warn!(self.log, "unhandled event"; "code" => code, "data" => ?data);
            },
        }

        Ok(())
    }

    fn on_device_mode_change(&mut self, mode: DeviceMode) {
        debug!(self.log, "device mode changed"; "mode" => ?mode);

        if let DeviceMode::Unknown(mode) = mode {
            error!(self.log, "unknown device mode"; "mode" => mode);
            return;
        }

        let mode = sdtx::DeviceMode::try_from(mode).unwrap();
        self.service.set_device_mode(mode);
    }

    fn on_latch_state_change(&mut self, status: LatchStatus) {
        debug!(self.log, "latch-state changed"; "status" => ?status);

        match status {
            LatchStatus::Opened => {
                self.service.signal_detach_state_change(DetachState::DetachReady)
            },
            LatchStatus::Closed => {},
            LatchStatus::Error(e) => {
                warn!(self.log, "latch status error"; "error" => ?e);
            },
            LatchStatus::Unknown(x) => {
                error!(self.log, "unknown latch status"; "status" => x);
            },
        }
    }

    fn on_connection_change(&mut self, base_state: BaseState) {
        debug!(self.log, "clipboard connection changed"; "state" => ?base_state);

        let state = *self.state.lock().unwrap();
        match (state, base_state) {
            (State::Detaching, BaseState::Detached) => {
                *self.state.lock().unwrap() = State::Normal;
                self.service.signal_detach_state_change(DetachState::DetachCompleted);
                debug!(self.log, "detachment procedure completed");
            },
            (State::Normal, BaseState::Attached) => {
                { *self.state.lock().unwrap() = State::Attaching; }
                self.schedule_task_attach();
            },
            (_, BaseState::NotFeasible) => {
                info!(self.log, "connection changed to not feasible";
                      "state" => ?(state, base_state));

                // TODO: what to do here?
            },
            _ => {
                error!(self.log, "invalid state"; "state" => ?(state, base_state));
            },
        }
    }

    fn on_detach_request(&mut self) -> Result<()> {
        if self.ignore_request > 0 {
            self.ignore_request -= 1;
            return Ok(());
        }

        let state = *self.state.lock().unwrap();
        match state {
            State::Normal => {
                debug!(self.log, "clipboard detach requested");
                *self.state.lock().unwrap() = State::Detaching;
                self.schedule_task_detach();
            },
            State::Detaching => {
                debug!(self.log, "clipboard detach-abort requested");
                *self.state.lock().unwrap() = State::Aborting;
                self.service.signal_detach_state_change(DetachState::DetachAborted);
                self.schedule_task_detach_abort();
            },
            State::Aborting | State::Attaching => {
                self.ignore_request += 1;
                self.device.latch_request().context("DTX latch request failed")?;
            },
        }

        Ok(())
    }

    fn on_detach_error(&mut self, err: CancelReason) {
        match err {
            CancelReason::Runtime(e)  => info!(self.log, "detachment procedure canceled: {}", e),
            CancelReason::Hardware(e) => warn!(self.log, "hardware failure, aborting detachment: {}", e),
            CancelReason::Unknown(x)  => error!(self.log, "unknown failure, aborting detachment: {}", x),
        }

        if *self.state.lock().unwrap() == State::Detaching {
            *self.state.lock().unwrap() = State::Aborting;
            self.schedule_task_detach_abort();
        }
    }

    fn schedule_task_attach(&mut self) {
        let log = self.log.clone();
        let delay = Duration::from_millis((self.config.delay.attach * 1000.0) as _);
        let handler = self.config.handler.attach.clone();
        let dir = self.config.dir.clone();
        let state = self.state.clone();
        let service = self.service.clone();

        let task = async move {
            debug!(log, "subprocess: delaying attach process");
            tokio::time::sleep(delay).await;

            if let Some(path) = handler {
                debug!(log, "subprocess: attach started, executing '{}'", path.display());

                let output = Command::new(path)
                    .current_dir(dir)
                    .output().await
                    .context("Subprocess error (attach)")?;

                log_process_output(&log, &output);
                debug!(log, "subprocess: attach finished");

            } else {
                debug!(log, "subprocess: no attach handler executable");
            }

            *state.lock().unwrap() = State::Normal;
            service.signal_detach_state_change(DetachState::AttachCompleted);

            Ok(())
        };

        self.schedule_process_task(Box::pin(task));
    }

    fn schedule_task_detach(&mut self) {
        let log = self.log.clone();
        let handler = self.config.handler.detach.clone();
        let dir = self.config.dir.clone();
        let state = self.state.clone();
        let device = self.device.clone();

        let task = async move {
            if let Some(ref path) = handler {
                debug!(log, "subprocess: detach started");

                let output = Command::new(path)
                    .current_dir(dir)
                    .env("EXIT_DETACH_COMMENCE", "0")
                    .env("EXIT_DETACH_ABORT", "1")
                    .output().await
                    .context("Subprocess error (detach)")?;

                log_process_output(&log, &output);
                debug!(log, "subprocess: detach finished");

                if *state.lock().unwrap() == State::Detaching {
                    if output.status.success() {
                        debug!(log, "commencing detach, opening latch");
                        device.latch_confirm().context("DTX latch confirmation failed")?;
                    } else {
                        info!(log, "aborting detach");
                        device.latch_cancel().context("DTX latch cancel request failed")?;
                    }
                } else {
                    debug!(log, "state changed during detachment, not opening latch");
                }

            } else {
                debug!(log, "subprocess: no detach handler executable");

                if *state.lock().unwrap() == State::Detaching {
                    debug!(log, "commencing detach, opening latch");
                    device.latch_confirm().context("DTX latch confirmation failed")?;
                } else {
                    debug!(log, "state changed during detachment, not opening latch");
                }
            }

            Ok(())
        };

        self.schedule_process_task(Box::pin(task));
    }

    fn schedule_task_detach_abort(&mut self) {
        let log = self.log.clone();
        let handler = self.config.handler.detach_abort.clone();
        let dir = self.config.dir.clone();
        let state = self.state.clone();

        let task = async move {
            if let Some(ref path) = handler {
                debug!(log, "subprocess: detach_abort started");

                let output = Command::new(path)
                    .current_dir(dir)
                    .output().await
                    .context("Subprocess error (detach_abort)")?;

                log_process_output(&log, &output);
                debug!(log, "subprocess: detach_abort finished");

            } else {
                debug!(log, "subprocess: no detach_abort handler executable");
            }

            *state.lock().unwrap() = State::Normal;
            Ok(())
        };

        self.schedule_process_task(Box::pin(task));
    }

    fn schedule_process_task(&mut self, task: Task) {
        use tokio::sync::mpsc::error::TrySendError;

        match self.task_queue_tx.try_send(task) {
            Err(TrySendError::Full(_)) => {
                warn!(self.log, "process queue is full, dropping task");
            },
            Err(TrySendError::Closed(_)) => {
                unreachable!("process queue closed");
            },
            Ok(_) => {},
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State { Normal, Detaching, Aborting, Attaching }

fn log_process_output(log: &Logger, output: &std::process::Output) {
    if !output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty() {
        info!(log, "subprocess terminated with {}", output.status);
    }

    if !output.stdout.is_empty() {
        let stdout = OsStr::from_bytes(&output.stdout);
        info!(log, "subprocess terminated with stdout: {:?}", stdout);
    }

    if !output.stderr.is_empty() {
        let stderr = OsStr::from_bytes(&output.stderr);
        info!(log, "subprocess terminated with stderr: {:?}", stderr);
    }
}

fn panic_with_critical_error(log: &Logger, err: &Error) -> ! {
    crit!(log, "Error: {}", err);
    for cause in err.chain() {
        crit!(log, "Caused by: {}", cause);
    }

    panic!("{}", err)
}
