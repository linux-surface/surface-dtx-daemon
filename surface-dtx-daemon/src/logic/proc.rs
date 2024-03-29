use crate::config::Config;
use crate::logic::{
    Adapter,
    AtHandle,
    DtHandle,
    DtcHandle,
};
use crate::utils::taskq::TaskSender;

use std::time::Duration;

use anyhow::{Context, Error, Result};
use tokio::process::Command;
use tracing::{Level, debug, trace};


const HEARTBEAT_PERIOD_MS: u64 = 2500;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitStatus {
    Commence = 0,
    Abort    = 1,
}

impl ExitStatus {
    fn as_str(&self) -> &'static str {
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
    queue: TaskSender<Error>,
}

impl ProcessAdapter {
    pub fn new(config: Config, queue: TaskSender<Error>) -> Self {
        Self {
            config,
            queue,
        }
    }
}

impl Adapter for ProcessAdapter {
    fn detachment_start(&mut self, handle: DtHandle) -> Result<()> {
        // build heartbeat task
        let h = handle.clone();
        let heartbeat = async move {
            loop {
                tokio::time::sleep(Duration::from_millis(HEARTBEAT_PERIOD_MS)).await;
                h.heartbeat()?;
            }
        };

        // build timeout task
        let h = handle.clone();
        let timeout = self.config.handler.detach.timeout * 1000.0;
        let timeout = async move {
            tokio::time::sleep(Duration::from_millis(timeout as _)).await;

            trace!(target: "sdtxd::proc", "detachment process timed out, canceling");
            h.timeout();

            Ok(())
        };

        // build process task
        let dir = self.config.dir.clone();
        let handler = self.config.handler.detach.exec.clone();
        let proc = async move {
            trace!(target: "sdtxd::proc", "detachment process started");

            // run handler if specified
            let status = if let Some(ref path) = handler {
                debug!(target: "sdtxd::proc", ?path, ?dir, "running detachment handler");

                // run handler
                let output = Command::new(path)
                    .current_dir(dir)
                    .env("EXIT_DETACH_COMMENCE", ExitStatus::Commence.as_str())
                    .env("EXIT_DETACH_ABORT", ExitStatus::Abort.as_str())
                    .kill_on_drop(true)
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

            // send response, will be ignored if already canceled
            if status == ExitStatus::Commence {
                debug!(target: "sdtxd::proc", "detachment commencing based on handler response");
                handle.confirm();
            } else {
                debug!(target: "sdtxd::proc", "detachment canceled based on handler response");
                handle.cancel();
            }

            trace!(target: "sdtxd::proc", "detachment process completed");
            Ok(())
        };

        // build task
        let task = async move {
            tokio::select! {
                r = proc      => r,
                r = heartbeat => r,
                r = timeout   => r,
            }
        };

        // submit task
        trace!(target: "sdtxd::proc", "scheduling detachment task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }

    fn detachment_cancel_start(&mut self, handle: DtcHandle) -> Result<()> {
        // build timeout task
        let h = handle.clone();
        let timeout = self.config.handler.detach_abort.timeout * 1000.0;
        let timeout = async move {
            tokio::time::sleep(Duration::from_millis(timeout as _)).await;

            trace!(target: "sdtxd::proc", "detachment-abort timed out, canceling");
            h.timeout();

            Ok(())
        };

        // build process task
        let dir = self.config.dir.clone();
        let handler = self.config.handler.detach_abort.exec.clone();
        let proc = async move {
            trace!(target: "sdtxd::proc", "detachment-abort process started");

            // run handler if specified
            if let Some(ref path) = handler {
                debug!(target: "sdtxd::proc", ?path, ?dir, "running detachment-abort handler");

                // run handler
                let output = Command::new(path)
                    .current_dir(dir)
                    .kill_on_drop(true)
                    .output().await
                    .context("Subprocess error (detachment-abort)")?;

                // log output
                output.log("detachment-abort handler");

            } else {
                debug!(target: "sdtxd::proc", "no detachment-abort handler specified, skipping");
            };

            trace!(target: "sdtxd::proc", "detachment-abort process completed");
            handle.complete();

            Ok(())
        };

        // build task
        let task = async move {
            tokio::select! {
                r = proc      => r,
                r = timeout   => r,
            }
        };

        // submit task
        trace!(target: "sdtxd::proc", "scheduling detachment-abort task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }

    fn attachment_start(&mut self, handle: AtHandle) -> Result<()> {
        // build timeout task
        let h = handle.clone();
        let timeout = self.config.handler.attach.timeout * 1000.0;
        let timeout = async move {
            tokio::time::sleep(Duration::from_millis(timeout as _)).await;

            trace!(target: "sdtxd::proc", "detachment-abort timed out, canceling");
            h.timeout();

            Ok(())
        };

        // build process task
        let dir = self.config.dir.clone();
        let handler = self.config.handler.attach.exec.clone();
        let proc = async move {
            trace!(target: "sdtxd::proc", "attachment process started");

            // run handler if specified
            if let Some(ref path) = handler {
                debug!(target: "sdtxd::proc", ?path, ?dir, "running attachment handler");

                // run handler
                let output = Command::new(path)
                    .current_dir(dir)
                    .kill_on_drop(true)
                    .output().await
                    .context("Subprocess error (attachment)")?;

                // log output
                output.log("attachment handler");

            } else {
                debug!(target: "sdtxd::proc", "no attachment handler specified, skipping");
            };

            trace!(target: "sdtxd::proc", "attachment process completed");
            handle.complete();

            Ok(())
        };

        // build task
        let delay = Duration::from_millis((self.config.handler.attach.delay * 1000.0) as _);
        let task = async move {
            // delay to ensure all devices are set up
            debug!(target: "sdtxd::proc", "delaying attachment process by {}ms", delay.as_millis());
            tokio::time::sleep(delay).await;

            // drive main tasks
            tokio::select! {
                r = proc      => r,
                r = timeout   => r,
            }
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
                    Ok(str) => {
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
