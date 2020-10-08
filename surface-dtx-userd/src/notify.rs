use crate::error::{ErrorKind, Result, ResultExt};

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

#[derive(Debug, Copy, Clone)]
pub struct NotificationHandle {
    pub id: u32,
}


#[allow(unused)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Timeout {
    Unspecified,
    Never,
    Millis(u32),
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

    pub fn add_hint_u8<K>(&mut self, key: K, value: u8)
    where
        K: Into<String>,
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

    pub async fn show(self, conn: &SyncConnection) -> Result<NotificationHandle> {
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
            .await
            .context(ErrorKind::DBus)?;

        Ok(NotificationHandle { id })
    }
}


impl NotificationHandle {
    pub async fn close(self, conn: &SyncConnection) -> Result<()> {
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
            .await
            .context(ErrorKind::DBus)?;

        Ok(())
    }
}
