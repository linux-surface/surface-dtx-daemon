#![allow(unused)]

use crate::Task;
use crate::config::Config;
use crate::service::Service;
use crate::tq::TaskSender;

use std::convert::TryFrom;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Error, Result};

use futures::prelude::*;

use sdtx::{BaseState, DeviceMode, Event, event, HardwareError};
use sdtx_tokio::Device;

use tokio::sync::mpsc::Sender;

use tracing::{debug, error, info, trace, warn};


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeState {
    Ready,
    Detaching,
    Aborting,
    Attaching,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EcState {
    Ready,
    InProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LatchState {
    Closed,
    Opened,
}

#[derive(Debug, Clone)]
struct State {
    base: BaseState,
    latch: LatchState,
    ec: EcState,
    needs_attachment: bool,
    rt: Arc<Mutex<RuntimeState>>,
}

impl State {
    fn init() -> Self {
        State {
            base: BaseState::Attached,
            latch: LatchState::Closed,
            ec: EcState::Ready,
            needs_attachment: false,
            rt: Arc::new(Mutex::new(RuntimeState::Ready)),
        }
    }
}


pub struct EventHandler {
    config: Config,
    device: Arc<Device>,
    service: Arc<Service>,
    task_queue_tx: TaskSender<Error>,
    state: State,
}

impl EventHandler {
    pub fn new(config: Config, service: Arc<Service>, device: Device,
               task_queue_tx: TaskSender<Error>)
        -> Self
    {
        EventHandler {
            config,
            device: Arc::new(device),
            service,
            task_queue_tx,
            state: State::init(),
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
        let base = self.device.get_base_info().context("DTX device error")?.state;
        let latch = self.device.get_latch_status().context("DTX device error")?;
        let mode = self.device.get_device_mode().context("DTX device error")?;

        let latch = match latch {
            sdtx::LatchStatus::Closed => LatchState::Closed,
            sdtx::LatchStatus::Opened => LatchState::Opened,
            sdtx::LatchStatus::Error(err) => Err(err).context("DTX hardware error")?,
        };

        let ec = match latch {
            LatchState::Closed => EcState::Ready,
            LatchState::Opened => EcState::InProgress,
        };

        self.state.base = base;
        self.state.latch = latch;
        self.state.ec = ec;
        *self.state.rt.lock().unwrap() = RuntimeState::Ready;

        self.service.set_device_mode(mode);

        // handle events
        while let Some(evt) = events.next().await {
            self.handle(evt.context("DTX device error")?).await?;
        }

        Ok(())
    }

    pub async fn handle(&mut self, evt: Event) -> Result<()> {
        trace!(event=?evt, "received event");

        match evt {
            Event::Request                      => self.on_request().await?,
            Event::Cancel { reason }            => self.on_cancel(reason).await?,
            Event::BaseConnection { state, .. } => self.on_base_state(state).await?,
            Event::LatchStatus { status }       => self.on_latch_status(status).await?,
            Event::DeviceMode { mode }          => self.on_device_mode(mode).await?,
            Event::Unknown { code, data }       => warn!(code, ?data, "unhandled event"),
        }

        Ok(())
    }

    async fn on_request(&mut self) -> Result<()> {
        debug!("request received");

        // handle cancellation signals
        if self.state.ec == EcState::InProgress {
            trace!("request: EC detachment in progress, treating this as cancelation");

            // reset EC state and abort if the latch is closed; if latch is
            // open, this will be done on the "closed" event
            if self.state.latch == LatchState::Closed {
                self.state.ec = EcState::Ready;
                self.detachment_cancel().await?;
            }

            return Ok(());
        }

        // if this request is not for cancellation, mark us as in-progress
        self.state.ec = EcState::InProgress;

        // if no base is attached (or not-feasible), cancel
        if self.state.base != BaseState::Attached {
            trace!("request: base not attached, canceling this request");

            self.device.latch_cancel().context("DTX device error")?;

            if self.state.base == BaseState::NotFeasible {
                // TODO: warn users via service
            }

            return Ok(());
        }

        trace!("request: core checks passed, starting detachment");
        self.detachment_start().await
    }

    async fn on_cancel(&mut self, reason: event::CancelReason) -> Result<()> {
        debug!(?reason, "cancel event received");

        // TODO: notify users?

        match self.state.ec {
            EcState::Ready => {         // no detachment in progress
                Ok(())
            },
            EcState::InProgress => {    // detachment in progress
                // reset EC state
                self.state.ec = EcState::Ready;

                // cancel current detachment procedure
                self.detachment_cancel().await
            },
        }
    }

    async fn on_base_state(&mut self, state: event::BaseState) -> Result<()> {
        debug!(?state, "base connection changed");

        // translate state, warn and return on errors
        let state = match state {
            event::BaseState::Attached    => BaseState::Attached,
            event::BaseState::Detached    => BaseState::Detached,
            event::BaseState::NotFeasible => BaseState::NotFeasible,
            event::BaseState::Unknown(x) => {
                error!(state=x, "unknown base state");
                return Ok(());
            },
        };

        // update state, return if it hasn't changed
        if self.state.base == state {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.state.base, state);

        // handle actual transition
        match (old, state) {
            (_, BaseState::Detached) => self.on_base_disconnected().await,
            (BaseState::Detached, _) => self.on_base_connected().await,
            (_, _) => Ok(()),
        }
    }

    async fn on_base_disconnected(&mut self) -> Result<()> {
        Ok(())          // TODO: notify users?
    }

    async fn on_base_connected(&mut self) -> Result<()> {
        // if latch is closed, start attachment process, otherwise wait for
        // latch to close before starting that

        match self.state.latch {
            LatchState::Closed => {
                self.state.needs_attachment = false;
                self.attachment_start().await
            },
            LatchState::Opened => {
                self.state.needs_attachment = true;
                Ok(())
            },
        }
    }

    async fn on_latch_status(&mut self, status: event::LatchStatus) -> Result<()> {
        debug!(?status, "latch status changed");

        // translate state, warn and return on errors
        let status = match status {
            event::LatchStatus::Closed => LatchState::Closed,
            event::LatchStatus::Opened => LatchState::Opened,
            event::LatchStatus::Error(error) => {
                use HardwareError as HwErr;

                error!(%error, "latch status error");

                // try to read latch status via ioctl, maybe we get an updated non-error state;
                // otherwise try to infer actual state
                let status = self.device.get_latch_status().context("DTX device error")?;
                let status = match status {
                    sdtx::LatchStatus::Closed                           => LatchState::Closed,
                    sdtx::LatchStatus::Opened                           => LatchState::Opened,
                    sdtx::LatchStatus::Error(HwErr::FailedToOpen)       => LatchState::Closed,
                    sdtx::LatchStatus::Error(HwErr::FailedToRemainOpen) => LatchState::Closed,
                    sdtx::LatchStatus::Error(HwErr::FailedToClose)      => LatchState::Opened,
                    sdtx::LatchStatus::Error(HwErr::Unknown(_))         => return Ok(()),
                };

                debug!(?status, "latch status updated");

                // TODO: forward error to user-space via service

                status
            },
            event::LatchStatus::Unknown(x) => {
                error!(status=x, "unknown latch status");
                return Ok(());
            },
        };

        // reset EC state if closed
        if status == LatchState::Closed {
            self.state.ec = EcState::Ready;
        }

        // update state, return if it hasn't changed
        if self.state.latch == status {
            return Ok(());
        }
        self.state.latch = status;

        // handle actual transition
        match status {
            LatchState::Opened => self.on_latch_opened().await,
            LatchState::Closed => self.on_latch_closed().await,
        }
    }

    async fn on_latch_opened(&mut self) -> Result<()> {
        Ok(())          // TODO: notify users that base can be detached
    }

    async fn on_latch_closed(&mut self) -> Result<()> {
        // TODO: notify users

        if self.state.base == BaseState::Detached {
            self.detachment_complete().await
        } else if !self.state.needs_attachment {
            self.detachment_cancel().await
        } else {
            self.state.needs_attachment = false;
            self.detachment_complete().await?;
            self.attachment_start().await
        }
    }

    async fn on_device_mode(&mut self, mode: event::DeviceMode) -> Result<()> {
        debug!(?mode, "device mode changed");

        if let event::DeviceMode::Unknown(mode) = mode {
            error!(mode, "unknown device mode");
            return Ok(());
        }

        let mode = DeviceMode::try_from(mode).unwrap();
        self.service.set_device_mode(mode);

        Ok(())
    }

    async fn detachment_start(&mut self) -> Result<()> {
        // additional checks (e.g. dGPU usage) could be added here

        {
            let mut rt_state = self.state.rt.lock().unwrap();

            // if any subprocess is running (attach/abort), cancel the (new) request
            if *rt_state != RuntimeState::Ready {
                trace!("request: process in progress, canceling this request");

                self.device.latch_cancel().context("DTX device error")?;
                return Ok(());
            }

            *rt_state = RuntimeState::Detaching;
        }

        let state = self.state.rt.clone();
        let device = self.device.clone();
        let task = async move {
            // TODO: properly implement detachment process

            info!("detachment process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!("detachment process: done");

            // state will be changed by either detachment_complete or detachment_cancel

            Ok(())
        };

        trace!("request: scheduling detachment task");
        if self.task_queue_tx.submit(task).await.is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }

    async fn detachment_cancel(&mut self) -> Result<()> {
        // We might have canceled in detachment_start() while the EC itself was
        // ready for detachment again. This will lead to a detachment_cancel()
        // call while we are already aborting. Make sure that we're only
        // scheduling the abort task once.
        {
            let mut state = self.state.rt.lock().unwrap();

            if *state != RuntimeState::Detaching {
                return Ok(());
            }

            *state = RuntimeState::Aborting;
        }

        let state = self.state.rt.clone();
        let task = async move {
            // TODO: properly implement detachment-abort process

            info!("abort process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!("abort process: done");

            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

        trace!("request: scheduling detachment-abort task");
        if self.task_queue_tx.submit(task).await.is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }

    async fn detachment_complete(&mut self) -> Result<()> {
        *self.state.rt.lock().unwrap() = RuntimeState::Ready;
        Ok(())  // TODO: notify users?
    }

    async fn attachment_start(&mut self) -> Result<()> {
        *self.state.rt.lock().unwrap() = RuntimeState::Attaching;

        let state = self.state.rt.clone();
        let task = async move {
            // TODO: properly implement attachment process

            info!("attachment process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!("attachment process: done");

            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

        trace!("request: scheduling attachment task");
        if self.task_queue_tx.submit(task).await.is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }
}
