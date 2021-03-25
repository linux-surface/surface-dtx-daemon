mod cli;

mod config;
use config::Config;

mod notify;
use notify::{Notification, NotificationHandle, Timeout};

use std::sync::Arc;

use anyhow::{Context, Result};

use dbus::Message;
use dbus::channel::BusType;
use dbus::message::MatchRule;
use dbus::nonblock::SyncConnection;
use dbus_tokio::connection;

use futures::prelude::*;

use slog::{Logger, crit, debug, info, o};

use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinHandle;


fn logger(config: &Config) -> Logger {
    use slog::Drain;

    let decorator = slog_term::TermDecorator::new().build();

    let drain = slog_term::FullFormat::new(decorator)
        .use_original_order()
        .build()
        .filter_level(config.log.level.into())
        .fuse();

    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // handle command line input
    let matches = cli::app().get_matches();

    // set up config
    let config = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    // set up logger
    let logger = logger(&config);

    // set up and start D-Bus connections (system and user-session)
    let (sys_rsrc, sys_conn) = connection::new::<SyncConnection>(BusType::System)
        .context("Failed to connect to D-Bus (system)")?;

    let (ses_rsrc, ses_conn) = connection::new::<SyncConnection>(BusType::Session)
        .context("Failed to connect to D-Bus (session)")?;

    let sys_task = tokio::spawn(sys_rsrc);
    let ses_task = tokio::spawn(ses_rsrc);

    // set up signal handling for shutdown
    let mut sigint = signal(SignalKind::interrupt()).context("Failed to set up signal handling")?;
    let mut sigterm = signal(SignalKind::terminate()).context("Failed to set up signal handling")?;

    let log = logger.clone();
    let sig = async move {
        let cause = tokio::select! {
            _ = sigint.recv()  => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        };

        info!(log, "received {}, shutting down...", cause);
    };

    // set up D-Bus message listener task
    let log = logger.clone();
    let main: JoinHandle<Result<_>> = tokio::spawn(async move {
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
    });

    // wait for error or shutdown signal
    tokio::select! {
        _ = sig => {
            Ok(())
        },
        result = main => {
            match result {
                Ok(r) => r,
                Err(e) if e.is_panic() => {
                    crit!(logger, "Main message handler task panicked");
                    std::panic::resume_unwind(e.into_panic())
                },
                Err(_) => Ok(()),
            }
        },
        result = sys_task => {
            match result {
                Ok(e) => Err(e).context("D-Bus connection error (system)"),
                Err(e) if e.is_panic() => {
                    crit!(logger, "D-Bus system task panicked");
                    std::panic::resume_unwind(e.into_panic())
                },
                Err(_) => Ok(()),
            }
        },
        result = ses_task => {
            match result {
                Ok(e) => Err(e).context("D-Bus connection error (session)"),
                Err(e) if e.is_panic() => {
                    crit!(logger, "D-Bus session task panicked");
                    std::panic::resume_unwind(e.into_panic())
                },
                Err(_) => Ok(()),
            }
        },
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
