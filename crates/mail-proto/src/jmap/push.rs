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

/// An event stream being read.
#[derive(Debug, Default)]
pub struct EventStream {
    pending: Vec<u8>,
    kind: String,
    data: Vec<String>,
}

impl EventStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next bytes; returns every event they completed.
    ///
    /// A line split across two reads waits for the rest. A CR at the very end of what arrived
    /// waits too, since the LF that may follow it belongs to the same line ending.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
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
        events
    }

    fn line(&mut self, line: &str) -> Option<Event> {
        if line.is_empty() {
            if self.data.is_empty() {
                self.kind.clear();
                return None;
            }
            let kind = std::mem::take(&mut self.kind);
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
            "data" => self.data.push(value.to_owned()),
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
            let mut events = reader.feed(&stream[..split]);
            events.extend(reader.feed(&stream[split..]));
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
