use crate::service::Service;
use crate::service::arg::DbusArg;

use std::collections::HashMap;
use std::sync::Mutex;

use dbus::arg::Variant;

use tracing::trace;


#[derive(Debug)]
pub struct Property<T> {
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

            trace!(target: "sdtxd::srvc", object=Service::PATH, interface=Service::INTERFACE,
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

        let msg = changed.to_emit_message(&Service::PATH.into());

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
