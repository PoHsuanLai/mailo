//! Waiting between passes: JMAP's event source where the server offers one, raced against the
//! outbox exactly as an IMAP `IDLE` is.
//!
//! The stream is opened with `closeafter=no` and a ping, and read until a `state` event names
//! an email or mailbox state other than the one this client last synced to. A server may send
//! its current states the moment the stream opens; comparing against the stored cursor, rather
//! than waking on any event, is what lets that first event wake the watch only when mail really
//! did arrive between the pass and the wait — the same question `UIDNEXT` answers for IDLE.

use super::JmapEngine;
use crate::engine::wait::due_alarm;
use crate::{Cancel, RuntimeError, Woke};
use chrono::Utc;
use mail_proto::jmap::{EventStream, StateChange};
use std::time::Duration;

/// How often the server is asked to ping. Also how silence is judged: three missed pings and
/// the connection is taken for dead, however the network hid it.
const PING: Duration = Duration::from_secs(60);
/// Longest one stream is held before it is reopened, so a connection a proxy silently broke is
/// replaced within this even when pings are somehow getting through.
const HOLD: Duration = Duration::from_secs(25 * 60);

impl JmapEngine {
    /// Whether the session names an event source, so [`Self::wait`] is woken by the server.
    pub fn pushes(&self) -> bool {
        self.client
            .as_ref()
            .is_some_and(|c| c.session.event_source_url.is_some())
    }

    /// Wait for news from the server, a send coming due, or `poll` — whichever is first.
    ///
    /// Push where the session names an event source; a plain sleep otherwise, and after a
    /// stream that ended without news. Reads the wall clock, as a watch must.
    pub async fn wait(
        &mut self,
        cancel: &mut Cancel,
        poll: Duration,
    ) -> Result<Woke, RuntimeError> {
        let started = Utc::now();
        let (store, account, every) = (self.store.clone(), self.account, self.outbox_every);
        let alarm = || due_alarm(store.clone(), account, started, every);

        if self.pushes() {
            let pushed = self.pushed();
            let alarm = alarm();
            tokio::pin!(pushed, alarm);
            tokio::select! {
                result = &mut pushed => match result {
                    Ok(true) => return Ok(Woke::Mail),
                    // The stream ended without news: the interval below is the floor.
                    Ok(false) => {}
                    Err(e) => return Err(e),
                },
                () = &mut alarm => return Ok(Woke::Due),
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        return Err(RuntimeError::Cancelled);
                    }
                }
            }
        }

        tokio::select! {
            () = tokio::time::sleep(poll) => Ok(Woke::Interval),
            () = alarm() => Ok(Woke::Due),
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    Err(RuntimeError::Cancelled)
                } else {
                    Ok(Woke::Interval)
                }
            }
        }
    }

    /// Hold the event stream open until it reports a state this client has not synced.
    ///
    /// `Ok(false)` when it ends, goes silent, or has been held for [`HOLD`] without news.
    async fn pushed(&self) -> Result<bool, RuntimeError> {
        let Some(client) = &self.client else {
            return Ok(false);
        };
        let synced = self.cursor()?;
        // Its own client: the shared one times a request out after thirty seconds, and a stream
        // is one request that lasts as long as it is useful.
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| RuntimeError::Connect(format!("cannot build an HTTP client: {e}")))?;
        let Some(mut response) = client.events(&http, PING).await? else {
            return Ok(false);
        };
        let account = client.session.account.as_str();
        let mut stream = EventStream::new();
        let deadline = tokio::time::Instant::now() + HOLD;
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk,
                () = tokio::time::sleep(PING * 3) => return Ok(false),
                () = tokio::time::sleep_until(deadline) => return Ok(false),
            };
            let Some(bytes) = chunk.map_err(|e| RuntimeError::Io(format!("JMAP push: {e}")))?
            else {
                return Ok(false);
            };
            for event in stream.feed(&bytes) {
                if event.kind != "state" {
                    continue;
                }
                // A push that does not parse is a push; the pass that follows will say whether
                // anything changed.
                let Ok(change) = StateChange::parse(&event.data) else {
                    return Ok(true);
                };
                if news(&change, account, synced.as_ref()) {
                    return Ok(true);
                }
            }
        }
    }
}

/// Whether `change` names an email or mailbox state other than the one synced to.
fn news(change: &StateChange, account: &str, synced: Option<&(String, String)>) -> bool {
    let Some((email, mailbox)) = synced else {
        return true;
    };
    change.state(account, "Email").is_some_and(|s| s != email)
        || change
            .state(account, "Mailbox")
            .is_some_and(|s| s != mailbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(email: &str) -> StateChange {
        StateChange {
            changed: vec![(
                "A1".to_owned(),
                vec![("Email".to_owned(), email.to_owned())],
            )],
        }
    }

    #[test]
    fn only_a_state_not_yet_synced_is_news() {
        let synced = ("e1".to_owned(), "m1".to_owned());
        assert!(!news(&change("e1"), "A1", Some(&synced)));
        assert!(news(&change("e2"), "A1", Some(&synced)));
        // Another account's change is not this one's.
        assert!(!news(&change("e2"), "B2", Some(&synced)));
        // Nothing synced yet: anything is worth a pass.
        assert!(news(&change("e1"), "A1", None));
    }
}
