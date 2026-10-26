//! Writing a calendar object back out: the one frame every object this crate makes shares, so a
//! reply, a counter-proposal and a calendar file for a store cannot disagree on its head, on
//! where a zone's rules go, or on how a line ends.

use super::ZoneRules;
use crate::line::{self, ContentLine};

/// A `VCALENDAR` of `events` (each the lines of one `VEVENT`, `BEGIN` and `END` left out), with
/// `METHOD` when there is one, and the rules of every zone a line names, each once.
pub(super) fn object(
    method: Option<&str>,
    product: &str,
    zones: &[ZoneRules],
    events: &[Vec<ContentLine>],
) -> String {
    let mut out = String::new();
    let mut head = vec![
        ContentLine::new("BEGIN", "VCALENDAR"),
        ContentLine::new("PRODID", product),
        ContentLine::new("VERSION", "2.0"),
        ContentLine::new("CALSCALE", "GREGORIAN"),
    ];
    if let Some(method) = method {
        head.push(ContentLine::new("METHOD", method));
    }
    for line in &head {
        out.push_str(&line::write(line));
    }
    // Every zone a carried time names, so the reader's calendar can read it (RFC 5545 §3.6.5),
    // each once.
    let mut named: Vec<&str> = Vec::new();
    for tzid in events.iter().flatten().flat_map(|l| l.values("TZID")) {
        if !named.contains(&tzid) {
            named.push(tzid);
        }
    }
    for zone in named
        .iter()
        .filter_map(|tzid| zones.iter().find(|z| z.tzid == *tzid))
    {
        for zone_line in &zone.lines {
            out.push_str(&line::write(zone_line));
        }
    }
    for event in events {
        out.push_str(&line::write(&ContentLine::new("BEGIN", "VEVENT")));
        for property in event {
            out.push_str(&line::write(property));
        }
        out.push_str(&line::write(&ContentLine::new("END", "VEVENT")));
    }
    out.push_str(&line::write(&ContentLine::new("END", "VCALENDAR")));
    out
}
