//! The launcher entry's messages on the real session bus, from a thread of their own.
//!
//! The window only queues an [`Update`]; this thread reaches the bus when there is first
//! something to say and keeps the one connection for the life of the process, because a dock
//! forgets a count when its sender leaves the bus. A burst of counts is coalesced to the newest.
//! No bus (a bare X session, a container) or a refused emit costs one line on the terminal, not
//! one per count, and is tried again with the next count.

use super::unity::{Bus, INTERFACE, MEMBER, PATH, Update};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use zbus::blocking::Connection;
use zbus::names::BusName;
use zbus::zvariant::Value;

/// The queue to the thread that talks to the session bus.
pub(super) struct Session(Sender<Update>);

impl Session {
    /// Start the thread. Should it not start, counts are dropped: the window runs without them.
    pub(super) fn start() -> Session {
        let (queue, updates) = channel();
        if let Err(e) = std::thread::Builder::new()
            .name("mailo-launcher".to_owned())
            .spawn(move || serve(updates))
        {
            eprintln!("the launcher's unread count is off: {e}");
        }
        Session(queue)
    }
}

impl Bus for Session {
    fn emit(&self, update: Update) {
        // Only fails once the thread has gone, and then there is nobody to show it to.
        let _ = self.0.send(update);
    }
}

fn serve(updates: Receiver<Update>) {
    let mut connection: Option<Connection> = None;
    let mut complained = false;
    while let Ok(first) = updates.recv() {
        let update = updates.try_iter().last().unwrap_or(first);
        let sent = match &connection {
            Some(connection) => emit(connection, &update),
            None => Connection::session().and_then(|opened| {
                let sent = emit(&opened, &update);
                connection = Some(opened);
                sent
            }),
        };
        if let Err(e) = sent {
            connection = None;
            if !complained {
                eprintln!("could not show the unread count on the launcher: {e}");
                complained = true;
            }
        }
    }
}

fn emit(connection: &Connection, update: &Update) -> zbus::Result<()> {
    let properties: HashMap<&str, Value<'_>> = HashMap::from([
        ("count", Value::from(update.count)),
        ("count-visible", Value::from(update.count_visible)),
    ]);
    connection.emit_signal(
        None::<BusName<'_>>,
        PATH,
        INTERFACE,
        MEMBER,
        &(update.app_uri.as_str(), properties),
    )
}
