//! Field labels, placeholders and help lines.
//!
//! Case: Labels in sentence case ("Display name"); help lines are sentences, with a capital and a full stop.

/// ⌘F's field over the open conversation, while nothing is typed.
pub const FIND: &str = "Find";
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
/// A Space's page in Settings, and a label's editor: the row of its colours.
pub const COLOUR: &str = "Colour";
/// A Space's page in Settings: its own theme, System, Light or Dark.
pub const APPEARANCE: &str = "Appearance";
/// The saved-view sheet: what the view's list sorts by.
pub const SORT_BY: &str = "Sort by";

// Settings › General's footnotes, each said only where a choice is unclear without it.
/// An account's switch: keep every message and attachment on this computer.
pub const DOWNLOAD_ALL_MAIL: &str = "Download all mail";
/// An account's page: how often a timed pass checks it.
pub const CHECK_FOR_NEW_MAIL: &str = "Check for new mail";
/// An account's page: how far back its mail is downloaded.
pub const DOWNLOAD_MAIL_FROM: &str = "Download mail from";
/// A POP3 account's page: when the server's copy of a message goes.
pub const REMOVE_FROM_SERVER: &str = "Remove from server";
/// Settings › Rules: whose rules the page shows, when there are several accounts.
pub const ACCOUNT: &str = "Account";
/// The keys Settings lists: the row that imports an OpenPGP key.
pub const OPENPGP_KEY: &str = "OpenPGP Key";
/// The keys Settings lists: the row that imports an S/MIME certificate or identity.
pub const SMIME_CERTIFICATE: &str = "S/MIME Certificate";

// Choose Time…, under Snooze…: a time typed in the words `mailo snooze` takes.
/// Redirect…'s one field: whom the message goes to, as the To row takes them.
pub const REDIRECT_TO: &str = "Redirect to";
pub const SOMEONE: &str = "name@example.com";
/// Under Redirect…'s field, before anything is typed.
pub const REDIRECT_HELP: &str = "The message goes unchanged, from you. Gmail may show your address as its sender. Return sends it.";
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
/// Settings › General › VIPs: the address to make a VIP, and its example.
pub const VIP_ADDRESS: &str = "Email address";
pub const VIP_ADDRESS_EXAMPLE: &str = "name@example.com";
pub const PHONE: &str = "Phone";
pub const ORGANISATION: &str = "Organisation";
pub const FROM: &str = "From";
/// The composer's header row of the signature the message ends with.
pub const SIGNATURE: &str = "Signature";
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
/// The same row, when any one of them is enough.
pub const WHEN_ANY_MATCH: &str = "When any of these match";
/// The choice between the two, and its words.
pub const MATCHES: &str = "Match";
pub const ALL_OF_THEM: &str = "All";
pub const ANY_OF_THEM: &str = "Any";
// A rule's size and age conditions.
pub const SIZE: &str = "Size";
pub const AGE: &str = "Age";
pub const IS_LARGER: &str = "is larger than";
pub const IS_SMALLER: &str = "is smaller than";
pub const IS_NEWER: &str = "is newer than";
pub const IS_OLDER: &str = "is older than";
/// Under a rule's conditions when one is a size: the server may count a message's size otherwise.
pub const SIZE_COUNTED_HERE: &str =
    "Sizes are counted on this computer, so a server may count a message differently.";
/// A size condition's field, while it is empty.
pub const A_SIZE: &str = "5M";
/// An age condition's field, while it is empty.
pub const AN_AGE: &str = "30d";
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

/// The file dialog's filter for `/` › Image: the pictures a body can show.
pub const PICTURES: &str = "Pictures";
/// The name a picture pasted into the body goes by, on the draft and in the message.
pub const PASTED_IMAGE: &str = "Pasted image.png";

/// An account's page in Settings: the rows of its servers and sign-in, by what each is.
pub mod account {
    pub const RECEIVING: &str = "Receiving";
    pub const SENDING: &str = "Sending";
    pub const SIGNS_IN: &str = "Signs in";
    /// A Microsoft 365 account's relay, through which Graph pushes new mail
    /// (`settings_window::push_relay`): the row's label, and the field's name to a screen reader.
    pub const PUSH_RELAY: &str = "Push relay";
    /// The field's placeholder while no relay is set.
    pub const PUSH_RELAY_EXAMPLE: &str = "https://relay.example.com/path-secret";
    /// Under the field.
    pub const PUSH_RELAY_HELP: &str =
        "New mail arrives at once through a relay you deploy. Leave it empty to check on a timer.";

