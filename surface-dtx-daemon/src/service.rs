use crate::ControlDevice;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use dbus::Message;
use dbus::channel::Sender;
use dbus::nonblock::SyncConnection;
use dbus_crossroads::{Crossroads, IfaceBuilder, MethodErr};

use sdtx::DeviceMode;

use slog::{debug, Logger};


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DetachState {
    DetachReady,
    DetachCompleted,
    DetachAborted,
    AttachCompleted,
}

impl DetachState {
    fn as_str(self) -> &'static str {
        match self {
            Self::DetachReady     => "detach-ready",
            Self::DetachCompleted => "detach-completed",
            Self::DetachAborted   => "detach-aborted",
            Self::AttachCompleted => "attach-completed",
        }
    }
}


pub struct Service {
    log: Logger,
    mode: Mutex<DeviceMode>,
    conn: Arc<SyncConnection>,
}

impl Service {
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

    pub fn signal_detach_state_change(&self, state: DetachState) {
        let msg = Message::new_signal("/org/surface/dtx", "org.surface.dtx", "DetachStateChanged")
                .unwrap()       // out of memory
                .append1(state.as_str());

        debug!(self.log, "service: sending detach-state-change signal";
               "value" => state.as_str());

        // send will only fail due to lack of memory
        self.conn.send(msg).unwrap();
    }
}


pub fn build(log: Logger, cr: &mut Crossroads, c: Arc<SyncConnection>, device: Arc<ControlDevice>)
        -> Result<Arc<Service>>
{
    let service = Arc::new(Service {
        log,
        mode: Mutex::new(DeviceMode::Laptop),
        conn: c,
    });

    let iface_token = cr.register("org.surface.dtx", |b: &mut IfaceBuilder<Arc<Service>>| {
        // detach-state signal
        // TODO: replace with property ?
        b.signal::<(String,), _>("DetachStateChanged", ("state",));

        // device-mode property
        b.property("DeviceMode")
            .emits_changed_true()
            .get(|_, service| { Ok(format!("{}", service.mode.lock().unwrap()).to_lowercase()) });

        // request method
        b.method("Request", (), (), move |_, _, _: ()| {
            match device.latch_request() {
                Ok(()) => { Ok(()) },
                Err(e) => { Err(MethodErr::failed(&e)) },
            }
        });
    });

    cr.insert("/org/surface/dtx", &[iface_token], service.clone());
    Ok(service)
}
