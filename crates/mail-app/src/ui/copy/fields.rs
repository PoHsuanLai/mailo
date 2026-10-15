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

// Settings › General's footnotes, each said only where a choice is unclear without it.
pub const LOGOS_UNVERIFIED: &str = "Logos can’t be verified on this computer yet.";
pub const SEARCH_SERVER_MEANS: &str = "Search server also looks in your servers’ mail as you type.";
/// The trusted senders' page, while nobody is trusted.
pub const HOW_TO_TRUST: &str = "Choose Always Load From on a message’s image banner.";
/// An account's switch: keep every message and attachment on this computer.
pub const DOWNLOAD_ALL_MAIL: &str = "Download all mail";
/// Settings › Rules: whose rules the page shows, when there are several accounts.
pub const ACCOUNT: &str = "Account";
/// Under the rules: why their order is theirs to choose.
pub const RULES_RUN_IN_ORDER: &str = "Rules act on new mail, in this order.";
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
