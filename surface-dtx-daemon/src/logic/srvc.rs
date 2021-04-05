use crate::logic::{Adapter, DeviceMode};
use crate::service::Service;

use std::sync::Arc;

use anyhow::Result;


pub struct ServiceAdapter {
    service: Arc<Service>,
}

impl ServiceAdapter {
    pub fn new(service: Arc<Service>) -> Self {
        Self { service }
    }
}

impl Adapter for ServiceAdapter {
    fn on_device_mode(&mut self, mode: DeviceMode) -> Result<()> {
        self.service.set_device_mode(mode);
        Ok(())
    }
}