    // The rows the person edits on the account's page.
    /// The account's own name, written in the sidebar in place of its address.
    pub const DESCRIPTION: &str = "Description";
    /// The switch that pauses an account's syncing.
    pub const SYNC: &str = "Sync";
    pub const SYNC_THIS_ACCOUNT: &str = "Sync this account";
    pub const LABELS: &str = "Labels";
    pub const KEEP_LABELS_ON_THE_SERVER: &str = "Keep labels on the server";

    // The folders picked for roles, in the order the page lists them.
    pub const DRAFTS: &str = "Drafts";
    pub const SENT: &str = "Sent";
    pub const JUNK: &str = "Junk";
    pub const TRASH: &str = "Trash";
    pub const ARCHIVE: &str = "Archive";
    pub const SENDS_AS: &str = "Sends as";
    /// How much of the account's storage its server says is used.
    pub const STORAGE: &str = "Storage";
}

// A signature's page in Settings.
/// Its name's field, as a screen reader hears it.
pub const SIGNATURE_NAME: &str = "Signature name";
/// The switch that starts every new message with it.
pub const DEFAULT_FOR_NEW_MESSAGES: &str = "Default for new messages";
/// The switch that starts every reply and forward with it.
pub const DEFAULT_FOR_REPLIES: &str = "Default for replies";
// A person's card in Settings › Contacts, and a group's page.
pub const FIND_CONTACTS: &str = "Search contacts";
pub const ANOTHER_EMAIL: &str = "Add an address";
pub const ANOTHER_PHONE: &str = "Add a number";
pub const STREET: &str = "Street";
pub const CITY: &str = "City";
pub const REGION: &str = "State or region";
pub const POSTAL_CODE: &str = "Postal code";
pub const COUNTRY: &str = "Country";
pub const NOTE: &str = "Note";
pub const PHOTO: &str = "Photo";
pub const GROUP_NAME: &str = "Group name";
pub const ADD_MEMBERS: &str = "Add people by address";

/// The file dialog's filter for a contact's photo.
pub const PICTURE_FILES: &str = "Picture";

/// A Reply rule's field: the days it leaves before it answers the same sender again.
pub const REPLY_AGAIN_AFTER: &str = "Reply again after (days)";

/// What that field means.
pub const REPLY_AGAIN_FOOTER: &str =
    "A sender is answered once, then again only after this many days.";

/// The privacy relay's key, kept in the keyring.
pub const RELAY_KEY: &str = "Relay key";

/// What the relay key is, while none is kept.
pub const RELAY_KEY_HELP: &str =
    "The key the relay was set up with, or its token. It is kept in the keyring, not in a file.";

/// What the relay key says once one is kept.
pub const RELAY_KEY_KEPT: &str = "A key is kept in the keyring. Type another to replace it.";

/// The relay key's field, while none is kept.
pub const RELAY_KEY_PLACEHOLDER: &str = "Paste the key";

/// The relay key's field, once one is kept.
pub const RELAY_KEY_PLACEHOLDER_KEPT: &str = "Kept in the keyring";

/// What is wrong with the privacy relay or proxy chosen, said under the field that holds it. Until
/// it is mended, images wait rather than load straight from the sender.
pub fn route_problem(problem: crate::settings::RouteProblem) -> &'static str {
    use crate::settings::RouteProblem;
    match problem {
        RouteProblem::Relay => {
            "Use the relay\u{2019}s https:// address, such as https://camo.example.com."
        }
        RouteProblem::RelayKey => "Paste the key the relay was set up with.",
        RouteProblem::Proxy => {
            "Use an http://, https:// or socks5h:// address, such as socks5h://127.0.0.1:9050."
        }
    }
}
/// Settings › General: which calendar Add to Calendar uses.
pub const CALENDAR_FOR_INVITATIONS: &str = "Calendar for invitations";
/// The first of the calendar choices: whichever calendar takes events first.
pub const FIRST_CALENDAR: &str = "First available";
/// The proposed time's field: its label and the shape it is written in.
pub const NEW_TIME: &str = "New time";
pub const NEW_TIME_EXAMPLE: &str = "2026-11-02 14:30";
/// The card of an attendee's proposed time, and of a request for the latest version.
pub const PROPOSAL_NOTE: &str = "Proposes another time.";
pub const REFRESH_NOTE: &str = "Asks for the latest version.";
/// The tags at the head of those cards.
pub const PROPOSAL_TAG: &str = "Proposal";
pub const REFRESH_TAG: &str = "Refresh";
/// Settings › General's calendar row, before the calendars are listed: a calendar was chosen.
pub const CHOSEN_CALENDAR: &str = "Chosen";
/// The placeholder of the note beside an answer or a proposed time (its label is [`NOTE`]).
pub const NOTE_PLACEHOLDER: &str = "Add a note (optional)";
