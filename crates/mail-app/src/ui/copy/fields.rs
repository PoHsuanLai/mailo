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

/// The composer's body, while nothing is written in it.
pub const WRITE_OR_SLASH: &str = "Write, or type / for more";

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
/// Settings › General: which application opens email links.
pub const DEFAULT_MAIL_APP: &str = "Default mail app";
/// The application that opens email links, when it is mailo.
pub const THIS_APP: &str = "mailo";
/// The application that opens email links, when there is none.
pub const NO_MAIL_APP: &str = "None";

// Settings › Vacation Reply's form.
/// The switch, and its row.
pub const VACATION_REPLY: &str = "Vacation reply";
pub const AWAY_SUBJECT_EXAMPLE: &str = "Away until the 12th";
/// The reply's body: its row, its accessible name and its placeholder.
pub const REPLY: &str = "Reply";
pub const REPLY_BODY: &str = "The reply’s text";
pub const REPLY_TEXT: &str = "Reply text";
/// The addresses the reply answers mail for.
pub const ANSWERS_MAIL_TO: &str = "Answers mail to";
/// The name an account's own address goes out under, on its page in Settings.
pub const FULL_NAME: &str = "Full name";
/// Where replies to an alias go, when not to the alias itself.
pub const REPLY_TO: &str = "Reply-To";
/// What the name fields hold before anything is typed.
pub const FULL_NAME_EXAMPLE: &str = "Ada Lovelace";
/// What an alias's address field holds before anything is typed.
pub const ALIAS_EXAMPLE: &str = "ada@example.com";
/// What an alias's Reply-To field holds before anything is typed: the alias itself.
pub const REPLY_TO_EXAMPLE: &str = "The alias itself";
/// When it starts and stops; `FROM` is the start's.
pub const UNTIL: &str = "Until";
pub const FROM_EXAMPLE: &str = "now, or 2026-10-08";
pub const UNTIL_EXAMPLE: &str = "turned off, or monday";

/// An account's page in Settings: the rows of its servers and sign-in, by what each is.
pub mod account {
    pub const RECEIVING: &str = "Receiving";
    /// What POP3 does with what it downloads.
    pub const DOWNLOADED_MAIL: &str = "Downloaded mail";
    pub const SENDING: &str = "Sending";
    pub const SIGNS_IN: &str = "Signs in";
    /// A Microsoft 365 account's relay, through which Graph pushes new mail
    /// (`settings_window::push_relay`): the row's label, and the field's name to a screen reader.
    pub const PUSH_RELAY: &str = "Push relay";
    /// The field's placeholder while no relay is set.
    pub const PUSH_RELAY_EXAMPLE: &str = "https://relay.example.com";
    /// Under the field.
    pub const PUSH_RELAY_HELP: &str =
        "New mail arrives at once through a relay you deploy. Leave it empty to check on a timer.";
}
