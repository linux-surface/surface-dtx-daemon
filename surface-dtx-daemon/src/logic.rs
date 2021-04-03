use crate::config::Config;
use crate::tq::TaskSender;

use std::convert::TryFrom;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Error, Result};

use futures::prelude::*;

use sdtx::{BaseInfo, BaseState, DeviceMode, DeviceType, Event, HardwareError, LatchStatus, event};
use sdtx_tokio::Device;

use tracing::{debug, error, info, trace, warn};


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    NotAttached,
    NotFeasible,
    Timeout,
    Unknown(u8),
}

impl From<sdtx::RuntimeError> for RuntimeError {
    fn from(err: sdtx::RuntimeError) -> Self {
        match err {
            sdtx::RuntimeError::NotFeasible => Self::NotFeasible,
            sdtx::RuntimeError::Timeout     => Self::Timeout,
            sdtx::RuntimeError::Unknown(x)  => Self::Unknown(x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    UserRequest,    // user or higher layer requested cancelation, or user did not act
    Runtime(RuntimeError),
    Hardware(HardwareError),
    Unknown(u16),
}

impl From<event::CancelReason> for CancelReason {
    fn from(reason: event::CancelReason) -> Self {
        match reason {
            event::CancelReason::Runtime(e)  => Self::Runtime(RuntimeError::from(e)),
            event::CancelReason::Hardware(e) => Self::Hardware(e),
            event::CancelReason::Unknown(x)  => Self::Unknown(x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatchState {
    Closed,
    Opened,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EcState {
    Ready,
    InProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CoreState {
    base: BaseState,
    latch: LatchState,
    ec: EcState,
    needs_attachment: bool,
}

pub struct Core<A> {
    device: Arc<Device>,
    state: CoreState,
    adapter: A,
}

impl<A: Adapter> Core<A> {
    pub fn new(device: Arc<Device>, adapter: A) -> Self {
        let state = CoreState {
            base: BaseState::Attached,
            latch: LatchState::Closed,
            ec: EcState::Ready,
            needs_attachment: false,
        };

        Self { device, state, adapter }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut evdev = Device::from(self.device.file().try_clone().await?);

        // enable events
        let mut events = evdev.events_async()
            .context("DTX device error")?;

        // Update our state before we start handling events but after we've
        // enabled them. This way, we can ensure that we don't miss any
        // events/changes and accidentally set a stale state.
        let base = self.device.get_base_info().context("DTX device error")?;
        let latch = self.device.get_latch_status().context("DTX device error")?;
        let mode = self.device.get_device_mode().context("DTX device error")?;

        let latch = match latch {
            LatchStatus::Closed => LatchState::Closed,
            LatchStatus::Opened => LatchState::Opened,
            LatchStatus::Error(err) => Err(err).context("DTX hardware error")?,
        };

        let ec = match latch {
            LatchState::Closed => EcState::Ready,
            LatchState::Opened => EcState::InProgress,
        };

        self.state.base = base.state;
        self.state.latch = latch;
        self.state.ec = ec;

        self.adapter.set_state(mode, base, latch);

        // handle events
        while let Some(evt) = events.next().await {
            self.handle(evt.context("DTX device error")?).await?;
        }

        Ok(())
    }

    async fn handle(&mut self, evt: Event) -> Result<()> {
        trace!(event=?evt, "received event");

        match evt {
            Event::Request => {
                self.on_request()
            },
            Event::Cancel { reason } => {
                self.on_cancel(reason)
            },
            Event::BaseConnection { state, device_type, id } => {
                self.on_base_state(state, device_type, id)
            },
            Event::LatchStatus { status } => {
                self.on_latch_status(status)
            },
            Event::DeviceMode { mode } => {
                self.on_device_mode(mode)
            },
            Event::Unknown { code, data } => {
                warn!(code, ?data, "unhandled event");
                Ok(())
            },
        }
    }

    fn on_request(&mut self) -> Result<()> {
        // handle cancellation signals
        if self.state.ec == EcState::InProgress {
            return match self.state.latch {
                LatchState::Opened => Ok(()),
                LatchState::Closed => {
                    // reset EC state and abort if the latch is closed; if
                    // latch is open, this will be done on the "closed" event
                    self.state.ec = EcState::Ready;
                    self.adapter.detachment_cancel(CancelReason::UserRequest)
                },
            }
        }

        // if this request is not for cancellation, mark us as in-progress
        self.state.ec = EcState::InProgress;

        // if no base is attached (or not-feasible), cancel
        if self.state.base != BaseState::Attached {
            self.device.latch_cancel().context("DTX device error")?;

            let reason = match self.state.base {
                BaseState::NotFeasible => CancelReason::Runtime(RuntimeError::NotFeasible),
                BaseState::Detached    => CancelReason::Runtime(RuntimeError::NotAttached),
                BaseState::Attached    => unreachable!("possibility already checked"),
            };

            // notify adapter
            return self.adapter.request_canceled(reason);
        }

        // commence detachment
        self.adapter.detachment_start()
    }

    fn on_cancel(&mut self, reason: event::CancelReason) -> Result<()> {
        let reason = CancelReason::from(reason);

        match self.state.ec {
            EcState::Ready => {         // no detachment in progress
                // forward to adapter
                self.adapter.request_canceled(reason)
            },
            EcState::InProgress => {    // detachment in progress
                // reset EC state
                self.state.ec = EcState::Ready;

                // cancel current detachment procedure
                self.adapter.detachment_cancel(reason)
            },
        }
    }

    fn on_base_state(&mut self, state: event::BaseState, ty: DeviceType, id: u8) -> Result<()> {
        // translate state, warn and return on errors
        let state = match state {
            event::BaseState::Attached    => BaseState::Attached,
            event::BaseState::Detached    => BaseState::Detached,
            event::BaseState::NotFeasible => BaseState::NotFeasible,
            event::BaseState::Unknown(state) => {
                error!(state, "unknown base state");
                return Ok(());
            },
        };

        // update state, return if it hasn't changed
        if self.state.base == state {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.state.base, state);

        // fowrard to adapter
        self.adapter.on_base_state(state, ty, id)?;

        // handle actual transition
        match (old, state) {
            (_, BaseState::Detached) => {       // disconnected
                if self.state.latch == LatchState::Closed {
                    // If the latch is closed, we don't expect any disconnect.
                    // This is either the user forcefully removing the
                    // clipboard, or incorrect reporting from the EC.
                    error!("unexpected disconnect: latch is closed");

                } else if self.state.ec != EcState::InProgress {
                    // If the latch is open, we expect the EC state to be
                    // in-progress. This is either a logic error or incorrect
                    // reporting from the EC.
                    error!("unexpected disconnect: detachment not in-progress but latch is open");
                }

                self.adapter.detachment_unexpected()
            },
            (BaseState::Detached, _) => {       // connected
                // if latch is closed, start attachment process, otherwise wait
                // for latch to close before starting that
                match self.state.latch {
                    LatchState::Closed => {
                        self.state.needs_attachment = false;
                        self.adapter.attachment_complete()
                    },
                    LatchState::Opened => {
                        self.state.needs_attachment = true;
                        Ok(())
                    },
                }
            },
            (_, _) => Ok(()),                   // other (attached <-> feasible)
        }
    }

    fn on_latch_status(&mut self, status: event::LatchStatus) -> Result<()> {
        // translate state, warn and return on errors
        let state = match status {
            event::LatchStatus::Closed => LatchState::Closed,
            event::LatchStatus::Opened => LatchState::Opened,
            event::LatchStatus::Error(error) => {
                use HardwareError as HwErr;

                error!(%error, "latch status error");

                // try to read latch status via ioctl, maybe we get an updated non-error state;
                // otherwise try to infer actual state
                let status = self.device.get_latch_status().context("DTX device error")?;
                let status = match status {
                    LatchStatus::Closed                           => LatchState::Closed,
                    LatchStatus::Opened                           => LatchState::Opened,
                    LatchStatus::Error(HwErr::FailedToOpen)       => LatchState::Closed,
                    LatchStatus::Error(HwErr::FailedToRemainOpen) => LatchState::Closed,
                    LatchStatus::Error(HwErr::FailedToClose)      => LatchState::Opened,
                    LatchStatus::Error(HwErr::Unknown(_))         => return Ok(()),
                };

                debug!(?status, "latch status updated");

                // forward error to adapter
                self.adapter.on_latch_status(LatchStatus::Error(error))?;

                status
            },
            event::LatchStatus::Unknown(x) => {
                error!(status=x, "unknown latch status");
                return Ok(());
            },
        };

        // reset EC state if closed
        let ec = self.state.ec;
        if state == LatchState::Closed {
            self.state.ec = EcState::Ready;
        }

        // update state, return if it hasn't changed
        if self.state.latch == state {
            return Ok(());
        }
        self.state.latch = state;

        // Fowrard to adapter: Note that we use the inferred state here in case
        // of any error. In case of errors, the adapter will get two events,
        // one with an error and one with an attempt at correcting this error.
        self.adapter.on_latch_status(match state {
            LatchState::Closed => LatchStatus::Closed,
            LatchState::Opened => LatchStatus::Opened,
        })?;

        // If latch has been opened, there's nothing left to do here. The
        // detachment procss will continue either when the base has been
        // detached or the latch has been closed again.
        if state == LatchState::Opened {
            return Ok(());
        }

        // Finish detachment process when latch has been closed.
        if self.state.base == BaseState::Detached {
            // The latch has been closed and the base is detached. This is what
            // we normally expect the detachment procedure to end with.
            self.adapter.detachment_complete()

        } else if !self.state.needs_attachment {
            // The latch has been opened and closed without the tablet being
            // detached. This is either due to the latch-close timeout or
            // (accelerated by) the user pressing the request button again.
            //
            // It might be possible that we have already canceled the
            // detachment procedure via a cancel event. Only tell the adapter
            // if we haven't done so yet.
            if ec == EcState::InProgress {
                self.adapter.detachment_cancel(CancelReason::UserRequest)
            } else {
                Ok(())
            }

        } else {
            // The latch has been opened and before it has been closed again
            // (signalled by this event), the tablet has been detached and
            // re-attached. Complete the detachment procedure and notify the
            // adapter that an attachmend has occured.
            self.adapter.detachment_complete()?;
            self.state.needs_attachment = false;
            self.adapter.attachment_complete()
        }
    }

    fn on_device_mode(&mut self, mode: event::DeviceMode) -> Result<()> {
        if let event::DeviceMode::Unknown(mode) = mode {
            error!(mode, "unknown device mode");
            return Ok(());
        }

        let mode = DeviceMode::try_from(mode).unwrap();
        self.adapter.on_device_mode(mode)
    }
}


#[allow(unused)]
pub trait Adapter {
    fn set_state(&mut self, mode: DeviceMode, base: BaseInfo, latch: LatchState) { }

    fn request_canceled(&mut self, reason: CancelReason) -> Result<()> {
        Ok(())
    }

    fn detachment_start(&mut self) -> Result<()> {
        Ok(())
    }

    fn detachment_cancel(&mut self, reason: CancelReason) -> Result<()> {
        Ok(())
    }

    fn detachment_complete(&mut self) -> Result<()> {
        Ok(())
    }

    fn detachment_unexpected(&mut self) -> Result<()> {
        Ok(())
    }

    fn attachment_complete(&mut self) -> Result<()> {
        Ok(())
    }

    fn on_base_state(&mut self, state: BaseState, ty: DeviceType, id: u8) -> Result<()> {
        Ok(())
    }

    fn on_latch_status(&mut self, status: LatchStatus) -> Result<()> {
        Ok(())
    }

    fn on_device_mode(&mut self, mode: DeviceMode) -> Result<()> {
        Ok(())
    }
}

macro_rules! impl_adapter_for_tuple {
    ( $( $name:ident )+ ) => {
        #[allow(non_snake_case)]
        impl<$($name: Adapter),+> Adapter for ($($name,)+)
        {
            fn set_state(&mut self, mode: DeviceMode, base: BaseInfo, latch: LatchState) {
                let ($($name,)+) = self;
                ($($name.set_state(mode, base, latch),)+);
            }

            fn request_canceled(&mut self, reason: CancelReason) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.request_canceled(reason)?,)+);
                Ok(())
            }

            fn detachment_start(&mut self) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.detachment_start()?,)+);
                Ok(())
            }

            fn detachment_cancel(&mut self, reason: CancelReason) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.detachment_cancel(reason)?,)+);
                Ok(())
            }

            fn detachment_complete(&mut self) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.detachment_complete()?,)+);
                Ok(())
            }

            fn detachment_unexpected(&mut self) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.detachment_unexpected()?,)+);
                Ok(())
            }

            fn attachment_complete(&mut self) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.attachment_complete()?,)+);
                Ok(())
            }

            fn on_base_state(&mut self, state: BaseState, ty: DeviceType, id: u8) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.on_base_state(state, ty, id)?,)+);
                Ok(())
            }

            fn on_latch_status(&mut self, status: LatchStatus) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.on_latch_status(status)?,)+);
                Ok(())
            }

            fn on_device_mode(&mut self, mode: DeviceMode) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.on_device_mode(mode)?,)+);
                Ok(())
            }
        }
    }
}

