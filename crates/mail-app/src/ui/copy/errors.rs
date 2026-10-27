//! Errors and remedies the window words itself.
//!
//! Case: Sentence case, a capital and a full stop; say what to do when there is something to do.

/// Who a sign-in is for, when the account has no name to say.
pub const THIS_ACCOUNT: &str = "this account";

// A folder change refused.
pub const POP3_NO_FOLDERS: &str = "POP3 has no folders.";
pub const LOCAL_HAS_LABELS: &str = "Local mail has labels, not folders.";
pub const ACCOUNT_GONE: &str = "That account isn’t here any more.";

// A push relay's address refused, said under the field on the account's page.
pub const RELAY_NOT_AN_ADDRESS: &str =
    "Enter the relay\u{2019}s web address, starting with https://.";
pub const RELAY_NOT_HTTPS: &str = "The relay\u{2019}s address must start with https://.";
pub const RELAY_EXTRAS: &str =
    "The relay\u{2019}s address can\u{2019}t have a query, a fragment or a password.";
pub const RELAY_NOT_KEPT: &str = "The relay couldn\u{2019}t be saved. Try again.";

/// Add Account: a provider picked with an address at another provider's own domain (iCloud with
/// a Gmail address, as an Apple ID often is).
pub fn address_at_other_provider(
    address: &str,
    other: &str,
    picked: &str,
    own: &[String],
) -> String {
    let at: Vec<String> = own.iter().map(|domain| format!("@{domain}")).collect();
    let at = match at.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    };
    format!(
        "{address} belongs to {other}. {picked} needs your {picked} address, one ending in {at}."
    )
}

pub fn special_folder(path: &str, special: &str) -> String {
    format!("“{path}” is the account’s {special} folder and can’t be changed.")
}

pub fn folder_holds(path: &str, messages: &str) -> String {
    format!("“{path}” holds {messages} and was kept.")
}

pub fn folder_exists(name: &str) -> String {
    format!("A folder called “{name}” already exists.")
}

pub fn folder_unknown(name: &str) -> String {
    format!("“{name}” is no longer on the server. Sync to refresh.")
}

pub fn folder_has_children(name: &str) -> String {
    format!("“{name}” has folders inside it. Move or delete those first.")
}

pub fn bad_folder_name(name: &str, why: &str) -> String {
    format!("“{name}” can’t be a folder name: {why}.")
}

pub fn nothing_changed(why: &str) -> String {
    format!("Nothing changed: {why}")
}

// A new folder's name refused before the server is asked.
pub const TYPE_A_NAME: &str = "Type a name first.";
pub const NAME_STARTS_WITH_FLAG: &str = "A folder name can’t start with “\\”.";
pub const NO_SUBFOLDERS: &str = "This server has no subfolders.";

pub fn name_has_delimiter(delimiter: char) -> String {
    format!("A folder name can’t contain “{delimiter}”.")
}

/// A file picked to import that could not be read.
pub fn cannot_read(name: &str) -> String {
    format!("Can’t read {name}.")
}

/// A choice in Settings with nowhere to be written.
pub const NO_CONFIG_DIRECTORY: &str = "There’s nowhere to save this.";

/// A rule or vacation reply kept here that its server did not take.
pub fn not_on_server(why: &str) -> String {
    format!("Kept on this computer, but the server refused it: {why}")
}

/// Choose Time… refusing a time that has gone: snoozing until then would bring it straight back.
pub fn time_passed(when: &str) -> String {
    format!("{when} has passed. Pick a later time.")
}

/// Under [`super::titles::use_graph`]: why it is asked.
pub const IMAP_REFUSED: &str = "Microsoft refused IMAP for this account, perhaps because its \
    organisation turned it off. Microsoft Graph still works.";

/// A link's card, when the link's text names another place than the one it goes to.
pub fn link_goes_elsewhere(goes_to: &str, claims: &str) -> String {
    format!("This link says {claims} but goes to {goes_to}.")
}

/// Under a message's head when the sender's domain says the server that sent it is not its own.
pub fn may_not_be_from(sender: &str) -> String {
    format!("This may not be from {sender}.")
}

/// A Space's name field, emptied.
pub const SPACE_NEEDS_A_NAME: &str = "A Space needs a name.";

