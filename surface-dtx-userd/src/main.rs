mod cli;

mod config;
use config::Config;

mod notify;
use notify::{Notification, NotificationHandle, Timeout};

use std::cell::Cell;
use std::sync::Arc;

use anyhow::{Context, Error, Result};

use slog::{crit, debug, o, Logger};

use dbus::Message;
use dbus::channel::BusType;
use dbus::message::MatchRule;
use dbus::nonblock::SyncConnection;
use dbus_tokio::connection;

use futures::prelude::*;


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
    let matches = cli::app().get_matches();

    let config = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    let logger = logger(&config);

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

    let mr = MatchRule::new_signal("org.surface.dtx", "DetachStateChanged");
    let (_msgs, stream) = sys_conn
        .add_match(mr)
        .await
        .context("Failed to set up D-Bus connection")?
        .msg_stream();

    let log = logger.clone();
    let handler = MessageHandler::new(logger, ses_conn);
    let stream = stream.for_each(move |m| {
        let log = log.clone();
        let handler = handler.clone();
        async move {
            if let Err(err) = handler.handle(m).await {
                panic_with_critical_error(&log, &err);
            }
        }
    });

    stream.await;
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
        let mut notif = Notification::new("Surface DTX");
        notif.set_summary("Surface DTX");
        notif.set_body("Clipboard can be detached.");
        notif.add_hint_s("image-path", "input-tablet");
        notif.add_hint_s("category", "device");
        notif.add_hint("urgency", 2);
        notif.add_hint("resident", true);
        notif.set_expires(Timeout::Never);

        let handle = notif.show(&self.connection).await
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
        let mut notif = Notification::new("Surface DTX");
        notif.set_summary("Surface DTX");
        notif.set_body("Clipboard attached.");
        notif.add_hint_s("image-path", "input-tablet");
        notif.add_hint_s("category", "device");
        notif.add_hint("transient", true);

        notif.show(&self.connection).await
            .context("Failed to display notification")?;

        Ok(())
    }
}

fn panic_with_critical_error(log: &Logger, err: &Error) -> ! {
    crit!(log, "Error: {}", err);
    for cause in err.chain().skip(1) {
        crit!(log, "Caused by: {}", cause);
    }

    panic!("{:?}", err)
}
