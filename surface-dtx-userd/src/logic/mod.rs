mod core;
use self::core::Core;

mod types;
pub use self::types::{CancelReason, Event};


use crate::utils::task::JoinHandleExt;

use anyhow::{Context, Result};

use dbus::message::MatchRule;
use dbus_tokio::connection;

use futures::prelude::*;

use tracing::trace;


pub async fn run() -> Result<()> {
    // set up and start D-Bus connections (system and user-session)
    let (sys_rsrc, sys_conn) = connection::new_system_sync()
        .context("Failed to connect to D-Bus (system)")?;

    let (ses_rsrc, ses_conn) = connection::new_session_sync()
        .context("Failed to connect to D-Bus (session)")?;

    let sys_rsrc = sys_rsrc.map(|e| Err(e).context("D-Bus connection error (system)"));
    let ses_rsrc = ses_rsrc.map(|e| Err(e).context("D-Bus connection error (session)"));

    let mut dsys_task = tokio::spawn(sys_rsrc).guard();
    let mut dses_task = tokio::spawn(ses_rsrc).guard();

    // set up D-Bus message listener task
    let mut main_task = tokio::spawn(async move {
        let mut core = Core::new(ses_conn);

        let mr = MatchRule::new_signal("org.surface.dtx", "Event");
        let (_msgs, mut stream) = sys_conn
            .add_match(mr).await
            .context("Failed to set up D-Bus connection")?
            .msg_stream();

        while let Some(mut msg) = stream.next().await {
            trace!(target: "sdtxu::core", message = ?msg, "message received");

            let msg = msg.as_result().context("D-Bus remote error")?;
            let evt = Event::try_from_message(msg)?;
            drop(msg);

            if let Some(evt) = evt {
                core.handle(evt).await?;
            }
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
