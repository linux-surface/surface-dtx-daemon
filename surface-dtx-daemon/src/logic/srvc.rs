use crate::logic::{
    Adapter,
    BaseInfo,
    DeviceMode,
    LatchState,
    LatchStatus,
};
use crate::service::ServiceHandle;

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
}
