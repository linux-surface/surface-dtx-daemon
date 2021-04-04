use crate::config::Config;
use crate::logic::{Adapter, BaseInfo, CancelReason, DeviceMode, LatchState};
use crate::tq::TaskSender;

use std::sync::{Arc, Mutex};

use anyhow::{Context, Error, Result};

use sdtx_tokio::Device;

use tracing::{debug, info, trace};


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeState {
    Ready,
    Detaching,
    Aborting,
    Attaching,
}

#[allow(unused)]
pub struct DebugAdapter {
    config: Config,
    device: Arc<Device>,
    queue: TaskSender<Error>,
    state: Arc<Mutex<RuntimeState>>,
}

impl DebugAdapter {
    pub fn new(config: Config, device: Arc<Device>, queue: TaskSender<Error>) -> Self {
        Self {
            config,
            device,
            queue,
            state: Arc::new(Mutex::new(RuntimeState::Ready)),
        }
    }
}

impl Adapter for DebugAdapter {
    fn set_state(&mut self, _mode: DeviceMode, _base: BaseInfo, _latch: LatchState) {
        *self.state.lock().unwrap() = RuntimeState::Ready;
    }

    fn detachment_start(&mut self) -> Result<()> {
        // additional checks (e.g. dGPU usage) could be added here

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

        let state = self.state.clone();
        let device = self.device.clone();
        let task = async move {
            // TODO: properly implement detachment process

            info!(target: "sdtxd::proc", "detachment process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!(target: "sdtxd::proc", "detachment process: done");

            // state will be changed by either detachment_complete or detachment_cancel

            if *state.lock().unwrap() == RuntimeState::Detaching {
                device.latch_confirm().context("DTX device error")
            } else {    // detachment has been canceled
                Ok(())
            }
        };

        trace!(target: "sdtxd::proc", "scheduling detachment task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }

    fn detachment_cancel(&mut self, _reason: CancelReason) -> Result<()> {
        // We might have canceled in detachment_start() while the EC itself was
        // ready for detachment again. This will lead to a detachment_cancel()
        // call while we are already aborting. Make sure that we're only
        // scheduling the abort task once.
        {
            let mut state = self.state.lock().unwrap();

            if *state != RuntimeState::Detaching {
                return Ok(());
            }

            *state = RuntimeState::Aborting;
        }

        let state = self.state.clone();
        let task = async move {
            // TODO: properly implement detachment-abort process

            info!(target: "sdtxd::proc", "abort process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!(target: "sdtxd::proc", "abort process: done");

            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

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
        *self.state.lock().unwrap() = RuntimeState::Attaching;

        let state = self.state.clone();
        let task = async move {
            // TODO: properly implement attachment process

            info!(target: "sdtxd::proc", "attachment process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!(target: "sdtxd::proc", "attachment process: done");

            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

        trace!(target: "sdtxd::proc", "scheduling attachment task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }
}
