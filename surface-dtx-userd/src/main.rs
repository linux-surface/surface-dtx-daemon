mod error;
use error::{Error, ErrorKind, ErrorStr, ResultExt, CliResult};

mod cli;

mod config;
use config::Config;

use std::rc::Rc;

use slog::{Logger, debug, o};

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

    let conn: Rc<_> = dbus::Connection::get_private(dbus::BusType::System)
        .context(ErrorKind::DBus)?
        .into();

    conn.add_match("type=signal,sender=org.surface.dtx,member=DetachStateChanged")
        .context(ErrorKind::DBus)?;

    conn.add_match("type=signal,sender=org.surface.dtx,member=PropertiesChanged")
        .context(ErrorKind::DBus)?;

    let mut rt = Runtime::new().context(ErrorKind::Runtime)?;
    let aconn = AConnection::new(conn, Handle::default(), &mut rt).unwrap();

    let messages = aconn.messages()
        .map_err(ErrorStr::from)
        .context(ErrorKind::DBus)?;

    let task = messages.for_each(move |m| {
        use dbus::SignalArgs;
        use dbus::stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged;

        debug!(logger, "signal received: {:#?}", m);

        if let Some(sig) = PropertiesPropertiesChanged::from_message(&m) {
            debug!(logger, "properties changed: {:#?}", sig);
        }

        Ok(())
    });

    rt.block_on(task).map_err(|_| Error::from(ErrorKind::DBus))?;
    Ok(())
}
