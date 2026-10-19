//! Field labels, placeholders and help lines.
//!
//! Case: Labels in sentence case ("Display name"); help lines are sentences, with a capital and a full stop.

/// A folder's name, being written: the field's label and a rename's placeholder.
pub const FOLDER_NAME: &str = "Folder name";
/// A new folder's row and its field's placeholder.
pub const NEW_FOLDER: &str = "New folder";
/// Tidy Up's choice of how old a conversation must be.
pub const OLDER_THAN: &str = "Older than";
/// A person's name in the contact editor: the field's label and placeholder.
pub const NAME: &str = "Name";
/// A Space's page in Settings: its name's field, and what the field is called to a screen reader.
pub const SPACE_NAME: &str = "Space name";
/// A Space's page in Settings: the row of its colours.
pub const COLOUR: &str = "Colour";
/// A Space's page in Settings: its own theme, System, Light or Dark.
pub const APPEARANCE: &str = "Appearance";

// Settings › General's footnotes, each said only where a choice is unclear without it.
/// An account's switch: keep every message and attachment on this computer.
pub const DOWNLOAD_ALL_MAIL: &str = "Download all mail";
/// Settings › Rules: whose rules the page shows, when there are several accounts.
pub const ACCOUNT: &str = "Account";
/// The keys Settings lists: the row that imports an OpenPGP key.
pub const OPENPGP_KEY: &str = "OpenPGP Key";
/// The keys Settings lists: the row that imports an S/MIME certificate or identity.
pub const SMIME_CERTIFICATE: &str = "S/MIME Certificate";

// Choose Time…, under Snooze…: a time typed in the words `mailo snooze` takes.
pub const SNOOZE_UNTIL: &str = "Snooze until";
pub const A_TIME: &str = "tomorrow 9";
/// Under the field while nothing is typed.
pub const TYPE_A_TIME: &str = "Type a time, like tomorrow 9.";

/// Under the field, the time a typed phrase reads as: "Back tomorrow 09:00".
pub fn back_at(when: &str) -> String {
    format!("Back {when}")
}

/// ⌘K's field, while it is empty.
pub const SEARCH_EVERYTHING: &str = "Search mail, places and commands";

// Peek's facts about a calendar file, a contact card or a forwarded message.
pub const WHEN: &str = "When";
pub const WHERE: &str = "Where";
pub const ORGANISER: &str = "Organiser";
pub const REPEATS: &str = "Repeats";
pub const EMAIL: &str = "Email";
pub const PHONE: &str = "Phone";
pub const ORGANISATION: &str = "Organisation";
pub const FROM: &str = "From";
pub const TO: &str = "To";
pub const DATE: &str = "Date";

// A rule's conditions, built from menus: what is looked at, how, and (for a state) which.
pub const SUBJECT: &str = "Subject";
pub const MESSAGE_TEXT: &str = "Message text";
pub const LABEL: &str = "Label";
pub const ATTACHMENT: &str = "Attachment";
pub const STATUS: &str = "Status";
pub const RECEIVED_BEFORE: &str = "Received before";
pub const RECEIVED_AFTER: &str = "Received after";
pub const CONTAINS: &str = "contains";
pub const LACKS: &str = "doesn’t contain";
pub const IS: &str = "is";
pub const IS_NOT: &str = "isn’t";
pub const HAS_ONE: &str = "has one";
pub const HAS_NONE: &str = "has none";
pub const UNREAD: &str = "Unread";
pub const READ: &str = "Read";
pub const STARRED: &str = "Starred";
pub const NOT_STARRED: &str = "Not starred";
pub const PINNED: &str = "Pinned";
pub const SNOOZED: &str = "Snoozed";
/// A date condition's field, while it is empty.
pub const A_DATE: &str = "2026-10-08";
/// The row a rule's conditions sit in: every one has to match.
pub const WHEN_ALL_MATCH: &str = "When all of these match";
/// The checkbox under a rule's actions, with how many conversations it would act on now.
pub fn apply_to_existing(count: Option<usize>) -> String {
    match count {
        None => "Also apply to mail already here".to_owned(),
        Some(1) => "Also apply to the 1 conversation that matches now".to_owned(),
        Some(n) => format!("Also apply to the {n} conversations that match now"),
    }
}
/// A rule's condition written as text: the field's name and its example.
pub const CONDITION: &str = "Condition";
pub const CONDITION_EXAMPLE: &str = "from:bank.example subject:statement";
