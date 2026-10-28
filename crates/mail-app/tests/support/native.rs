//! What the native tests say the same way: how long, which row, and a click that waits.

use crate::drive::Drive;
use ds_harness::Harness;
use std::time::Duration;

/// `n` milliseconds, for the harness's clock.
pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The `n`th item of the list (1-based).
pub fn item(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

/// The `n`th conversation of the list, as its row draws it.
pub fn thread(n: usize) -> String {
    format!("{} .ds-thread", item(n))
}

/// Click the middle of `selector` and let the window answer.
pub fn click(harness: &mut Harness, selector: &str) {
    let at = harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(300));
}
