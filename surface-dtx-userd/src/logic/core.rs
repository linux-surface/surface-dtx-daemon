use crate::logic::{CancelReason, Event};
use crate::utils::notify::{Notification, NotificationHandle, Timeout};

use std::borrow::Cow;
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

        // display detachment-ready notification
        let handle = Notification::create("Surface DTX")
            .summary("Surface DTX: Clipboard can be detached")
            .body("You can disconnect the clipboard now.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device.removed")
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

    async fn on_detachment_complete(&mut self) -> Result<()> {
        // close detachment-ready notification
        self.close_current_notification().await
    }

    async fn on_detachment_cancel(&mut self, reason: CancelReason) -> Result<()> {
        // close detachment-ready notification
        self.close_current_notification().await?;

        // mark ourselves as canceled and prevent new detachment-ready notifications
        self.canceled = true;

        let (category, summary, body): (_, _, Cow<'static, str>) = match reason {
            CancelReason::HandlerTimeout => (
                "device.error",
                "Surface DTX: Error",
                "Detachment canceled due to handler timeout. \
                 This may lead to data loss! \
                 Please consult the logs for mode details."
                    .into()
            ),
            CancelReason::Runtime(err) => match err {
                super::types::RuntimeError::NotFeasible => (
                    "device",
                    "Surface DTX: Detachment canceled",
                    "Detachment canceled by the controller. \
                     Please make sure that the tablet battery is sufficently charged."
                        .into()
                ),
                super::types::RuntimeError::Timeout => (
                    "device.error",
                    "Surface DTX: Detachment canceled",
                    "The detachment process has timed out while the base was locked. \
                     Please ensure that the detachment handler is set up correctly."
                        .into()
                ),
                super::types::RuntimeError::Unknown(x) => (
                    "device.error",
                    "Surface DTX: Error",
                    format!("Detachment canceled due to unknown runtime error ({}).", x)
                        .into()
                ),
                _ => { return Ok(()); },
            },
            CancelReason::Hardware(err) => match err {
                super::types::HardwareError::FailedToOpen => (
                    "device.error",
                    "Surface DTX: Error",
                    "Hardware error: The controller failed to open the latch."
                        .into()
                ),
                super::types::HardwareError::FailedToRemainOpen => (
                    "device.error",
                    "Surface DTX: Error",
                    "Hardware error: The controller failed to keep the latch open."
                        .into()
                ),
                super::types::HardwareError::FailedToClose => (
                    "device.error",
                    "Surface DTX: Error",
                    "Hardware error: The controller failed to close the latch."
                        .into()
                ),
                super::types::HardwareError::Unknown(x) => (
                    "device.error",
                    "Surface DTX: Error",
                    format!("Detachment canceled due to unknown hardware error ({}).", x)
                        .into()
                ),
            },
            CancelReason::Unknown(x) => (
                "device.error",
                "Surface DTX: Error",
                format!("Detachment canceled due to unknown error ({}).", x)
                    .into()
            ),
            _ => { return Ok(()); },
        };

        let handle = Notification::create("Surface DTX")
            .summary(summary)
            .body(body)
            .hint_s("image-path", "input-tablet")
            .hint_s("category", category)
            .hint("urgency", 2)
            .build()
            .show(&self.session).await
            .context("Failed to display notification")?;

        trace!(target: "sdtxu::notify", id = handle.id, ty = "detach-cancel",
               "displaying notification");

        Ok(())
    }

    async fn on_detachment_cancel_timeout(&mut self) -> Result<()> {
        let handle = Notification::create("Surface DTX")
            .summary("Surface DTX: Error")
            .body("The detachment cancellation handler has timed out. \
                   This may lead to data loss! \
                   Please consult the logs for more details.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device.error")
            .hint("urgency", 2)
            .build()
            .show(&self.session).await
            .context("Failed to display notification")?;

        trace!(target: "sdtxu::notify", id = handle.id, ty = "detach-cancel-timeout",
               "displaying notification");

        Ok(())
    }

    async fn on_detachment_unexpected(&mut self) -> Result<()> {
        let handle = Notification::create("Surface DTX")
            .summary("Surface DTX: Error")
            .body("Base disconnected unexpectedly. \
                   This may lead to data loss! \
                   Please consult the logs for more details.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device.error")
            .hint("urgency", 2)
            .build()
            .show(&self.session).await
            .context("Failed to display notification")?;

        trace!(target: "sdtxu::notify", id = handle.id, ty = "detach-unexpected",
               "displaying notification");

        Ok(())
    }

    async fn on_attachment_complete(&mut self) -> Result<()> {
        let handle = Notification::create("Surface DTX")
            .summary("Surface DTX: Base attached")
            .body("The base has been successfully attached and is now fully usable.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device.added")
            .hint("transient", true)
            .build()
            .show(&self.session).await
            .context("Failed to display notification")?;

        trace!(target: "sdtxu::notify", id = handle.id, ty = "attach-complete",
               "displaying notification");

        Ok(())
    }

    async fn on_attachment_timeout(&mut self) -> Result<()> {
        let handle = Notification::create("Surface DTX")
            .summary("Surface DTX: Error")
            .body("The attachment handler has timed out. \
                   Please consult the logs for more details.")
            .hint_s("image-path", "input-tablet")
            .hint_s("category", "device.error")
            .hint("urgency", 2)
            .build()
            .show(&self.session).await
            .context("Failed to display notification")?;

        trace!(target: "sdtxu::notify", id = handle.id, ty = "attach-timeout",
               "displaying notification");

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
}