impl_adapter_for_tuple! { A1 }
impl_adapter_for_tuple! { A1 A2 }
impl_adapter_for_tuple! { A1 A2 A3 }


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
                trace!("request: process in progress, canceling this request");

                self.device.latch_cancel().context("DTX device error")?;
                return Ok(());
            }

            *state = RuntimeState::Detaching;
        }

        let state = self.state.clone();
        let device = self.device.clone();
        let task = async move {
            // TODO: properly implement detachment process

            info!("detachment process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!("detachment process: done");

            // state will be changed by either detachment_complete or detachment_cancel

            if *state.lock().unwrap() == RuntimeState::Detaching {
                device.latch_confirm().context("DTX device error")
            } else {    // detachment has been canceled
                Ok(())
            }
        };

        trace!("request: scheduling detachment task");
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

            info!("abort process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!("abort process: done");

            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

        trace!("request: scheduling detachment-abort task");
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

            info!("attachment process: starting");
            tokio::time::sleep(std::time::Duration::new(5, 0)).await;
            info!("attachment process: done");

            *state.lock().unwrap() = RuntimeState::Ready;

            Ok(())
        };

        trace!("request: scheduling attachment task");
        if self.queue.submit(task).is_err() {
            unreachable!("receiver dropped");
        }

        Ok(())
    }
}
