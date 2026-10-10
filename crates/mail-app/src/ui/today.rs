//! Today: threads opened recently, and drafts put aside, per Space.
//!
//! quire's kit (`ds::components::app::spaces::Today`) over mail's items: a thread is what opens,
//! a draft is what parks. Stored in `today.json` under the state directory. An entry is a
//! shortcut in the sidebar's menu (the chevron at the foot, which lists the newest few); Clear
//! Today, or letting it expire after [`IDLE`], removes the shortcut and does not touch the mail. A parked draft stays until it is reopened, sent or discarded,
//! because the draft itself is in the store and this is only the way back to it.
//!
//! mailo's clock is chrono's (a test's is virtual); the kit's is seconds, made here by [`at`].

use chrono::{DateTime, Utc};
use ds::components::app::spaces::{self, Epoch};
use ds_settings::SpacesStorage;
use mail_domain::{DraftId, ThreadId};
use std::path::Path;

pub use ds::components::app::spaces::IDLE;

/// The threads opened and the drafts parked, in every Space.
pub type Today = spaces::Today<ThreadId, DraftId>;

/// One opened thread, in one Space.
pub type Entry = spaces::Entry<ThreadId>;

/// A draft put aside with Esc, in one Space.
pub type Parked = spaces::Parked<DraftId>;

/// `time` as the kit counts it.
pub fn at(time: DateTime<Utc>) -> Epoch {
    Epoch(time.timestamp())
}

/// Where Today lives: the state directory alone.
fn storage(dir: &Path) -> SpacesStorage {
    SpacesStorage::at(None, Some(dir.to_owned()))
}

/// The stored list, or an empty one when there is no file or it cannot be read. An entry that
/// cannot be read drops on its own.
pub fn load(dir: &Path) -> Today {
    storage(dir).load_today()
}

/// Write `today` to `dir/today.json`, creating `dir` if needed.
pub fn save(dir: &Path, today: &Today) -> Result<(), String> {
    storage(dir)
        .save_today(today)
        .map_err(|why| why.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Today, at, load, save};
    use crate::ui::space::SpaceId;
    use chrono::{DateTime, Utc};
    use mail_domain::{DraftId, ThreadId};
    use uuid::Uuid;

    fn time(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap_or_else(|| panic!("1_700_000_000 is a valid timestamp"))
            + chrono::Duration::minutes(minutes)
    }

    /// The file mailo wrote before the kit, as it was: positional Spaces, RFC3339 times, the
    /// parked drafts under `drafts`, and the items under `thread` and `draft`.
    const BEFORE: &str = r#"{
        "entries": [
            {"space": 1, "thread": "00000000-0000-0000-0000-000000000004", "last_opened": "2023-11-14T22:13:20Z"},
            {"space": 0, "thread": "00000000-0000-0000-0000-000000000005", "last_opened": "2023-11-14T22:43:20Z"}
        ],
        "drafts": [
            {"space": 1, "draft": "00000000-0000-0000-0000-000000000009", "title": "Plans", "parked": "2023-11-14T22:13:20Z"}
        ]
    }"#;

    #[test]
    fn the_file_mailo_wrote_before_reads_as_it_was() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("today.json"), BEFORE).unwrap_or_else(|e| panic!("{e}"));
        let today = load(dir.path());
        let thread = |n| ThreadId::from_uuid(Uuid::from_u128(n));
        let opened: Vec<(SpaceId, ThreadId)> = today
            .entries
            .iter()
            .map(|entry| (entry.space, entry.item))
            .collect();
        assert_eq!(opened, [(SpaceId(1), thread(4)), (SpaceId(0), thread(5))]);
        assert_eq!(today.entries[0].last_opened, at(time(0)));
        let parked = today.parked_in(SpaceId(1));
        assert_eq!(parked.len(), 1);
        assert_eq!(parked[0].item, DraftId::from_uuid(Uuid::from_u128(9)));
        assert_eq!(parked[0].title, "Plans");
    }

    #[test]
    fn today_round_trips_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(dir.path()), Today::default());
        let mut today = Today::default();
        today.opened(
            SpaceId(0),
            ThreadId::from_uuid(Uuid::from_u128(4)),
            at(time(0)),
        );
        today.park(
            SpaceId(1),
            DraftId::from_uuid(Uuid::from_u128(9)),
            "Plans",
            at(time(5)),
        );
        save(dir.path(), &today).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(dir.path()), today);
    }
}
