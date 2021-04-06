use crate::logic::Event;
use crate::utils::notify::{Notification, NotificationHandle, Timeout};

use std::sync::Arc;

use anyhow::{Context, Result};

use dbus::nonblock::SyncConnection;

use tracing::{debug, trace};


pub struct Core {
    session:      Arc<SyncConnection>,
    detach_notif: Option<NotificationHandle>,
}

impl Core {
    pub fn new(session: Arc<SyncConnection>) -> Self {
        Core {
            session,
            detach_notif: None,
        }
    }

    pub async fn handle(&mut self, event: Event) -> Result<()> {
        debug!(target: "sdtxu::core", ?event, "event received");

        // TODO

        Ok(())
    }

    #[allow(unused)]
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

        trace!(target: "sdtxu::notify", id = handle.id, ty = "detach-ready",
               "displaying notification");

        self.detach_notif = Some(handle);
        Ok(())
    }

    #[allow(unused)]
    async fn notify_detach_completed(&mut self) -> Result<()> {
        if let Some(handle) = self.detach_notif {
            trace!(target: "sdtxu::notify", id = handle.id, ty = "detach-ready",
                   "closing notification");

            handle.close(&self.session).await
                .context("Failed to close notification")?;
        }

        Ok(())
    }

    #[allow(unused)]
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

        trace!(target: "sdtxu::notify", id = handle.id, ty = "attach-complete",
               "displaying notification");

        Ok(())
    }
}
