use crate::error::{Error, ErrorKind, ErrorStr, Result, ResultExt};

use std::borrow::Cow;
use std::collections::HashMap;

use tokio::prelude::*;

use dbus::Message;
use dbus::arg::{RefArg, Variant};

use dbus_tokio::AConnection;


#[derive(Debug)]
pub struct Notification<'a> {
    app_name: Cow<'a, str>,
    replaces: u32,
    icon:     Cow<'a, str>,
    summary:  Cow<'a, str>,
    body:     Cow<'a, str>,
    actions:  Vec<String>,
    hints:    HashMap<String, Variant<Box<dyn RefArg>>>,
    expires:  i32,
}

pub struct NotificationHandle {
    id: u32,
}


#[allow(unused)]
impl<'a> Notification<'a> {
    pub fn new<S: Into<Cow<'a, str>>>(app_name: S) -> Self {
        Notification {
            app_name: app_name.into(),
            replaces: 0,
            icon:     Default::default(),
            summary:  Default::default(),
            body:     Default::default(),
            actions:  Default::default(),
            hints:    Default::default(),
            expires:  -1,
        }
    }

    pub fn set_replaces(&mut self, id: u32) {
        self.replaces = id
    }

    pub fn set_icon<S: Into<Cow<'a, str>>>(&mut self, icon: S) {
        self.icon = icon.into()
    }

    pub fn set_summary<S: Into<Cow<'a, str>>>(&mut self, summary: S) {
        self.summary = summary.into()
    }

    pub fn set_body<S: Into<Cow<'a, str>>>(&mut self, body: S) {
        self.body = body.into()
    }

    pub fn add_hint_s<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<Cow<'a, str>>,
    {
        let value = value.into().into_owned();
        self.hints.insert(key.into(), Variant(Box::new(value) as Box<dyn RefArg>));
    }

    pub fn add_hint_b<K>(&mut self, key: K, value: bool)
    where
        K: Into<String>,
    {
        self.hints.insert(key.into(), Variant(Box::new(value) as Box<dyn RefArg>));
    }

    pub fn set_expires(&mut self, timeout: Option<u32>) {
        self.expires = timeout.map(|v| v as i32).unwrap_or(-1);
    }

    pub fn into_message(self) -> Message {
        let m = Message::new_method_call(
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "Notify").unwrap();

        let m = m.append1(self.app_name.into_owned());
        let m = m.append1(self.replaces);
        let m = m.append1(self.icon.into_owned());
        let m = m.append1(self.summary.into_owned());
        let m = m.append1(self.body.into_owned());
        let m = m.append1(self.actions);
        let m = m.append1(self.hints);
        let m = m.append1(self.expires);

        m
    }

    pub fn send(self, conn: &AConnection) -> Result<impl Future<Item=NotificationHandle, Error=Error>> {
        let task = conn.method_call(self.into_message())
            .map_err(ErrorStr::from)
            .context(ErrorKind::DBus)?
            .map_err(|e| Error::with(e, ErrorKind::DBus));

        let task = task.and_then(|m| {
            let id: u32 = m.read1().context(ErrorKind::DBus)?;
            Ok(NotificationHandle { id })
        });

        Ok(task)
    }
}

impl<'a> Into<Message> for Notification<'a> {
    fn into(self) -> Message {
        self.into_message()
    }
}


#[allow(unused)]
impl NotificationHandle {
    pub fn close(self, conn: &AConnection) -> Result<impl Future<Item=(), Error=Error>> {
        let m = Message::new_method_call(
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "Notify").unwrap();

        let m = m.append1(self.id);

        let task = conn.method_call(m)
            .map_err(ErrorStr::from)
            .context(ErrorKind::DBus)?
            .map_err(|e| Error::with(e, ErrorKind::DBus))
            .map(|_| ());

        Ok(task)
    }
}
