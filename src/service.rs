use crate::error::{Result, ResultExt, Error, ErrorKind, ErrorStr};
use crate::device::OpMode;

use std::rc::Rc;
use std::sync::Arc;
use std::cell::Cell;

use slog::{Logger, debug};

use dbus::{Connection, SignalArgs};
use dbus::tree::{MTFn, ObjectPath, Interface, Signal, Property, Access, EmitsChangedSignal};

use dbus_tokio::AConnection;
use dbus_tokio::tree::{ATree, AFactory, ATreeServer};

use tokio::prelude::*;
use tokio::reactor::Handle;
use tokio::runtime::current_thread::Runtime;


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
    connection: Rc<Connection>,
    object: Arc<ObjectPath<MTFn<ATree<()>>, ATree<()>>>,
    iface: Arc<Interface<MTFn<ATree<()>>, ATree<()>>>,
    sig_detach_state: Arc<Signal<ATree<()>>>,
    prop_device_mode: Arc<Property<MTFn<ATree<()>>, ATree<()>>>,
    prop_device_mode_val: Arc<Cell<OpMode>>,
}

impl Service {
    pub fn task(&self, rt: &mut Runtime) -> Result<impl Future<Item=(), Error=Error>> {
        let factory = AFactory::new_afn::<()>();

        let tree: Arc<_> = factory.tree(ATree::new())
            .add(self.object.clone())
            .into();

        tree.set_registered(&self.connection, true)
            .context(ErrorKind::DBusService)?;

        let aconn = AConnection::new(self.connection.clone(), Handle::default(), rt)
            .context(ErrorKind::DBusService)?;

        let msgs = aconn.messages()
            .map_err(ErrorStr::from)
            .context(ErrorKind::DBusService)?;

        let server = ATreeServer::new(self.connection.clone(), tree, msgs);

        let task = server.for_each(|_| Ok(()))
            .map_err(|_| ErrorKind::DBusService.into());

        Ok(task)
    }

    pub fn set_device_mode(&self, mode: OpMode) {
        let old = self.prop_device_mode_val.replace(mode);

        debug!(self.log, "service: changing device mode";
               "old" => old.as_str(), "new" => mode.as_str());

        if mode != old {
            // signal property changed
            let mut chg = Vec::new();
            self.prop_device_mode.add_propertieschanged(&mut chg, self.iface.get_name(), || {
                Box::new(mode.as_str().to_owned()) as _
            });

            let msg = chg.first().unwrap().to_emit_message(self.object.get_name());

            // this will only fail due to lack of memory
            self.connection.send(msg).unwrap();
        }
    }

    pub fn signal_detach_state_change(&self, state: DetachState) {
        let msg = self.sig_detach_state.msg(self.object.get_name(), self.iface.get_name())
            .append1(state.as_str());

        debug!(self.log, "service: sending detach-state-change signal";
               "value" => state.as_str());

        // this will only fail due to lack of memory
        self.connection.send(msg).unwrap();
    }
}


pub fn build(log: Logger, connection: Rc<Connection>) -> Result<Service> {
    let factory = AFactory::new_afn::<()>();

    connection.register_name("org.surface.dtx", dbus::NameFlag::ReplaceExisting as u32)
        .context(ErrorKind::DBusService)?;

    // detach-state signal
    let state_signal: Arc<_> = factory.signal("DetachStateChanged", ())
        .sarg::<&str, _>("state")
        .into();

    // device-mode property
    let device_mode_val = Arc::new(Cell::new(OpMode::Laptop));      // TODO: ensure that this is current
    let device_mode = factory.property::<&str, _>("DeviceMode", ())
        .emits_changed(EmitsChangedSignal::True)
        .access(Access::Read);

    let val = device_mode_val.clone();
    let device_mode = device_mode.on_get(move |i, _m| {
        i.append(val.get().as_str());
        Ok(())
    });

    let device_mode = Arc::new(device_mode);

    // interface
    let iface: Arc<_> = factory.interface("org.surface.dtx", ())
        .add_s(state_signal.clone())
        .add_p(device_mode.clone())
        .into();

    // interface
    let object: Arc<_> = factory.object_path("/org/surface/dtx", ())
        .introspectable()
        .add(iface.clone())
        .into();

    Ok(Service {
        log,
        connection,
        object,
        iface,
        sig_detach_state: state_signal,
        prop_device_mode: device_mode,
        prop_device_mode_val: device_mode_val,
    })
}
