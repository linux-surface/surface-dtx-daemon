#![allow(unused)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use dbus::arg::{RefArg, Variant};
use dbus::nonblock::{Proxy, SyncConnection};


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

#[derive(Debug)]
pub struct NotificationBuilder<'a> {
    notif: Notification<'a>,
}

#[derive(Debug, Copy, Clone)]
pub struct NotificationHandle {
    pub id: u32,
}


#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Timeout {
    Unspecified,
    Never,
    Millis(u32),
}


impl<'a> Notification<'a> {
    pub fn create<S: Into<Cow<'a, str>>>(app_name: S) -> NotificationBuilder<'a> {
        NotificationBuilder { notif: Notification::new(app_name) }
    }

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

    pub fn add_hint<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: RefArg + 'static,
    {
        self.hints.insert(key.into(), Variant(Box::new(value) as Box<dyn RefArg>));
    }

    pub fn set_expires(&mut self, timeout: Timeout) {
        self.expires = match timeout {
            Timeout::Unspecified => -1,
            Timeout::Never => 0,
            Timeout::Millis(t) => t as _,
        }
    }

    pub async fn show(self, conn: &SyncConnection) -> Result<NotificationHandle, dbus::Error> {
        let proxy = Proxy::new(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            Duration::from_secs(5),
            conn,
        );

        let (id,): (u32,) = proxy
            .method_call(
                "org.freedesktop.Notifications",
                "Notify",
                (
                    self.app_name.into_owned(),
                    self.replaces,
                    self.icon.into_owned(),
                    self.summary.into_owned(),
                    self.body.into_owned(),
                    self.actions,
                    self.hints,
                    self.expires,
                ),
            )
            .await?;

        Ok(NotificationHandle { id })
    }
}


impl<'a> NotificationBuilder<'a> {
    pub fn build(self) -> Notification<'a> {
        self.notif
    }

    pub fn replaces(mut self, id: u32) -> Self {
        self.notif.set_replaces(id);
        self
    }

    pub fn icon<S: Into<Cow<'a, str>>>(mut self, icon: S) -> Self {
        self.notif.set_icon(icon);
        self
    }

    pub fn summary<S: Into<Cow<'a, str>>>(mut self, summary: S) -> Self {
        self.notif.set_summary(summary);
        self
    }

    pub fn body<S: Into<Cow<'a, str>>>(mut self, body: S) -> Self {
        self.notif.set_body(body);
        self
    }

    pub fn hint_s<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<Cow<'a, str>>,
    {
        self.notif.add_hint_s(key, value);
        self
    }

    pub fn hint<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: RefArg + 'static,
    {
        self.notif.add_hint(key, value);
        self
    }

    pub fn expires(mut self, timeout: Timeout) -> Self {
        self.notif.set_expires(timeout);
        self
    }
}


impl NotificationHandle {
    pub async fn close(self, conn: &SyncConnection) -> Result<(), dbus::Error> {
        let proxy = Proxy::new(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            Duration::from_secs(5),
            conn,
        );

        let (): () = proxy
            .method_call(
                "org.freedesktop.Notifications",
                "CloseNotification",
                (self.id,),
            )
            .await?;

        Ok(())
    }
}
