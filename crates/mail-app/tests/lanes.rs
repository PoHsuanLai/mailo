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

#[path = "support/drive.rs"]
mod drive;
#[path = "support/row_menu.rs"]
mod row_menu;
#[path = "support/settle.rs"]
mod settle;

// What every lane drives and reads.
#[path = "lanes/body.rs"]
mod body;
#[path = "lanes/hands.rs"]
mod hands;
#[path = "lanes/look.rs"]
mod look;
#[path = "lanes/seed.rs"]
mod seed;
#[path = "lanes/settings.rs"]
mod settings;
#[path = "lanes/window.rs"]
mod window;

// 1–2: writing, attaching and sending; formatting.
#[path = "lanes/compose.rs"]
mod compose;
#[path = "lanes/format.rs"]
mod format;
