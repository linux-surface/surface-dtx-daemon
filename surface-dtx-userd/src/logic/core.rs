use crate::logic::{CancelReason, Event};
use crate::utils::notify::{Notification, NotificationHandle, Timeout};

use std::sync::Arc;

use anyhow::{Context, Result};

use dbus::nonblock::SyncConnection;

use tracing::{debug, trace};


pub struct Core {
    session:  Arc<SyncConnection>,
    canceled: bool,
    notif:    Option<NotificationHandle>,
}

impl Core {
    pub fn new(session: Arc<SyncConnection>) -> Self {
        Core {
            session,
            canceled: false,
            notif:    None,
        }
    }

    pub async fn handle(&mut self, event: Event) -> Result<()> {
        debug!(target: "sdtxu::core", ?event, "event received");

        match event {
            Event::DetachmentInhibited { reason } => self.on_detachment_inhibited(reason).await,
            Event::DetachmentStart                => self.on_detachment_start().await,
            Event::DetachmentReady                => self.on_detachment_ready().await,
            Event::DetachmentComplete             => self.on_detachment_complete().await,
            Event::DetachmentCancel { reason }    => self.on_detachment_cancel(reason).await,
            Event::DetachmentCancelTimeout        => self.on_detachment_cancel_timeout().await,
            Event::DetachmentUnexpected           => self.on_detachment_unexpected().await,
            Event::AttachmentComplete             => self.on_attachment_complete().await,
            Event::AttachmentTimeout              => self.on_attachment_timeout().await,
            _ => Ok(()),
        }
    }

    async fn on_detachment_inhibited(&mut self, reason: CancelReason) -> Result<()> {
        // TODO: display info or error notification
        Ok(())
    }

    async fn on_detachment_start(&mut self) -> Result<()> {
        // reset state
        self.close_current_notification().await?;
        self.canceled = false;

        Ok(())
    }

    async fn on_detachment_ready(&mut self) -> Result<()> {
        if self.canceled {
            return Ok(());
        }

        // TODO: display detachment-ready notification

        Ok(())
    }

    async fn on_detachment_complete(&mut self) -> Result<()> {
        // close detachment-ready notification
        self.close_current_notification().await
    }

    async fn on_detachment_cancel(&mut self, reason: CancelReason) -> Result<()> {
        // close detachment-ready notification
        self.close_current_notification().await?;

        // mark ourselves as canceled and prevent new detachment-ready notifications
        self.canceled = true;

        // TODO: on error, show error notification

        Ok(())
    }

    async fn on_detachment_cancel_timeout(&mut self) -> Result<()> {
        // TODO: show error notification
        Ok(())
    }

    async fn on_detachment_unexpected(&mut self) -> Result<()> {
        // TODO: show error notification
        Ok(())
    }

    async fn on_attachment_complete(&mut self) -> Result<()> {
        // TODO: show info notification
        Ok(())
    }

    async fn on_attachment_timeout(&mut self) -> Result<()> {
        // TODO: show error notification
        Ok(())
    }

    async fn close_current_notification(&mut self) -> Result<()> {
        match self.notif {
            Some(handle) => {
                trace!(target: "sdtxu::notify", id = handle.id, "closing notification");

                handle.close(&self.session).await
                    .context("Failed to close notification")
            },
            None => Ok(()),
        }
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

        self.notif = Some(handle);
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
