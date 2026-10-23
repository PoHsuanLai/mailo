//! Push over an event source (RFC 8620 §7.3): the stream's events, and what a state change says.
//!
//! Sans-I/O like everything in this crate: bytes go in as they arrive, whole events come out.
//! The stream format is the HTML standard's `text/event-stream`: lines of `field: value`, a blank
//! line ending each event, `:` starting a comment, and any of CRLF, LF or CR ending a line.

use super::field::malformed;
use crate::ProtoError;
use serde_json::Value;

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The `event:` field.
    pub kind: EventKind,
    /// Every `data:` line, joined by line feeds.
    pub data: String,
}

/// What an event is, from its `event:` field.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventKind {
    /// `state`: a [`StateChange`] in the data.
    State,
    /// `ping`: the server keeping the stream open.
    Ping,
    /// No `event:` field at all, which the standard calls `message`.
    Message,
    /// Any other name.
    Other(String),
}

impl EventKind {
    fn named(name: &str) -> EventKind {
        match name {
            "" | "message" => EventKind::Message,
            "state" => EventKind::State,
            "ping" => EventKind::Ping,
            other => EventKind::Other(other.to_owned()),
        }
    }
}

/// The most an event may hold, its unfinished line or its `data:` lines together. A state change
/// is a few hundred bytes; a stream that grows one past this without ending it is refused rather
/// than kept in memory.
pub const MAX_EVENT: usize = 1024 * 1024;

/// An event stream being read.
#[derive(Debug, Default)]
pub struct EventStream {
    pending: Vec<u8>,
    kind: String,
    data: Vec<String>,
    held: usize,
}

impl EventStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next bytes; returns every event they completed, or an error once an event grows
    /// past [`MAX_EVENT`] without ending.
    ///
    /// A line split across two reads waits for the rest. A CR at the very end of what arrived
    /// waits too, since the LF that may follow it belongs to the same line ending.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Event>, ProtoError> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut start = 0;
        let mut at = 0;
        while at < self.pending.len() {
            let byte = self.pending[at];
            if byte != b'\n' && byte != b'\r' {
                at += 1;
                continue;
            }
            let end = at;
            let skip = match (byte, self.pending.get(at + 1)) {
                (b'\r', Some(b'\n')) => 2,
                (b'\r', None) => break,
                _ => 1,
            };
            let line = String::from_utf8_lossy(&self.pending[start..end]).into_owned();
            if let Some(event) = self.line(&line) {
                events.push(event);
            }
            at += skip;
            start = at;
        }
        self.pending.drain(..start);
        if self.pending.len() > MAX_EVENT || self.held > MAX_EVENT {
            return Err(malformed(format!(
                "a pushed event grew past {MAX_EVENT} bytes without ending"
            )));
        }
        Ok(events)
    }

    fn line(&mut self, line: &str) -> Option<Event> {
        if line.is_empty() {
            if self.data.is_empty() {
                self.kind.clear();
                return None;
            }
            let kind = std::mem::take(&mut self.kind);
            self.held = 0;
            return Some(Event {
                kind: EventKind::named(&kind),
                data: std::mem::take(&mut self.data).join("\n"),
            });
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.kind = value.to_owned(),
            "data" => {
                self.held += value.len();
                self.data.push(value.to_owned());
            }
            // `id` and `retry` steer a browser's reconnection; a watch reconnects on its own
            // schedule and resynchronises by state, so neither is needed here.
            _ => {}
        }
        None
    }
}

/// A `StateChange` (RFC 8620 §7.1): for each account, the new state of each type that changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    pub changed: Vec<(String, Vec<(String, String)>)>,
}

impl std::str::FromStr for StateChange {
    type Err = ProtoError;

    /// Parse the data of a `state` event.
    fn from_str(data: &str) -> Result<StateChange, ProtoError> {
        let value: Value = serde_json::from_str(data)
            .map_err(|e| malformed(format!("a push is not JSON: {e}")))?;
        if value.get("@type").and_then(Value::as_str) != Some("StateChange") {
            return Err(malformed("a push is not a StateChange"));
        }
        let changed = value
            .get("changed")
            .and_then(Value::as_object)
            .ok_or_else(|| malformed("a StateChange has no changed map"))?
            .iter()
            .map(|(account, types)| {
                let types = types
                    .as_object()
                    .map(|t| {
                        t.iter()
                            .filter_map(|(ty, state)| {
                                Some((ty.clone(), state.as_str()?.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (account.clone(), types)
            })
            .collect();
        Ok(StateChange { changed })
    }
}

impl StateChange {
    /// The new state of `kind` (`Email`, `Mailbox`) on `account`, if this change names one.
    pub fn state(&self, account: &str, kind: &str) -> Option<&str> {
        self.changed
            .iter()
            .find(|(a, _)| a == account)?
            .1
            .iter()
            .find(|(t, _)| t == kind)
            .map(|(_, s)| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_whole_however_the_bytes_are_split() {
        let stream = b"event: state\r\ndata: {\"a\":1}\r\n\r\n: comment\n\nevent: ping\ndata: {\"interval\":300}\r\r\n";
        // Every split point, so a CRLF broken across reads is exercised.
        for split in 0..stream.len() {
            let mut reader = EventStream::new();
            let mut events = reader.feed(&stream[..split]).unwrap();
            events.extend(reader.feed(&stream[split..]).unwrap());
            assert_eq!(
                events,
                vec![
                    Event {
                        kind: EventKind::State,
                        data: "{\"a\":1}".to_owned()
                    },
                    Event {
                        kind: EventKind::Ping,
                        data: "{\"interval\":300}".to_owned()
                    },
                ],
                "split at {split}"
            );
        }
    }

    #[test]
    fn an_event_that_never_ends_is_refused_past_the_limit() {
        let line = vec![b'x'; MAX_EVENT + 1];
        assert!(EventStream::new().feed(&line).is_err(), "an unended line");

        let mut reader = EventStream::new();
        let data = format!("data: {}\n", "x".repeat(MAX_EVENT / 4));
        for _ in 0..4 {
            reader.feed(data.as_bytes()).unwrap();
        }
        assert!(
            reader.feed(data.as_bytes()).is_err(),
            "data lines never ended"
        );

        let mut reader = EventStream::new();
        for _ in 0..8 {
            let events = reader.feed(format!("{data}\n").as_bytes()).unwrap();
            assert_eq!(events.len(), 1, "ended events do not add up");
        }
    }

    #[test]
    fn a_state_change_names_what_changed_per_account() {
        // RFC 8620 §7.1's example.
        let change: StateChange = r#"{"@type":"StateChange","changed":{
                "a456":{"Email":"d35ecb040aab","EmailDelivery":"428d565f2440","CalendarEvent":"87accfac587a"},
                "a901":{"Mailbox":"993f41b78d7e"}}}"#
            .parse()
            .unwrap();
        assert_eq!(change.state("a456", "Email"), Some("d35ecb040aab"));
        assert_eq!(change.state("a901", "Mailbox"), Some("993f41b78d7e"));
        assert_eq!(change.state("a901", "Email"), None);
        assert!(r#"{"@type":"Other"}"#.parse::<StateChange>().is_err());
    }
}
