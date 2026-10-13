//! Menu and section headers and group names: the small heading over rows.
//!
//! Case: One or two words, Title Case when it names a place ("Drafts", "Scheduled"), sentence case otherwise.

// The sidebar's menu.
pub const TODAY: &str = "Today";
/// The drafts put aside in this Space.
pub const DRAFTS: &str = "Drafts";
/// The messages waiting for their time.
pub const SCHEDULED: &str = "Scheduled";

// The sidebar's lists.
pub const PLACES: &str = "Places";
pub const LABELS: &str = "Labels";
pub const VIEWS: &str = "Views";
pub const PINNED: &str = "Pinned";
pub const FOLDERS: &str = "Folders";
// The "+" menu's groups: accounts to bring into the Space, and a new one.
pub const SHOW_HERE: &str = "Show Here";
pub const NEW: &str = "New";

// The list's bands.
pub const YESTERDAY: &str = "Yesterday";
pub const THIS_WEEK: &str = "This week";
pub const OLDER: &str = "Older";
pub const UNREAD: &str = "Unread";
pub const READ: &str = "Read";
pub const NO_LABEL: &str = "No label";
/// A search's best matches, over the rest.
pub const TOP_RESULTS: &str = "Top results";
/// The rest of a search's matches, under its top results.
pub const NEWEST_FIRST: &str = "Newest first";
/// What a search of the server found that the list does not hold: the band's header and the
/// chip on each of its rows.
pub const FROM_SERVER: &str = "Server";
