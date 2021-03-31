use crate::notify::{Notification, NotificationHandle, Timeout};
use crate::utils::JoinHandleExt;

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};

use dbus::Message;
use dbus::message::MatchRule;
use dbus::nonblock::SyncConnection;
use dbus_tokio::connection;

use futures::prelude::*;

use tracing::{debug, trace};


pub async fn run() -> Result<()> {
    // set up and start D-Bus connections (system and user-session)
    let (sys_rsrc, sys_conn) = connection::new_system_sync()
        .context("Failed to connect to D-Bus (system)")?;

    let (ses_rsrc, ses_conn) = connection::new_session_sync()
        .context("Failed to connect to D-Bus (session)")?;

    let sys_rsrc = sys_rsrc.map(|e| Err(e).context("D-Bus connection error (system)"));
    let ses_rsrc = ses_rsrc.map(|e| Err(e).context("D-Bus connection error (session)"));

    let mut dsys_task = tokio::spawn(sys_rsrc).guard();
    let mut dses_task = tokio::spawn(ses_rsrc).guard();

    // set up D-Bus message listener task
    let mut main_task = tokio::spawn(async move {
        let mut core = Core::new(ses_conn);

        let mr = MatchRule::new_signal("org.surface.dtx", "DetachStateChanged");
        let (_msgs, mut stream) = sys_conn
            .add_match(mr).await
            .context("Failed to set up D-Bus connection")?
            .msg_stream();

        while let Some(m) = stream.next().await {
            core.handle(m).await?;
        }

        Ok(())
    }).guard();

    // wait for error, panic, or shutdown signal
    let result = tokio::select! {
        result = &mut main_task => result,
        result = &mut dsys_task => result,
        result = &mut dses_task => result,
    };

    // (try to) shut down all active tasks
    main_task.abort();
    dses_task.abort();
    dsys_task.abort();

    // handle subtask result, propagate resume unwind panic
    match result {
        Ok(result) => result,
        Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        Err(_) => unreachable!("Subtask canceled before completing parent task"),
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    DetachReady,
    DetachCompleted,
    DetachAborted,
    AttachCompleted,
}

impl FromStr for Status {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "detach-ready"     => Ok(Self::DetachReady),
            "detach-completed" => Ok(Self::DetachCompleted),
            "detach-aborted"   => Ok(Self::DetachAborted),
            "attach-completed" => Ok(Self::AttachCompleted),
            _ => anyhow::bail!("Invalid detachment state: '{}'", s),
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::DetachReady     => write!(f, "detach-ready"),
            Status::DetachCompleted => write!(f, "detach-completed"),
            Status::DetachAborted   => write!(f, "detach-aborted"),
            Status::AttachCompleted => write!(f, "attach-completed"),
        }
    }
}


#[derive(Clone)]
struct Core {
    session:      Arc<SyncConnection>,
    detach_notif: Option<NotificationHandle>,
}

impl Core {
    fn new(session: Arc<SyncConnection>) -> Self {
        Core {
            session,
            detach_notif: None,
        }
    }

    async fn handle(&mut self, mut message: Message) -> Result<()> {
        let m = message.as_result()
            .context("D-Bus remote error")?;

        debug!(msg = ?m, "message received");

        // ignore any message that is not intended for us
        if m.interface() != Some("org.surface.dtx".into()) {
            return Ok(());
        }

        if m.member() != Some("DetachStateChanged".into()) {
            return Ok(());
        }

        // get status argument
        let status: Status = m.read1::<&str>()
            .context("Protocol error")?
            .parse()
            .context("Protocol error")?;

        debug!(status = %status, "detach-state changed");

        // handle status notification
        match status {
            Status::DetachReady     => self.notify_detach_ready().await,
            Status::DetachCompleted => self.notify_detach_completed().await,
            Status::DetachAborted   => self.notify_detach_completed().await,
            Status::AttachCompleted => self.notify_attach_completed().await,
        }
    }

    async fn notify_detach_ready(&mut self) -> Result<()> {
        let handle = Notification::create("Surface DTX")
            .summary("Surface DTX")
            .body("Clipboard can be detached.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device")
            .hint("urgency", 2)
            .hint("resident", true)
            .expires(Timeout::Never)
            .build()
            .show(&self.session).await
            .context("Failed to display notification")?;

        trace!(target: "surface_dtx_userd::notify", id = handle.id, ty = "detach-ready",
               "displaying notification");

        self.detach_notif = Some(handle);
        Ok(())
    }

    async fn notify_detach_completed(&mut self) -> Result<()> {
        if let Some(handle) = self.detach_notif {
            trace!(target: "surface_dtx_userd::notify", id = handle.id, ty = "detach-ready",
                   "closing notification");

            handle.close(&self.session).await
                .context("Failed to close notification")?;
        }

        Ok(())
    }

    async fn notify_attach_completed(&self) -> Result<()> {
        let handle = Notification::create("Surface DTX")
            .summary("Surface DTX")
            .body("Clipboard attached.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device")
            .hint("transient", true)
            .build()
            .show(&self.session).await
            .context("Failed to display notification")?;

        trace!(target: "surface_dtx_userd::notify", id = handle.id, ty = "attach-complete",
               "displaying notification");

        Ok(())
    }
}
