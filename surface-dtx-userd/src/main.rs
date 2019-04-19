mod error;
use error::{Error, ErrorKind, ErrorStr, ResultExt, CliResult};

mod cli;

mod config;
use config::Config;

mod notify;

use std::rc::Rc;

use slog::{Logger, debug, crit, o};

use tokio::prelude::*;
use tokio::reactor::Handle;
use tokio::runtime::current_thread::Runtime;

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

    sys_c.add_match("type=signal,sender=org.surface.dtx,member=PropertiesChanged")
        .context(ErrorKind::DBus)?;

    let mut rt = Runtime::new().context(ErrorKind::Runtime)?;
    let sys_ac = AConnection::new(sys_c, Handle::default(), &mut rt).unwrap();
    let ses_ac = AConnection::new(ses_c, Handle::default(), &mut rt).unwrap();

    let messages = sys_ac.messages()
        .map_err(ErrorStr::from)
        .context(ErrorKind::DBus)?;

    let task = messages.map_err(|_| Error::from(ErrorKind::DBus));
    let task = task.for_each(move |mut m| {
        let m = m.as_result().context(ErrorKind::DBus)?;
        debug!(logger, "message received: {:#?}", m);

        use dbus::SignalArgs;
        use dbus::stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged;

        if let Some(sig) = PropertiesPropertiesChanged::from_message(&m) {
            debug!(logger, "properties changed: {:#?}", sig);

            let mut notif = notify::Notification::new("Surface DTX");
            notif.set_summary("Device mode changed");
            notif.set_body(format!("New value: {}", "TODO"));
            notif.add_hint_s("image-path", "input-tablet");
            notif.add_hint_s("category", "device");
            notif.add_hint_b("transient", true);

            let task = notif.send(&ses_ac)?;

            let log = logger.clone();
            let task = task.map(|_| ()).map_err(move |e| {
                panic_with_critical_error(&log, &e)
            });

            tokio::runtime::current_thread::spawn(task);
        }

        Ok(())
    });

    rt.block_on(task)?;
    Ok(())
}

fn panic_with_critical_error(log: &Logger, err: &Error) -> ! {
    crit!(log, "Error: {}", err);
    for cause in err.iter_causes() {
        crit!(log, "Caused by: {}", cause);
    }

    panic!(format!("{}", err))
}
