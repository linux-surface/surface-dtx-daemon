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
use sdtx::{Event, LatchStatus};
use sdtx_tokio::Device;

use tracing::{debug, error, trace, warn};


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
        trace!(target: "sdtxd::core", "enabling events");

        let mut events = evdev.events_async()
            .context("DTX device error")?;

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
            LatchState::Opened => EcState::InProgress,
        };

        self.state.base = base.state;
        self.state.latch = latch;
        self.state.ec = ec;

        self.adapter.set_state(mode, base, latch);

        // handle events
        trace!(target: "sdtxd::core", "running event loop");
        while let Some(evt) = events.next().await {
            self.handle(evt.context("DTX device error")?)?;
        }

        Ok(())
    }

    fn handle(&mut self, event: Event) -> Result<()> {
        trace!(target: "sdtxd::core", ?event, "received event");

        match event {
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
                warn!(target: "sdtxd::core", code, ?data, "unhandled event");
                Ok(())
            },
        }
    }

    fn on_request(&mut self) -> Result<()> {
        // handle cancellation signals
        if self.state.ec == EcState::InProgress {
            // reset EC state and abort if the latch is closed; if latch is
            // open, this will be done on the "closed" event
            return match self.state.latch {
                LatchState::Opened => {
                    debug!(target: "sdtxd::core", "request: deferring cancellation until latch closes");
                    Ok(())
                },
                LatchState::Closed => {
                    debug!(target: "sdtxd::core", "request: canceling current request");

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

        self.adapter.detachment_start()
    }

    fn on_cancel(&mut self, reason: event::CancelReason) -> Result<()> {
        let reason = CancelReason::from(reason);

        match self.state.ec {
            EcState::Ready => {         // no detachment in progress
                debug!(target: "sdtxd::core", %reason, "cancel: detachment prevented");

                // forward to adapter
                self.adapter.request_canceled(reason)
            },
            EcState::InProgress => {    // detachment in progress
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

                } else if self.state.ec != EcState::InProgress {
                    // If the latch is open, we expect the EC state to be
                    // in-progress. This is either a logic error or incorrect
                    // reporting from the EC.
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
                        self.adapter.attachment_complete()
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
            if ec == EcState::InProgress {
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
            self.adapter.attachment_complete()
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
