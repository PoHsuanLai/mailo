//! What `mailo snooze`, `mailo wake` and `mailo pin` say once they have done it.

use chrono::{DateTime, Local, Utc};
use mail_core::when::{Clock, Stamp, stamp};

/// `mailo snooze`: the instant the phrase meant, in the user's time and by their `clock`.
pub fn snoozed(at: DateTime<Utc>, clock: Clock) -> String {
    format!("snoozed until {}\n", stamp(at, &Local, Stamp::Full, clock))
}

/// `mailo wake`.
pub fn woke() -> String {
    "back in the inbox\n".to_owned()
}

/// `mailo archive`.
pub fn archived() -> String {
    "archived\n".to_owned()
}

/// `mailo pin`: whether the conversation is pinned now.
pub fn pinned(now_pinned: bool) -> String {
    if now_pinned {
        "pinned\n".to_owned()
    } else {
        "unpinned\n".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_and_wake_say_what_became_of_the_conversation() {
        assert_eq!(pinned(true), "pinned\n");
        assert_eq!(pinned(false), "unpinned\n");
        assert_eq!(woke(), "back in the inbox\n");
    }
}
