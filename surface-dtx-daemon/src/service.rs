use crate::logic::DeviceMode;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use dbus::{arg::Variant, channel::Sender};
use dbus::nonblock::SyncConnection;
use dbus_crossroads::{Crossroads, IfaceBuilder, MethodErr};

use sdtx_tokio::Device;

use tracing::debug;


pub struct Service {
    conn: Arc<SyncConnection>,
    device: Device,
    mode: Mutex<DeviceMode>,
}

impl Service {
    const NAME: &'static str = "/org/surface/dtx";
    const INTERFACE: &'static str = "org.surface.dtx";

    pub fn new(conn: Arc<SyncConnection>, device: Device) -> Arc<Self> {
        let service = Service {
            conn,
            device,
            mode: Mutex::new(DeviceMode::Laptop),
        };

        Arc::new(service)
    }

    pub async fn request_name(&self) -> Result<()> {
        self.conn.request_name(Self::INTERFACE, false, true, false).await
            .context("Failed to set up D-Bus service")
            .map(|_| ())
    }

    pub fn register(self: &Arc<Self>, cr: &mut Crossroads) -> Result<()> {
        let iface_token = cr.register(Self::INTERFACE, |b: &mut IfaceBuilder<Arc<Service>>| {
            // device-mode property
            b.property("DeviceMode")
                .emits_changed_true()
                .get(|_, service| { Ok(service.mode.lock().unwrap().as_string()) });

            // request method
            b.method("Request", (), (), move |_ctx, service, _args: ()| {
                match service.device.latch_request() {
                    Ok(()) => { Ok(()) },
                    Err(e) => { Err(MethodErr::failed(&e)) },
                }
            });
        });

        cr.insert(Self::NAME, &[iface_token], self.clone());
        Ok(())
    }

    pub fn unregister(self: &Arc<Self>, cr: &mut Crossroads) {
        let _ : Option<Arc<Service>> = cr.remove(&Self::NAME.into());
    }

    #[allow(unused)]
    pub fn set_device_mode(&self, new: DeviceMode) {
        let old = {
            let mut mode = self.mode.lock().unwrap();
            std::mem::replace(&mut *mode, new)
        };

        debug!(%old, %new, "changing device mode");

        // signal property changed
        if old != new {
            use dbus::arg::RefArg;
            use dbus::message::SignalArgs;
            use dbus::ffidisp::stdintf::org_freedesktop_dbus as dbffi;
            use dbffi::PropertiesPropertiesChanged as PropertiesChanged;

            let mut changed: HashMap<String, Variant<Box<dyn RefArg>>> = HashMap::new();
            changed.insert("DeviceMode".into(), new.as_variant());

            let changed = PropertiesChanged {
                interface_name: Self::INTERFACE.into(),
                changed_properties: changed,
                invalidated_properties: Vec::new(),
            };

            let msg = changed.to_emit_message(&"/org/surface/dtx".into());

            // send will only fail due to lack of memory
            self.conn.send(msg).unwrap();
        }
    }
}


impl DbusStrArgument for DeviceMode {
    fn as_str(&self) -> &str {
        match self {
            DeviceMode::Tablet => "tablet",
            DeviceMode::Laptop => "laptop",
            DeviceMode::Studio => "studio",
        }
    }
}


trait DbusArgument {
    fn as_variant(&self) -> Variant<Box<dyn dbus::arg::RefArg>>;
}

trait DbusStrArgument {
    fn as_str(&self) -> &str;
}

trait DbusStringArgument {
    fn as_string(&self) -> String;
}


impl<T> DbusStringArgument for T where T: DbusStrArgument {
    fn as_string(&self) -> String {
        self.as_str().to_owned()
    }
}

impl<T> DbusArgument for T where T: DbusStringArgument {
    fn as_variant(&self) -> Variant<Box<dyn dbus::arg::RefArg>> {
        Variant(Box::new(self.as_string()))
    }
}
