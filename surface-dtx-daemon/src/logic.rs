#![allow(unused)]

use crate::Task;
use crate::config::Config;
use crate::service::Service;

use std::convert::TryFrom;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use futures::prelude::*;

use sdtx::{BaseState, DeviceMode, Event, event, HardwareError};
use sdtx_tokio::Device;

use slog::{debug, error, trace, warn, Logger};

use tokio::sync::mpsc::Sender;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeState {
    Ready,
    _TODO,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LatchStatus {
    Closed,
    Opened,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
    base: BaseState,
    latch: LatchStatus,
    rt: RuntimeState,
}

impl State {
    fn init() -> Self {
        State {
            base: BaseState::Attached,
            latch: LatchStatus::Closed,
            rt: RuntimeState::Ready,
        }
    }
}


pub struct EventHandler {
    log: Logger,
    config: Config,
    device: Device,
    service: Arc<Service>,
    task_queue_tx: Sender<Task>,
    state: State,
}

impl EventHandler {
    pub fn new(log: Logger, config: Config, service: Arc<Service>, device: Device,
               task_queue_tx: Sender<Task>)
        -> Self
    {
        EventHandler {
            log,
            config,
            device,
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
            sdtx::LatchStatus::Closed => LatchStatus::Closed,
            sdtx::LatchStatus::Opened => LatchStatus::Opened,
            sdtx::LatchStatus::Error(err) => Err(err).context("DTX hardware error")?,
        };

        self.state.base = base;
        self.state.latch = latch;
        self.state.rt = RuntimeState::Ready;

        self.service.set_device_mode(mode);

        // handle events
        while let Some(evt) = events.next().await {
            self.handle(evt.context("DTX device error")?).await?;
        }

        Ok(())
    }

    pub async fn handle(&mut self, evt: Event) -> Result<()> {
        trace!(self.log, "received event"; "event" => ?evt);

        match evt {
            Event::Request                      => self.on_request().await?,
            Event::Cancel { reason }            => self.on_cancel(reason).await?,
            Event::BaseConnection { state, .. } => self.on_base_state(state).await?,
            Event::LatchStatus { status }       => self.on_latch_status(status).await?,
            Event::DeviceMode { mode }          => self.on_device_mode(mode).await?,
            Event::Unknown { code, data } => {
                warn!(self.log, "unhandled event"; "code" => code, "data" => ?data);
            },
        }

        Ok(())
    }

    async fn on_request(&mut self) -> Result<()> {
        debug!(self.log, "request received");

        todo!("handle request events")
    }

    async fn on_cancel(&mut self, reason: event::CancelReason) -> Result<()> {
        debug!(self.log, "cancel event received"; "reason" => ?reason);

        todo!("handle cancel events")
    }

    async fn on_base_state(&mut self, state: event::BaseState) -> Result<()> {
        debug!(self.log, "base connection changed"; "state" => ?state);

        // translate state, warn and return on errors
        let state = match state {
            event::BaseState::Attached    => BaseState::Attached,
            event::BaseState::Detached    => BaseState::Detached,
            event::BaseState::NotFeasible => BaseState::NotFeasible,
            event::BaseState::Unknown(x) => {
                error!(self.log, "unknown base state"; "state" => x);
                return Ok(());
            },
        };

        // update state, return if it hasn't changed
        if self.state.base == state {
            return Ok(());
        }

        todo!("handle base state events");

        self.state.base = state;

        Ok(())
    }

    async fn on_latch_status(&mut self, status: event::LatchStatus) -> Result<()> {
        debug!(self.log, "latch status changed"; "status" => ?status);

        // translate state, warn and return on errors
        let status = match status {
            event::LatchStatus::Closed => LatchStatus::Closed,
            event::LatchStatus::Opened => LatchStatus::Opened,
            event::LatchStatus::Error(err) => {
                error!(self.log, "latch status error"; "error" => %err);

                // try to read latch status via ioctl, maybe we get an updated non-error state;
                // otherwise try to infer actual state
                let status = self.device.get_latch_status().context("DTX device error")?;
                let status = match status {
                    sdtx::LatchStatus::Closed                                   => LatchStatus::Closed,
                    sdtx::LatchStatus::Opened                                   => LatchStatus::Opened,
                    sdtx::LatchStatus::Error(HardwareError::FailedToOpen)       => LatchStatus::Closed,
                    sdtx::LatchStatus::Error(HardwareError::FailedToRemainOpen) => LatchStatus::Closed,
                    sdtx::LatchStatus::Error(HardwareError::FailedToClose)      => LatchStatus::Opened,
                    sdtx::LatchStatus::Error(HardwareError::Unknown(_))         => return Ok(()),
                };

                debug!(self.log, "latch status updated"; "status" => ?status);

                // TODO: forward error to user-space via service

                status
            },
            event::LatchStatus::Unknown(x) => {
                error!(self.log, "unknown latch status"; "status" => x);
                return Ok(());
            },
        };

        // update state, return if it hasn't changed
        if self.state.latch == status {
            return Ok(());
        }

        todo!("handle latch status events");

        self.state.latch = status;

        Ok(())
    }

    async fn on_device_mode(&mut self, mode: event::DeviceMode) -> Result<()> {
        debug!(self.log, "device mode changed"; "mode" => ?mode);

        if let event::DeviceMode::Unknown(mode) = mode {
            error!(self.log, "unknown device mode"; "mode" => mode);
            return Ok(());
        }

        let mode = DeviceMode::try_from(mode).unwrap();
        self.service.set_device_mode(mode);

        Ok(())
    }
}
