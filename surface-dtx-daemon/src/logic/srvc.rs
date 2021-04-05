use crate::logic::{Adapter, DeviceMode};
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
    fn on_device_mode(&mut self, mode: DeviceMode) -> Result<()> {
        self.service.set_device_mode(mode);
        Ok(())
    }
}
