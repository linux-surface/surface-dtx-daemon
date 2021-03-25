use crate::notify::{Notification, NotificationHandle, Timeout};
use crate::utils::JoinHandleExt;

use std::sync::Arc;

use anyhow::{Context, Result};

use dbus::Message;
use dbus::channel::BusType;
use dbus::message::MatchRule;
use dbus::nonblock::SyncConnection;
use dbus_tokio::connection;

use futures::prelude::*;

use slog::{Logger, debug};


pub async fn run(logger: Logger) -> Result<()> {
    // set up and start D-Bus connections (system and user-session)
    let (sys_rsrc, sys_conn) = connection::new::<SyncConnection>(BusType::System)
        .context("Failed to connect to D-Bus (system)")?;

    let (ses_rsrc, ses_conn) = connection::new::<SyncConnection>(BusType::Session)
        .context("Failed to connect to D-Bus (session)")?;

    let sys_rsrc = sys_rsrc.map(|e| Err(e).context("D-Bus connection error (system)"));
    let ses_rsrc = ses_rsrc.map(|e| Err(e).context("D-Bus connection error (session)"));

    let mut dsys_task = tokio::spawn(sys_rsrc).guard();
    let mut dses_task = tokio::spawn(ses_rsrc).guard();

    // set up D-Bus message listener task
    let log = logger.clone();
    let mut main_task = tokio::spawn(async move {
        let mut handler = MessageHandler::new(log, ses_conn);

        let mr = MatchRule::new_signal("org.surface.dtx", "DetachStateChanged");
        let (_msgs, mut stream) = sys_conn
            .add_match(mr).await
            .context("Failed to set up D-Bus connection")?
            .msg_stream();

        while let Some(m) = stream.next().await {
            handler.handle(m).await?;
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


#[derive(Clone)]
struct MessageHandler {
    log:          Logger,
    connection:   Arc<SyncConnection>,
    detach_notif: Option<NotificationHandle>,
}

impl MessageHandler {
    fn new(log: Logger, connection: Arc<SyncConnection>) -> Self {
        MessageHandler {
            log,
            connection,
            detach_notif: None,
        }
    }

    async fn handle(&mut self, mut message: Message) -> Result<()> {
        let m = message.as_result()
            .context("D-Bus remote error")?;

        debug!(self.log, "message received"; "message" => ?m);

        if m.interface() != Some("org.surface.dtx".into()) {
            return Ok(());
        }

        if m.member() != Some("DetachStateChanged".into()) {
            return Ok(());
        }

        let state: &str = m.read1()
            .context("Protocol error")?;

        debug!(self.log, "detach-state changed"; "value" => state);

        match state {
            "detach-ready" => {
                self.notify_detach_ready().await
            },
            "detach-completed" | "detach-aborted" => {
                self.notify_detach_completed().await
            },
            "attach-completed" => {
                self.notify_attach_completed().await
            },
            _ => {
                Err(anyhow::anyhow!("Invalid detachment state: {}", state)
                    .context("Protocol error"))
            },
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
            .show(&self.connection).await
            .context("Failed to display notification")?;

        debug!(self.log, "added notification {}", handle.id);

        self.detach_notif = Some(handle);
        Ok(())
    }

    async fn notify_detach_completed(&mut self) -> Result<()> {
        if let Some(notif) = self.detach_notif {
            debug!(self.log, "closing notification {}", notif.id);

            notif.close(&self.connection).await
                .context("Failed to close notification")?;
        }

        Ok(())
    }

    async fn notify_attach_completed(&self) -> Result<()> {
        Notification::create("Surface DTX")
            .summary("Surface DTX")
            .body("Clipboard attached.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device")
            .hint("transient", true)
            .build()
            .show(&self.connection).await
            .context("Failed to display notification")?;

        Ok(())
    }
}
