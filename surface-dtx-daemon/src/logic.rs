use crate::Task;
use crate::config::Config;
use crate::service::{DetachState, Service};

use std::convert::TryFrom;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

use futures::prelude::*;

use sdtx::Event;
use sdtx::event::{BaseState, CancelReason, DeviceMode, LatchStatus};
use sdtx_tokio::Device;

use slog::{debug, error, info, trace, warn, Logger};

use tokio::process::Command;
use tokio::sync::mpsc::Sender;


pub struct EventHandler {
    log: Logger,
    config: Config,
    service: Arc<Service>,
    device: Arc<Device>,
    state: Arc<Mutex<State>>,
    task_queue_tx: Sender<Task>,
    ignore_request: u32,
}

impl EventHandler {
    pub fn new(log: &Logger, config: Config, service: &Arc<Service>, device: Device,
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

    pub async fn run(&mut self) -> Result<()> {
        let mut evdev = Device::from(self.device.file().try_clone().await?);

        // enable events
        let mut events = evdev.events_async()
            .context("DTX device error")?;

        // Update our state before we start handling events but after we've
        // enabled them. This way, we can ensure that we don't miss any
        // events/changes and accidentally set a stale state.
        let mode = self.device.get_device_mode()
            .context("DTX device error")?;

        self.service.set_device_mode(mode);

        // handle events
        while let Some(evt) = events.next().await {
            self.handle(evt.context("DTX device error")?)?;
        }

        Ok(())
    }

    pub fn handle(&mut self, evt: Event) -> Result<()> {
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