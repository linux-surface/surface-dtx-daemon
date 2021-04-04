use crate::config::Config;
use crate::logic::{Adapter, BaseInfo, CancelReason, DeviceMode, LatchState};
use crate::tq::TaskSender;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Error, Result};

use sdtx_tokio::Device;

use tokio::process::Command;

use tracing::{Level, debug, trace};


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeState {
    Ready,
    Detaching,
    Aborting,
    Attaching,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitStatus {
    Commence = 0,
    Abort    = 1,
}

impl ExitStatus {
    fn to_str(&self) -> &'static str {
        match self {
            Self::Commence => "0",
            Self::Abort    => "1",
        }
    }
}

impl From<std::process::ExitStatus> for ExitStatus {
    fn from(status: std::process::ExitStatus) -> Self {
        status.code().map(|s| if s == 0 {
            ExitStatus::Commence
        } else {
            ExitStatus::Abort
        }).unwrap_or(ExitStatus::Abort)
    }
}


pub struct ProcessAdapter {
    config: Config,
    device: Arc<Device>,
    queue: TaskSender<Error>,
    state: Arc<Mutex<RuntimeState>>,
}

impl ProcessAdapter {
    pub fn new(config: Config, device: Arc<Device>, queue: TaskSender<Error>) -> Self {
        Self {
            config,
            device,
            queue,
            state: Arc::new(Mutex::new(RuntimeState::Ready)),
        }
    }
}

impl Adapter for ProcessAdapter {
    fn set_state(&mut self, _mode: DeviceMode, _base: BaseInfo, _latch: LatchState) {
        *self.state.lock().unwrap() = RuntimeState::Ready;
    }

