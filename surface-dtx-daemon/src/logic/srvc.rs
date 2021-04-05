use crate::logic::{
    Adapter,
    BaseInfo,
    CancelReason,
    DeviceMode,
    DtHandle,
    DtcHandle,
    LatchState,
    LatchStatus,
};
use crate::service::{ServiceHandle, Event};

use anyhow::Result;


pub struct ServiceAdapter {
    service: ServiceHandle,
}

impl ServiceAdapter {
    pub fn new(service: ServiceHandle) -> Self {
        Self { service }
    }
}

impl Adapter for ServiceAdapter {
    fn set_state(&mut self, mode: DeviceMode, base: BaseInfo, latch: LatchState) {
        self.service.set_base_info(base);
        self.service.set_latch_status(latch.into());
        self.service.set_device_mode(mode);
    }

    fn on_base_state(&mut self, info: BaseInfo) -> Result<()> {
        self.service.set_base_info(info);
        Ok(())
    }

    fn on_latch_status(&mut self, status: LatchStatus) -> Result<()> {
        self.service.set_latch_status(status);
        Ok(())
    }

    fn on_device_mode(&mut self, mode: DeviceMode) -> Result<()> {
        self.service.set_device_mode(mode);
        Ok(())
    }

    fn detachment_start(&mut self, _handle: DtHandle) -> Result<()> {
        self.service.emit_event(Event::DetachmentStart);
        Ok(())
    }

    fn detachment_complete(&mut self) -> Result<()> {
        self.service.emit_event(Event::DetachmentComplete);
        Ok(())
    }

    fn detachment_timeout(&mut self) -> Result<()> {
        self.service.emit_event(Event::DetachmentTimeout);
        Ok(())
    }

    fn detachment_cancel_start(&mut self, _handle: DtcHandle, reason: CancelReason) -> Result<()> {
        self.service.emit_event(Event::DetachmentCancelStart { reason });
        Ok(())
    }

    fn detachment_cancel_complete(&mut self) -> Result<()> {
        self.service.emit_event(Event::DetachmentCancelComplete);
        Ok(())
    }

    fn detachment_cancel_timeout(&mut self) -> Result<()> {
        self.service.emit_event(Event::DetachmentCancelTimeout);
        Ok(())
    }
}
