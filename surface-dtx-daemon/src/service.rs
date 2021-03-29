use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use dbus::channel::Sender;
use dbus::nonblock::SyncConnection;
use dbus_crossroads::{Crossroads, IfaceBuilder, MethodErr};

use sdtx::DeviceMode;
use sdtx_tokio::Device;

use slog::{debug, Logger};


pub struct Service {
    log: Logger,
    conn: Arc<SyncConnection>,
    device: Device,
    mode: Mutex<DeviceMode>,
}

impl Service {
    pub fn new(log: Logger, conn: Arc<SyncConnection>, device: Device) -> Arc<Self> {
        let service = Service {
            log,
            conn,
            device,
            mode: Mutex::new(DeviceMode::Laptop),
        };

        Arc::new(service)
    }

    pub async fn request_name(&self) -> Result<()> {
        self.conn.request_name("org.surface.dtx", false, true, false).await
            .context("Failed to set up D-Bus service")
            .map(|_| ())
    }

    pub fn register(self: &Arc<Self>, cr: &mut Crossroads) -> Result<()> {
        let iface_token = cr.register("org.surface.dtx", |b: &mut IfaceBuilder<Arc<Service>>| {
            // device-mode property
            b.property("DeviceMode")
                .emits_changed_true()
                .get(|_, service| { Ok(format!("{}", service.mode.lock().unwrap()).to_lowercase()) });

            // request method
            b.method("Request", (), (), move |_ctx, service, _args: ()| {
                match service.device.latch_request() {
                    Ok(()) => { Ok(()) },
                    Err(e) => { Err(MethodErr::failed(&e)) },
                }
            });
        });

        cr.insert("/org/surface/dtx", &[iface_token], self.clone());
        Ok(())
    }

    pub fn unregister(self: &Arc<Self>, cr: &mut Crossroads) {
        let _ : Option<Arc<Service>> = cr.remove(&"/org/surface/dtx".into());
    }

    pub fn set_device_mode(&self, new: DeviceMode) {
        let old = {
            let mut mode = self.mode.lock().unwrap();
            std::mem::replace(&mut *mode, new)
        };

        debug!(self.log, "service: changing device mode"; "old" => %old, "new" => %new);

        // signal property changed
        if old != new {
            use dbus::arg::{Variant, RefArg};
            use dbus::message::SignalArgs;
            use dbus::ffidisp::stdintf::org_freedesktop_dbus as dbffi;
            use dbffi::PropertiesPropertiesChanged as PropertiesChanged;

            let mut changed: HashMap<String, Variant<Box<dyn RefArg>>> = HashMap::new();
            changed.insert("DeviceMode".into(), Variant(Box::new(format!("{}", new).to_lowercase())));

            let changed = PropertiesChanged {
                interface_name: "org.surface.dtx".into(),
                changed_properties: changed,
                invalidated_properties: Vec::new(),
            };

            let msg = changed.to_emit_message(&"/org/surface/dtx".into());

            // send will only fail due to lack of memory
            self.conn.send(msg).unwrap();
        }
    }
}
