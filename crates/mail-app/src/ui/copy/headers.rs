//! Menu and section headers and group names: the small heading over rows.
//!
//! Case: One or two words, Title Case when it names a place ("Drafts", "Scheduled"), sentence case otherwise.

// The sidebar's lists.
/// Pinned conversations, people and searches, at the top.
pub const FAVOURITES: &str = "Favourites";
/// The conversations opened and the drafts put aside, until the person clears them.
pub const TODAY: &str = "Today";
pub const PLACES: &str = "Places";
pub const LABELS: &str = "Labels";
pub const VIEWS: &str = "Views";
pub const FOLDERS: &str = "Folders";

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

// Settings › General's two groups.
pub const READING: &str = "Reading";
pub const MAIL: &str = "Mail";
/// An account's keys and certificates, on its page in Settings.
pub const ENCRYPTION: &str = "Encryption";
/// An account's servers and sign-in, on its page in Settings: the one place a protocol is named.
pub const SERVERS: &str = "Servers";

// ⌘K's sections, in the order it draws them.
/// Where the person was: the conversations opened and the drafts put aside.
pub const RECENT: &str = "Recent";
/// The places, and with something typed the people, it names.
pub const PLACES_AND_PEOPLE: &str = "Places and People";
pub const COMMANDS: &str = "Commands";
pub const TEMPLATES: &str = "Templates";
/// Under a Space's name in ⌘K, saying what it is.
pub const SPACE: &str = "Space";
