use crate::error::Result;
use crate::device::{Device, OpMode};

use std::sync::{Arc, Mutex};
use std::collections::HashMap;

use slog::{Logger, debug};

use dbus::Message;
use dbus::nonblock::SyncConnection;
use dbus::channel::Sender;
use dbus_crossroads::{MethodErr, Crossroads, IfaceBuilder};


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
            DetachState::DetachReady     => "detach-ready",
            DetachState::DetachCompleted => "detach-completed",
            DetachState::DetachAborted   => "detach-aborted",
            DetachState::AttachCompleted => "attach-completed",
        }
    }
}


pub struct Service {
    log: Logger,
    mode: Mutex<OpMode>,
    conn: Arc<SyncConnection>,
}

impl Service {
    pub fn set_device_mode(&self, new: OpMode) {
        let old = {
            let mut mode = self.mode.lock().unwrap();
            std::mem::replace(&mut *mode, new)
        };

        debug!(self.log, "service: changing device mode";
              "old" => old.as_str(), "new" => new.as_str());

        // signal property changed
        if old != new {
            use dbus::arg::{Variant, RefArg};
            use dbus::message::SignalArgs;
            use dbus::ffidisp::stdintf::org_freedesktop_dbus as dbffi;
            use dbffi::PropertiesPropertiesChanged as PropertiesChanged;

            let mut changed: HashMap<String, Variant<Box<dyn RefArg>>> = HashMap::new();
            changed.insert("DeviceMode".into(), Variant(Box::new(new.as_str().to_owned())));

            let changed = PropertiesChanged {
                interface_name: "org.surface.dtx".into(),
                changed_properties: changed,
                invalidated_properties: Vec::new(),
            };

            let msg = changed.to_emit_message(&"/org/surface/dtx".into());
            self.conn.send(msg).unwrap();
        }
    }

    pub fn signal_detach_state_change(&self, state: DetachState) {
        let msg = Message::new_signal("/org/surface/dtx", "org.surface.dtx", "DetachStateChanged")
                .unwrap()
                .append1(state.as_str());

        debug!(self.log, "service: sending detach-state-change signal";
               "value" => state.as_str());

        self.conn.send(msg).unwrap();
    }
}


pub fn build(log: Logger, cr: &mut Crossroads, c: Arc<SyncConnection>, device: Arc<Device>)
        -> Result<Arc<Service>>
{
    let service = Arc::new(Service {
        log,
        mode: Mutex::new(OpMode::Laptop),
        conn: c,
    });

    let iface_token = cr.register("org.surface.dtx", |b: &mut IfaceBuilder<Arc<Service>>| {
        // detach-state signal
        // TODO: replace with property ?
        b.signal::<(String,), _>("DetachStateChanged", ("state",));

        // device-mode property
        b.property("DeviceMode")
            .emits_changed_true()
            .get(|_, service| { Ok(service.mode.lock().unwrap().as_str().to_owned()) });

        // request method
        b.method("Request", (), (), move |_, _, _: ()| {
            match device.commands().latch_request() {
                Ok(()) => { Ok(()) },
                Err(e) => { Err(MethodErr::failed(&e)) },
            }
        });
    });

    cr.insert("/org/surface/dtx", &[iface_token], service.clone());
    Ok(service)
}