/// Why a `keyboard.json` naming one action twice cannot be used.
pub fn key_given_twice(action: &str) -> String {
    format!("{action} is given a key twice.")
}

/// Send pressed with nobody to send to: To's error.
pub const ADD_A_RECIPIENT: &str = "Add at least one person to send to.";

/// A saved file that is no longer where it was saved.
pub fn moved_or_deleted(name: &str) -> String {
    format!("{name} was moved or deleted.")
}

/// A file the system would not open, and why.
pub fn couldnt_open(why: &str) -> String {
    format!("Couldn’t open it: {why}")
}

// A failure whose own words are a library's: the detail goes to the log, and the window says
// one of these (`ui::remedy::plainly`).
/// Mail, a rule or a setting kept on this computer that could not be read.
pub const NOT_READ_HERE: &str = "This couldn’t be read from this computer. Try again.";
/// A change that could not be written on this computer.
pub const NOT_KEPT_HERE: &str = "The change couldn’t be saved on this computer. Try again.";
/// An attachment that could not be written to the downloads folder.
pub const ATTACHMENT_NOT_SAVED: &str = "The attachment couldn’t be saved.";
/// An attachment the system would not open.
pub const ATTACHMENT_NOT_OPENED: &str = "The attachment couldn’t be opened.";
/// A rule run over the mail already here that stopped part way.
pub const RULE_STOPPED: &str = "The rule stopped before it finished. Try again.";
/// A downloaded file the system would not open.
pub const FILE_NOT_OPENED: &str = "The file couldn’t be opened.";
/// A downloaded file the file manager would not show.
pub const FILE_NOT_SHOWN: &str = "The file couldn’t be shown in its folder.";

// Settings › Vacation Reply, refusing what its fields say.
/// A date field that does not read as a time.
pub fn not_a_time(text: &str) -> String {
    format!("“{text}” isn’t a time. Try 2026-10-08, 2026-10-08 09:00 or tomorrow.")
}
pub const REPLY_NEEDS_SUBJECT: &str = "The reply needs a subject.";
pub const REPLY_NEEDS_BODY: &str = "The reply needs something to say.";
/// “Until” already gone: `when` is it, in the clock's words.
pub fn until_passed(when: &str) -> String {
    format!("“Until” has passed: {when}.")
}
pub const UNTIL_BEFORE_FROM: &str = "“Until” has to be after “From”.";
pub fn not_an_address(word: &str) -> String {
    format!("“{word}” isn’t an address.")
}
pub const ADD_AN_ADDRESS: &str = "Add an address to reply for.";

// Remove Account, refused: why nothing was removed.
pub const ACCOUNT_ALREADY_GONE: &str = "This account is already gone.";
pub const LOCAL_NOT_REMOVABLE: &str = "Local mail can’t be removed like an account.";
pub const KEYRING_WOULD_NOT_FORGET: &str = "The keyring wouldn’t forget the saved sign-in, so nothing was removed. Unlock the keyring and try again.";
pub fn nothing_removed(why: &str) -> String {
    format!(
        "Nothing was removed: {why}. The saved sign-in is already forgotten, so it will ask you to sign in again."
    )
}

/// Why "Check for mail when quit" is off again: the system's reason, and what to do.
pub fn login_item_not_registered(why: &str) -> String {
    let why = why.trim_end_matches('.');
    format!("Mail won’t be checked while mailo is quit: {why}. Try again later.")
}

/// Why "Check for mail when quit" may still start at login: the system's reason.
pub fn login_item_not_removed(why: &str) -> String {
    let why = why.trim_end_matches('.');
    format!("mailo may still check for mail when you log in: {why}.")
}

/// What macOS asks before a login item runs.
pub const ALLOW_LOGIN_ITEM: &str = "To check for mail while mailo is quit, turn mailo on in System Settings › General › Login Items.";

/// `mailo open --inbox` with no account after it.
pub const OPEN_INBOX_NEEDS_ACCOUNT: &str =
    "open --inbox needs an account id: mailo open --inbox <account>";

/// `mailo open --inbox` with something after it that is not an account id.
pub fn not_an_account_id(given: &str) -> String {
    format!("{given:?} is not an account id")
}
