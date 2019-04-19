mod error;
use error::{Error, ErrorKind, ErrorStr, Result, ResultExt, CliResult};

mod cli;

mod config;
use config::Config;

mod notify;
use notify::{Notification, NotificationHandle, Timeout};

use std::rc::Rc;
use std::cell::Cell;

use slog::{Logger, debug, crit, o};

use tokio::prelude::*;
use tokio::reactor::Handle;
use tokio::runtime::current_thread::Runtime;

use dbus::Message;
use dbus_tokio::AConnection;


fn logger(config: &Config) -> Logger {
    use slog::{Drain};

    let decorator = slog_term::TermDecorator::new()
        .build();

    let drain = slog_term::FullFormat::new(decorator)
        .use_original_order()
        .build()
        .filter_level(config.log.level.into())
        .fuse();

    let drain = std::sync::Mutex::new(drain)
        .fuse();

    Logger::root(drain, o!())
}

fn main() -> CliResult {
    let matches = cli::app().get_matches();

    let config = match matches.value_of("config") {
        Some(path) => Config::load_file(path)?,
        None       => Config::load()?,
    };

    let logger = logger(&config);

    let sys_c: Rc<_> = dbus::Connection::get_private(dbus::BusType::System)
        .context(ErrorKind::DBus)?
        .into();

    let ses_c: Rc<_> = dbus::Connection::get_private(dbus::BusType::Session)
        .context(ErrorKind::DBus)?
        .into();

    sys_c.add_match("type=signal,sender=org.surface.dtx,member=DetachStateChanged")
        .context(ErrorKind::DBus)?;

    let mut rt = Runtime::new().context(ErrorKind::Runtime)?;
    let sys_ac = AConnection::new(sys_c, Handle::default(), &mut rt).unwrap();
    let ses_ac = AConnection::new(ses_c, Handle::default(), &mut rt).unwrap();

    let task = sys_ac.messages()
        .map_err(ErrorStr::from)
        .context(ErrorKind::DBus)?
        .map_err(|_| Error::from(ErrorKind::DBus));

    let mut handler = MessageHandler::new(logger, ses_ac);
    let task = task.for_each(move |m| {
        handler.handle(m)
    });

    rt.block_on(task)?;
    Ok(())
}


struct MessageHandler {
    log: Logger,
    connection: AConnection,
    detach_notif: Rc<Cell<Option<NotificationHandle>>>,
}

impl MessageHandler {
    fn new(log: Logger, connection: AConnection) -> Self {
        MessageHandler { log, connection, detach_notif: Rc::new(Cell::new(None)) }
    }

    fn handle(&mut self, mut message: Message) -> Result<()> {
        let m = message.as_result().context(ErrorKind::DBus)?;
        debug!(self.log, "message received"; "message" => ?m);

        if m.interface() != Some("org.surface.dtx".into()) {
            return Ok(())
        }

        if m.member() != Some("DetachStateChanged".into()) {
            return Ok(())
        }

        let state: &str = m.read1().context(ErrorKind::DBus)?;
        debug!(self.log, "detach-state changed"; "value" => state);

        match state {
            "detach-ready" => {
                self.notify_detach_ready()
            },
            "detach-completed" | "detach-aborted" => {
                self.notify_detach_completed()
            },
            "attach-completed" => {
                self.notify_attach_completed()
            },
            _ => {
                Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid detach-state"))
                    .context(ErrorKind::DBus)
                    .map_err(Into::into)
            },
        }
    }

    fn notify_detach_ready(&mut self) -> Result<()> {
        let mut notif = Notification::new("Surface DTX");
        notif.set_summary("Surface DTX");
        notif.set_body("Clipboard can be detached.");
        notif.add_hint_s("image-path", "input-tablet");
        notif.add_hint_s("category", "device");
        notif.add_hint_u8("urgency", 2);
        notif.add_hint_b("resident", true);
        notif.set_expires(Timeout::Never);

        let task = notif.show(&self.connection)?;

        let log = self.log.clone();
        let notif_handle = self.detach_notif.clone();
        let task = task.map(move |h| {
            debug!(log, "added notification {}", h.id);
            notif_handle.set(Some(h))
        });

        let log = self.log.clone();
        let task = task.map_err(move |e| {
            panic_with_critical_error(&log, &e)
        });

        tokio::runtime::current_thread::spawn(task);

        Ok(())
    }

    fn notify_detach_completed(&mut self) -> Result<()> {
        let notif = self.detach_notif.replace(None);

        if let Some(notif) = notif {
            debug!(self.log, "closing notification {}", notif.id);
            let task = notif.close(&self.connection)?;

            let log = self.log.clone();
            let task = task.map_err(move |e| {
                panic_with_critical_error(&log, &e)
            });

            tokio::runtime::current_thread::spawn(task);
        }

        Ok(())
    }

    fn notify_attach_completed(&mut self) -> Result<()> {
        let mut notif = Notification::new("Surface DTX");
        notif.set_summary("Surface DTX");
        notif.set_body("Clipboard attached.");
        notif.add_hint_s("image-path", "input-tablet");
        notif.add_hint_s("category", "device");
        notif.add_hint_b("transient", true);

        let task = notif.show(&self.connection)?;

        let log = self.log.clone();
        let task = task.map(|_| ()).map_err(move |e| {
            panic_with_critical_error(&log, &e)
        });

        tokio::runtime::current_thread::spawn(task);

        Ok(())
    }
}

fn panic_with_critical_error(log: &Logger, err: &Error) -> ! {
    crit!(log, "Error: {}", err);
    for cause in err.iter_causes() {
        crit!(log, "Caused by: {}", cause);
    }

    panic!(format!("{}", err))
}
