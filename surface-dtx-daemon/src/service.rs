use crate::logic::{DeviceMode, HardwareError, LatchStatus};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use dbus::arg::Variant;
use dbus::nonblock::SyncConnection;
use dbus_crossroads::{Crossroads, IfaceBuilder, MethodErr};

use sdtx_tokio::Device;

use tracing::debug;


pub struct Service {
    conn: Arc<SyncConnection>,
    inner: Arc<ServiceInner>,
}

impl Service {
    const NAME: &'static str = "/org/surface/dtx";
    const INTERFACE: &'static str = "org.surface.dtx";

    pub fn new(conn: Arc<SyncConnection>, device: Device) -> Self {
        Self { conn, inner: Arc::new(ServiceInner::new(device)) }
    }

    pub async fn request_name(&self) -> Result<()> {
        self.conn.request_name(Self::INTERFACE, false, true, false).await
            .context("Failed to set up D-Bus service")
            .map(|_| ())
    }

    pub fn register(&self, cr: &mut Crossroads) -> Result<()> {
        let iface_token = cr.register(Self::INTERFACE, |b: &mut IfaceBuilder<Arc<ServiceInner>>| {
            // device-mode property
            b.property("DeviceMode")
                .emits_changed_true()
                .get(|_, service| { Ok(service.device_mode.as_arg()) });

            // latch status
            b.property("LatchStatus")
                .emits_changed_true()
                .get(|_, service| Ok(service.latch_status.as_arg()));

            // request method
            b.method("Request", (), (), move |_ctx, service, _args: ()| {
                match service.device.latch_request() {
                    Ok(()) => { Ok(()) },
                    Err(e) => { Err(MethodErr::failed(&e)) },
                }
            });
        });

        cr.insert(Self::NAME, &[iface_token], self.inner.clone());
        Ok(())
    }

    pub fn unregister(&self, cr: &mut Crossroads) {
        let _ : Option<Arc<ServiceInner>> = cr.remove(&Self::NAME.into());
    }

    pub fn handle(&self) -> ServiceHandle {
        ServiceHandle { conn: self.conn.clone(), inner: self.inner.clone() }
    }
}


#[derive(Clone)]
pub struct ServiceHandle {
    conn: Arc<SyncConnection>,
    inner: Arc<ServiceInner>,
}

impl ServiceHandle {
    pub fn set_device_mode(&self, value: DeviceMode) {
        self.inner.device_mode.set(self.conn.as_ref(), value);
    }

    pub fn set_latch_status(&self, value: LatchStatus) {
        self.inner.latch_status.set(self.conn.as_ref(), value);
    }
}


struct ServiceInner {
    device: Device,
    device_mode: Property<DeviceMode>,
    latch_status: Property<LatchStatus>,
}

impl ServiceInner {
    fn new(device: Device) -> Self {
        Self {
            device,
            device_mode: Property::new("DeviceMode", DeviceMode::Laptop),
            latch_status: Property::new("LatchStatus", LatchStatus::Closed),
        }
    }
}


trait DbusArg {
    type Arg: dbus::arg::RefArg + 'static;

    fn as_arg(&self) -> Self::Arg;

    fn as_variant(&self) -> Variant<Box<dyn dbus::arg::RefArg>> {
        Variant(Box::new(self.as_arg()))
    }
}

impl DbusArg for DeviceMode {
    type Arg = String;

    fn as_arg(&self) -> String {
        match self {
            DeviceMode::Tablet => "tablet",
            DeviceMode::Laptop => "laptop",
            DeviceMode::Studio => "studio",
        }.into()
    }
}

impl DbusArg for LatchStatus {
    type Arg = String;

    fn as_arg(&self) -> String {
        match self {
            LatchStatus::Closed => "closed".into(),
            LatchStatus::Opened => "opened".into(),
            LatchStatus::Error(error) => match error {
                HardwareError::FailedToOpen       => "error:failed-to-open".into(),
                HardwareError::FailedToRemainOpen => "error:failed-to-remain-open".into(),
                HardwareError::FailedToClose      => "error:failed-to-close".into(),
                HardwareError::Unknown(x) => format!("error:unknown:{}", x),
            },
        }
    }
}


#[derive(Debug)]
struct Property<T> {
    name: &'static str,
    value: Mutex<T>,
}

impl<T> Property<T> {
    pub fn new(name: &'static str, value: T) -> Self {
        Self { name, value: Mutex::new(value) }
    }

    pub fn set<C>(&self, conn: &C, value: T)
    where
        C: dbus::channel::Sender,
        T: DbusArg + PartialEq + std::fmt::Debug,
    {
        // update stored value and get variant
        let value = {
            let mut stored = self.value.lock().unwrap();

            // check for actual change
            if *stored == value {
                return;
            }

            debug!(target: "sdtxd::srvc", object=Service::NAME, interface=Service::INTERFACE,
                   name=self.name, old=?*stored, new=?value, "changing property");

            *stored = value;
            stored.as_variant()
        };

        // signal property changed
        use dbus::arg::RefArg;
        use dbus::message::SignalArgs;
        use dbus::ffidisp::stdintf::org_freedesktop_dbus as dbffi;
        use dbffi::PropertiesPropertiesChanged as PropertiesChanged;

        let mut changed: HashMap<String, Variant<Box<dyn RefArg>>> = HashMap::new();
        changed.insert(self.name.into(), value);

        let changed = PropertiesChanged {
            interface_name: Service::INTERFACE.into(),
            changed_properties: changed,
            invalidated_properties: Vec::new(),
        };

        let msg = changed.to_emit_message(&Service::NAME.into());

        // send will only fail due to lack of memory
        conn.send(msg).unwrap();
    }
}

impl<T> DbusArg for Property<T>
where
    T: DbusArg
{
    type Arg = T::Arg;

    fn as_arg(&self) -> Self::Arg {
        self.value.lock().unwrap().as_arg()
    }
}

impl<T> std::ops::Deref for Property<T> {
    type Target = Mutex<T>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
