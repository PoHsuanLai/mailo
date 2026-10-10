//! Scenario lanes: whole flows through the real window on Blitz, driven only the way a person
//! drives it (keys, clicks, typing, paste, drag), with what they would see checked after each
//! step and what was stored or queued checked at the end.
//!
//! Every lane opens [`window::Window`]: the window over a store in a `TempDir`, on the harness's
//! virtual clock, every seam to the desktop (print dialog, file dialog, browser, notifications,
//! the save directory) a recorder. Nothing here reads or touches the real store, config,
//! keyring, downloads or network.
//!
//! A lane that stops at a gap carries `#[ignore = "gap: …"]` and keeps the asserts that describe
//! what should happen.

use crate::drive;
use crate::row_menu;
use crate::settle;

// What every lane drives and reads.
mod body;
mod hands;
mod look;
mod seed;
mod settings;
mod window;

// 1–2: writing, attaching and sending; formatting.
mod compose;
mod format;

// 3–6: a reply's round trip, an invitation, links, forwarding.
mod downloads;
mod forward;
mod invite;
mod links;
mod reply;

// 7–11: triage, send later and remind, contacts, rules, print.
mod contacts;
mod later;
mod print;
mod rules;
mod triage;
