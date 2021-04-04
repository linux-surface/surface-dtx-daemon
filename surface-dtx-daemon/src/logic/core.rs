use crate::logic::{
    BaseInfo,
    BaseState,
    CancelReason,
    DeviceMode,
    DeviceType,
    HardwareError,
    LatchState,
    RuntimeError,
};

use std::convert::TryFrom;
use std::sync::Arc;

use anyhow::{Context, Result};

use futures::prelude::*;

use sdtx::event;
use sdtx::LatchStatus;
use sdtx_tokio::Device;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use tracing::{debug, error, trace, warn};


#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Request,

    DetachConfirm,

    DetachCancel,

    Attached,

    Cancel {
        reason: event::CancelReason,
    },

    BaseConnection {
        state: event::BaseState,
        device_type: DeviceType,
        id: u8,
    },

    LatchStatus {
        status: event::LatchStatus,
    },

    DeviceMode {
        mode: event::DeviceMode,
    },

    Unknown {
        code: u16,
        data: Vec<u8>,
    },
}

impl From<sdtx::Event> for Event {
    fn from(event: sdtx::Event) -> Self {
        match event {
            sdtx::Event::Request => {
                Self::Request
            },
            sdtx::Event::Cancel { reason } => {
                Self::Cancel { reason }
            },
            sdtx::Event::BaseConnection { state, device_type, id } => {
                Self::BaseConnection { state, device_type, id }
            },
            sdtx::Event::LatchStatus { status } => {
                Self::LatchStatus { status }
            },
            sdtx::Event::DeviceMode { mode } => {
                Self::DeviceMode { mode }
            },
            sdtx::Event::Unknown { code, data } => {
                Self::Unknown { code, data }
            },
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EcState {
    Ready,          // ready for new detachment request
    InProgress,     // detachment in progress, waiting for confirmation or cancellation
    Confirmed,      // detachment in progress and confirmed
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
    inject_rx: UnboundedReceiver<Event>,
    inject_tx: UnboundedSender<Event>,
    state: CoreState,
    adapter: A,
}

impl<A: Adapter> Core<A> {
    pub fn new(device: Device, adapter: A) -> Self {
        let state = CoreState {
            base: BaseState::Attached,
            latch: LatchState::Closed,
            ec: EcState::Ready,
            needs_attachment: false,
        };

        let device = Arc::new(device);
        let (inject_tx, inject_rx) = tokio::sync::mpsc::unbounded_channel();

        Self { device, inject_rx, inject_tx, state, adapter }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut evdev = Device::from(self.device.file().try_clone().await?);

        // enable events
        trace!(target: "sdtxd::core", "enabling events");

        let mut events = evdev.events_async()
            .context("DTX device error")?
            .map(|r| r.map(Event::from));

        // Update our state before we start handling events but after we've
        // enabled them. This way, we can ensure that we don't miss any
        // events/changes and accidentally set a stale state.
        trace!(target: "sdtxd::core", "updating state");

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
            LatchState::Opened => EcState::Confirmed,
        };

        self.state.base = base.state;
        self.state.latch = latch;
        self.state.ec = ec;

        self.adapter.set_state(mode, base, latch);

        // handle events
        trace!(target: "sdtxd::core", "running event loop");
        loop {
            let event = tokio::select! {
                event = self.inject_rx.recv() => event,
                event = events.next() => {
                    event.map_or(Ok(None), |r| r.map(Some))
                        .context("DTX device error")?
                },
            };

            if let Some(event) = event {
                self.handle(event)?;
            } else {
                break;
            }
        }

        Ok(())
    }

    fn handle(&mut self, event: Event) -> Result<()> {
        trace!(target: "sdtxd::core", ?event, "received event");

        match event {
            Event::Request => {
                self.on_request()
            },
            Event::DetachConfirm => {
                self.on_detach_confirm()
            },
            Event::DetachCancel => {
                self.on_detach_cancel()
            },
            Event::Attached => {
                self.on_attached()
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
                warn!(target: "sdtxd::core", code, ?data, "unhandled event");
                Ok(())
            },
        }
    }

    fn on_request(&mut self) -> Result<()> {
        // handle cancellation signals
        if self.state.ec != EcState::Ready {
            // reset EC state and abort if the latch is closed; if latch is
            // open or has been requested to be opened, this will be done on
            // the "closed" event
            if self.state.latch == LatchState::Opened || self.state.ec == EcState::Confirmed {
                debug!(target: "sdtxd::core", "request: deferring cancellation until latch closes");
            } else {
                debug!(target: "sdtxd::core", "request: canceling current request");

                self.state.ec = EcState::Ready;
                self.adapter.detachment_cancel(CancelReason::UserRequest)?;
            }

            return Ok(());
        }

        // if this request is not for cancellation, mark us as in-progress
        self.state.ec = EcState::InProgress;

        // if no base is attached (or not-feasible), cancel
        if self.state.base != BaseState::Attached {
            self.device.latch_cancel().context("DTX device error")?;

            let reason = match self.state.base {
                BaseState::NotFeasible => {
                    debug!(target: "sdtxd::core", "request: detachment not feasible, low battery");
                    CancelReason::Runtime(RuntimeError::NotFeasible)
                },
                BaseState::Detached => {
                    debug!(target: "sdtxd::core", "request: detachment not feasible, no base attached");
                    CancelReason::Runtime(RuntimeError::NotAttached)
                },
                BaseState::Attached => unreachable!("possibility already checked"),
            };

            // notify adapter
            return self.adapter.request_canceled(reason);
        }

        // commence detachment
        debug!(target: "sdtxd::core", "detachment requested");

        let handle = DtHandle { device: self.device.clone(), inject: self.inject_tx.clone() };
        self.adapter.detachment_start(handle)
    }

    fn on_detach_confirm(&mut self) -> Result<()> {
        // internal event, sent by adapter when confirming latch open

        if self.state.ec != EcState::InProgress {
            debug!(target: "sdtxd::core", "logic error: confirmation sent while no detachment in progress");
            return Ok(());
        }

        debug!(target: "sdtxd::core", "confirming detachment");
        self.state.ec = EcState::Confirmed;

        self.device.latch_confirm().context("DTX device error")
    }

    fn on_detach_cancel(&mut self) -> Result<()> {
        // internal event, sent by adapter when confirming latch open

        if self.state.ec != EcState::InProgress {
            debug!(target: "sdtxd::core", "logic error: cancellation sent while no detachment in progress");
            return Ok(());
        }

        debug!(target: "sdtxd::core", "canceling detachment");
        self.device.latch_cancel().context("DTX device error")
    }

    fn on_attached(&mut self) -> Result<()> {
        // internal event, sent by adapter when attachment is completed
        debug!(target: "sdtxd::core", "attachment complete");
        self.adapter.attachment_complete()
    }

    fn on_cancel(&mut self, reason: event::CancelReason) -> Result<()> {
        let reason = CancelReason::from(reason);

        match self.state.ec {
            EcState::Ready => {                             // no detachment in progress
                debug!(target: "sdtxd::core", %reason, "cancel: detachment prevented");

                // forward to adapter
                self.adapter.request_canceled(reason)
            },
            EcState::InProgress | EcState::Confirmed => {   // detachment in progress
                debug!(target: "sdtxd::core", %reason, "cancel: detachment canceled");

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
                error!(target: "sdtxd::core", state, "base: unknown base state");
                return Ok(());
            },
        };

        // update state, return if it hasn't changed
        if self.state.base == state {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.state.base, state);

        debug!(target: "sdtxd::core", ?state, ?ty, id, "base: state changed");

        // fowrard to adapter
        self.adapter.on_base_state(state, ty, id)?;

        // handle actual transition
        match (old, state) {
            (_, BaseState::Detached) => {       // disconnected
                if self.state.latch == LatchState::Closed {
                    // If the latch is closed, we don't expect any disconnect.
                    // This is either the user forcefully removing the
                    // clipboard, or incorrect reporting from the EC.
                    error!(target: "sdtxd::core", "unexpected disconnect: latch is closed");

                    self.adapter.detachment_unexpected()

                } else if self.state.ec == EcState::Ready {
                    // If the latch is open, we expect the EC state to be
                    // in-progress or confirmed. This is either a logic error
                    // or incorrect reporting from the EC.
                    error!(target: "sdtxd::core", "unexpected disconnect: detachment not \
                           in-progress but latch is open");

                    self.adapter.detachment_unexpected()
                } else {
                    Ok(())
                }
            },
            (BaseState::Detached, _) => {       // connected
                // if latch is closed, start attachment process, otherwise wait
                // for latch to close before starting that
                match self.state.latch {
                    LatchState::Closed => {
                        debug!(target: "sdtxd::core", "base attached, starting attachment process");

                        self.state.needs_attachment = false;

                        let handle = AtHandle { inject: self.inject_tx.clone() };
                        self.adapter.attachment_start(handle)
                    },
                    LatchState::Opened => {
                        debug!(target: "sdtxd::core", "base attached, deferring attachment");

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

                error!(target: "sdtxd::core", %error, "latch: status error");

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

                debug!(target: "sdtxd::core", ?status, "latch: status inferred after error");

                // forward error to adapter
                self.adapter.on_latch_status(LatchStatus::Error(error))?;

                status
            },
            event::LatchStatus::Unknown(status) => {
                error!(target: "sdtxd::core", status, "latch: unknown latch status");
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

        debug!(target: "sdtxd::core", ?status, "latch: status changed");

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
            debug!(target: "sdtxd::core", "detachment completed via latch close");

            self.adapter.detachment_complete()

        } else if !self.state.needs_attachment {
            // The latch has been opened and closed without the tablet being
            // detached. This is either due to the latch-close timeout or
            // (accelerated by) the user pressing the request button again.
            //
            // It might be possible that we have already canceled the
            // detachment procedure via a cancel event. Only tell the adapter
            // if we haven't done so yet.
            if ec != EcState::Ready {
                debug!(target: "sdtxd::core", "detachment canceled via latch close");

                self.adapter.detachment_cancel(CancelReason::UserRequest)
            } else {
                debug!(target: "sdtxd::core", "detachment already canceled before latch closed");
                Ok(())
            }

        } else {
            // The latch has been opened and before it has been closed again
            // (signalled by this event), the tablet has been detached and
            // re-attached. Complete the detachment procedure and notify the
            // adapter that an attachmend has occured.
            debug!(target: "sdtxd::core", "detachment completed via latch close");
            self.adapter.detachment_complete()?;

            debug!(target: "sdtxd::core", "running deferred attachment process now");
            self.state.needs_attachment = false;

            let handle = AtHandle { inject: self.inject_tx.clone() };
            self.adapter.attachment_start(handle)
        }
    }

    fn on_device_mode(&mut self, mode: event::DeviceMode) -> Result<()> {
        if let event::DeviceMode::Unknown(mode) = mode {
            error!(target: "sdtxd::core", mode, "mode: unknown device mode");
            return Ok(());
        }
        let mode = DeviceMode::try_from(mode).unwrap();

        debug!(target: "sdtxd::core", ?mode, "mode: device mode changed");

        self.adapter.on_device_mode(mode)
    }
}


#[derive(Clone)]
pub struct DtHandle {
    device: Arc<Device>,
    inject: UnboundedSender<Event>,
}

impl DtHandle {
    pub fn confirm(&self) {
        let _ = self.inject.send(Event::DetachConfirm);
    }

    pub fn cancel(&self) {
        let _ = self.inject.send(Event::DetachCancel);
    }
}


#[derive(Clone)]
pub struct AtHandle {
    inject: UnboundedSender<Event>,
}

impl AtHandle {
    pub fn complete(&self) {
        let _ = self.inject.send(Event::Attached);
    }
}


#[allow(unused)]
pub trait Adapter {
    fn set_state(&mut self, mode: DeviceMode, base: BaseInfo, latch: LatchState) { }

    fn request_canceled(&mut self, reason: CancelReason) -> Result<()> {
        Ok(())
    }

    fn detachment_start(&mut self, handle: DtHandle) -> Result<()> {
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

    fn attachment_start(&mut self, handle: AtHandle) -> Result<()> {
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

            fn detachment_start(&mut self, handle: DtHandle) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.detachment_start(handle.clone())?,)+);
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

            fn attachment_start(&mut self, handle: AtHandle) -> Result<()> {
                let ($($name,)+) = self;
                ($($name.attachment_start(handle.clone())?,)+);
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