    fn detachment_start(&mut self) -> Result<()> {
        // state transition
        {
            let mut state = self.state.lock().unwrap();

            // if any subprocess is running (attach/abort), cancel the (new) request
            if *state != RuntimeState::Ready {
                debug!(target: "sdtxd::proc", "request: already processing, canceling this request");

                self.device.latch_cancel().context("DTX device error")?;
                return Ok(());
            }

            *state = RuntimeState::Detaching;
        }

        // TODO: heartbeat

        // build process task
        let state = self.state.clone();
        let device = self.device.clone();
        let dir = self.config.dir.clone();
        let handler = self.config.handler.detach.clone();
        let task = async move {
            trace!(target: "sdtxd::proc", "detachment process started");

            // run handler if specified
            let status = if let Some(ref path) = handler {
                debug!(target: "sdtxd::proc", ?path, ?dir, "running detachment handler");

                // run handler
                let output = Command::new(path)
                    .current_dir(dir)
                    .env("EXIT_DETACH_COMMENCE", ExitStatus::Commence.to_str())
                    .env("EXIT_DETACH_ABORT", ExitStatus::Abort.to_str())
                    .output().await
                    .context("Subprocess error (detachment)")?;

                // log output
                output.log("detachment handler");

                // confirm latch open/detach commence based on return status
                ExitStatus::from(output.status)

            } else {
                debug!(target: "sdtxd::proc", "no detachment handler specified, skipping");
                ExitStatus::Commence
            };

            // send response only if detachment has not been canceled already
            if *state.lock().unwrap() == RuntimeState::Detaching {
                if status == ExitStatus::Commence {
                    debug!(target: "sdtxd::proc", "detachment commencing based on handler response");
                    device.latch_confirm().context("DTX device error")?;
                } else {
                    debug!(target: "sdtxd::proc", "detachment canceled based on handler response");
                    device.latch_cancel().context("DTX device error")?;
                }
            } else {
                debug!(target: "sdtxd::proc", "detachment has already been canceled, skipping");
            }

            trace!(target: "sdtxd::proc", "detachment process completed");
            Ok(())
        };

        // submit task
        trace!(target: "sdtxd::proc", "scheduling detachment task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }

    fn detachment_cancel(&mut self, _reason: CancelReason) -> Result<()> {
        // state transition
        {
            let mut state = self.state.lock().unwrap();

            // We might have canceled in detachment_start() while the EC itself
            // was ready for detachment again. This will lead to a
            // detachment_cancel() call while we are already aborting. Make
            // sure that we're only scheduling the abort task once.
            if *state != RuntimeState::Detaching {
                return Ok(());
            }

            *state = RuntimeState::Aborting;
        }

        // build task
        let state = self.state.clone();
        let dir = self.config.dir.clone();
        let handler = self.config.handler.detach_abort.clone();
        let task = async move {
            trace!(target: "sdtxd::proc", "detachment-abort process started");

            // run handler if specified
            if let Some(ref path) = handler {
                debug!(target: "sdtxd::proc", ?path, ?dir, "running detachment-abort handler");

                // run handler
                let output = Command::new(path)
                    .current_dir(dir)
                    .output().await
                    .context("Subprocess error (detachment-abort)")?;

                // log output
                output.log("detachment-abort handler");

            } else {
                debug!(target: "sdtxd::proc", "no detachment-abort handler specified, skipping");
            };

            trace!(target: "sdtxd::proc", "detachment-abort process completed");
            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

        // submit task
        trace!(target: "sdtxd::proc", "scheduling detachment-abort task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }

    fn detachment_complete(&mut self) -> Result<()> {
        *self.state.lock().unwrap() = RuntimeState::Ready;
        Ok(())
    }

    fn attachment_complete(&mut self) -> Result<()> {
        // state transition
        *self.state.lock().unwrap() = RuntimeState::Attaching;

        // build task
        let state = self.state.clone();
        let dir = self.config.dir.clone();
        let handler = self.config.handler.attach.clone();
        let delay = Duration::from_millis((self.config.delay.attach * 1000.0) as _);
        let task = async move {
            trace!(target: "sdtxd::proc", "attachment process started");

            // delay to ensure all devices are set up
            debug!(target: "sdtxd::proc", "delaying attachment process by {}ms", delay.as_millis());
            tokio::time::sleep(delay).await;

            // run handler if specified
            if let Some(ref path) = handler {
                debug!(target: "sdtxd::proc", ?path, ?dir, "running attachment handler");

                // run handler
                let output = Command::new(path)
                    .current_dir(dir)
                    .output().await
                    .context("Subprocess error (attachment)")?;

                // log output
                output.log("attachment handler");

            } else {
                debug!(target: "sdtxd::proc", "no attachment handler specified, skipping");
            };

            trace!(target: "sdtxd::proc", "attachment process completed");
            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

        // submit task
        trace!(target: "sdtxd::proc", "scheduling attachment task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }
}



trait ProcessOutputExt {
    fn log<S: AsRef<str>>(&self, procname: S);
}

impl ProcessOutputExt for std::process::Output {
    fn log<S: AsRef<str>>(&self, procname: S) {

        fn log_stream(level: Level, name: &'static str, data: &[u8]) {
            if !data.is_empty() {
                event!(target: "sdtxd::proc", level, "  (contd.)");
                event!(target: "sdtxd::proc", level, "  (contd.) {}:", name);

                let data = std::str::from_utf8(data);
                match data {
                    Ok(ref str) => {
                        for line in str.lines() {
                            event!(target: "sdtxd::proc", level, "  (contd.)   {}", line);
                        }
                    },
                    Err(_) => {
                        event!(target: "sdtxd::proc", level, "  (contd.)   {:?}", data);
                    },
                }
            }
        }

        let level = if !self.stderr.is_empty() {
            tracing::Level::WARN
        } else if !self.stdout.is_empty() {
            tracing::Level::INFO
        } else {
            tracing::Level::DEBUG
        };

        event!(target: "sdtxd::proc", level, "{} exited with {}", procname.as_ref(), self.status);
        log_stream(level, "stdout", &self.stdout);
        log_stream(level, "stderr", &self.stderr);
    }
}
