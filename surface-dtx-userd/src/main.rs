mod cli;

mod config;
use config::Config;

mod notify;
use notify::{Notification, NotificationHandle, Timeout};

use std::cell::Cell;
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


enum Msg {
    Msg(Message),
    Exit,
}


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

    // set up and start D-Bus connections (system and user)
    let (sys_rsrc, sys_conn) = connection::new::<SyncConnection>(BusType::System)
        .context("Failed to connect to D-Bus (system)")?;

    let (ses_rsrc, ses_conn) = connection::new::<SyncConnection>(BusType::Session)
        .context("Failed to connect to D-Bus (user)")?;

    let log = logger.clone();
    tokio::spawn(async move {
        let err = sys_rsrc.await;

        crit!(log, "Error: D-Bus error"; "type" => "system", "error" => %err);
        std::process::exit(1);
    });

    let log = logger.clone();
    tokio::spawn(async move {
        let err = ses_rsrc.await;

        crit!(log, "Error: D-Bus error"; "type" => "user", "error" => %err);
        std::process::exit(1);
    });

    // set up signal handling for shutdown
    let mut sigint = signal(SignalKind::interrupt()).context("Failed to set up signal handling")?;
    let mut sigterm = signal(SignalKind::terminate()).context("Failed to set up signal handling")?;

    let log = logger.clone();
    let sig = async move {
        // wait for first signal
        let cause = tokio::select! {
            _ = sigint.recv()  => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        };

        info!(log, "received {}, shutting down...", cause);

        // force termination on second signal
        tokio::spawn(async move {
            let (cause, tval) = tokio::select! {
                _ = sigint.recv()  => ("SIGINT",   2),   // = value of SIGINT
                _ = sigterm.recv() => ("SIGTERM", 15),   // = value of SIGTERM
            };

            info!(log, "received {}, terminating...", cause);
            std::process::exit(128 + tval)
        });

        Msg::Exit
    }.into_stream().boxed();

    // set up D-Bus message listener
    let mr = MatchRule::new_signal("org.surface.dtx", "DetachStateChanged");
    let (_msgs, stream) = sys_conn
        .add_match(mr).await
        .context("Failed to set up D-Bus connection")?
        .msg_stream();

    let stream = stream.map(Msg::Msg);

    // main message handler loop
    let mut stream = futures::stream::select(stream, sig);

    let handler = MessageHandler::new(logger.clone(), ses_conn);
    while let Some(Msg::Msg(m)) = stream.next().await {
        handler.handle(m).await?;
    }

    Ok(())
}

#[derive(Clone)]
struct MessageHandler {
    log:          Logger,
    connection:   Arc<SyncConnection>,
    detach_notif: Arc<Cell<Option<NotificationHandle>>>,
}

impl MessageHandler {
    fn new(log: Logger, connection: Arc<SyncConnection>) -> Self {
        MessageHandler {
            log,
            connection,
            detach_notif: Arc::new(Cell::new(None)),
        }
    }

    async fn handle(&self, mut message: Message) -> Result<()> {
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

    async fn notify_detach_ready(&self) -> Result<()> {
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

        self.detach_notif.set(Some(handle));
        Ok(())
    }

    async fn notify_detach_completed(&self) -> Result<()> {
        let notif = self.detach_notif.replace(None);

        if let Some(notif) = notif {
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
