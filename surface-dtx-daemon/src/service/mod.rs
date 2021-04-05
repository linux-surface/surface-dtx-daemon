mod arg;
use arg::DbusArg;

mod event;
pub use event::Event;

mod prop;
use prop::Property;


use crate::logic::{
    BaseInfo,
    BaseState,
    DeviceMode,
    DeviceType,
    LatchStatus,
};

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};

use dbus::{Message, arg::{RefArg, Variant}};
use dbus::nonblock::SyncConnection;
use dbus_crossroads::{Crossroads, IfaceBuilder, MethodErr};

use sdtx_tokio::Device;


pub struct Service {
    conn: Arc<SyncConnection>,
    inner: Arc<Shared>,
}

impl Service {
    const PATH: &'static str = "/org/surface/dtx";
    const INTERFACE: &'static str = "org.surface.dtx";

    pub fn new(conn: Arc<SyncConnection>, device: Device) -> Self {
        Self { conn, inner: Arc::new(Shared::new(device)) }
    }

    pub async fn request_name(&self) -> Result<()> {
        self.conn.request_name(Self::INTERFACE, false, true, false).await
            .context("Failed to set up D-Bus service")
            .map(|_| ())
    }

    pub fn register(&self, cr: &mut Crossroads) -> Result<()> {
        let iface_token = cr.register(Self::INTERFACE, |b: &mut IfaceBuilder<Arc<Shared>>| {
            // device-mode property
            b.property("DeviceMode")
                .emits_changed_true()
                .get(|_, service| { Ok(service.device_mode.as_arg()) });

            // latch status
            b.property("LatchStatus")
                .emits_changed_true()
                .get(|_, service| Ok(service.latch_status.as_arg()));

            // base info
            b.property("Base")
                .emits_changed_true()
                .get(|_, service| Ok(service.base_info.as_arg()));

            // request method
            b.method("Request", (), (), move |_ctx, service, _args: ()| {
                match service.device.latch_request() {
                    Ok(()) => { Ok(()) },
                    Err(e) => { Err(MethodErr::failed(&e)) },
                }
            });

            // event signal
            b.signal::<(String, HashMap<String, Variant<Box<dyn RefArg>>>), _>
                ("Event", ("type", "values"));
        });

        cr.insert(Self::PATH, &[iface_token], self.inner.clone());
        Ok(())
    }

    pub fn unregister(&self, cr: &mut Crossroads) {
        let _ : Option<Arc<Shared>> = cr.remove(&Self::PATH.into());
    }

    pub fn handle(&self) -> ServiceHandle {
        ServiceHandle { conn: self.conn.clone(), inner: self.inner.clone() }
    }
}


#[derive(Clone)]
pub struct ServiceHandle {
    conn: Arc<SyncConnection>,
    inner: Arc<Shared>,
}

impl ServiceHandle {
    pub fn set_device_mode(&self, value: DeviceMode) {
        self.inner.device_mode.set(self.conn.as_ref(), value);
    }

    pub fn set_latch_status(&self, value: LatchStatus) {
        self.inner.latch_status.set(self.conn.as_ref(), value);
    }

    pub fn set_base_info(&self, value: BaseInfo) {
        self.inner.base_info.set(self.conn.as_ref(), value);
    }

    pub fn emit_event(&self, event: Event) {
        use dbus::channel::Sender;

        let path = Service::PATH.into();
        let interface = Service::INTERFACE.into();

        // build signal message
        let mut signal = Message::signal(&path, &interface, &"Event".into());
        signal.append_all(event);

        // only fails when memory runs out
        self.conn.send(signal).unwrap();
    }
}


struct Shared {
    device: Device,
    device_mode: Property<DeviceMode>,
    latch_status: Property<LatchStatus>,
    base_info: Property<BaseInfo>,
}

impl Shared {
    fn new(device: Device) -> Self {
        let base = BaseInfo {
            state: BaseState::Attached,
            device_type: DeviceType::Ssh,
            id: 0,
        };

        Self {
            device,
            device_mode: Property::new("DeviceMode", DeviceMode::Laptop),
            latch_status: Property::new("LatchStatus", LatchStatus::Closed),
            base_info: Property::new("Base", base),
        }
    }
}
