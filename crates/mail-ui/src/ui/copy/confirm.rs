//! Confirmations: the title and body of a sheet that asks before something is done.
//!
//! Case: The question in Title Case as a title, the body in sentence case with full stops; the buttons are [`super::controls`].

/// Asked on a Space's page in Settings before the Space goes: its name, or "this Space".
pub fn delete_space(name: &str) -> String {
    match name.trim() {
        "" => "Delete This Space?".to_owned(),
        name => format!("Delete “{name}”?"),
    }
}

/// What deleting a Space leaves: the mail, which is the accounts'.
pub const DELETE_SPACE_KEEPS: &str = "Your mail won’t be deleted. This can’t be undone.";

/// The last line of an alert before something that has no way back.
pub const CANT_BE_UNDONE: &str = "This can’t be undone.";

/// Asked in a folder's row before the folder goes with the mail in it.
pub fn delete_folder(name: &str, messages: &str) -> String {
    format!("Delete “{name}” and the {messages} in it?")
}

/// Tidy Up with nothing to archive at the age chosen.
pub const TIDY_NOTHING: &str = "Nothing that old in the Inbox.";
/// Under Tidy Up's list: what it leaves alone.
pub const TIDY_SKIPS: &str = "Skips starred, pinned, snoozed and waiting mail.";
/// Under Tidy Up's list, when it names only some of what it archives.
pub fn tidy_more(count: usize) -> String {
    format!("{count} more")
}

/// Under [`super::titles::SETTINGS_UNREADABLE`]: each file and why, then what each button does.
pub fn settings_unreadable(files: &[(String, String)]) -> String {
    let said: Vec<String> = files
        .iter()
        .map(|(name, why)| format!("{name}: {why}."))
        .collect();
    format!(
        "{} Use Defaults sets each file aside as “.bad” and starts from defaults. Quit leaves them as they are.",
        said.join(" ")
    )
}

/// Asked once at launch while another application opens email links; Settings › General's row
/// asks the same later.
pub const MAKE_DEFAULT_MAIL: &str = "Make mailo Your Default Mail App?";
/// Under [`MAKE_DEFAULT_MAIL`]: what changes, and where to change it back.
pub const MAKE_DEFAULT_MAIL_BODY: &str =
    "Email links will open a new message in mailo. You can change this in Settings.";

/// Asked on an account's page before Remove takes it from this computer.
pub fn remove_account(address: &str) -> String {
    format!("Remove {address}?")
}
/// What Remove takes and leaves: `mail` is how many messages are here, `server` what becomes of
/// the server's copy ([`MAIL_STAYS_ON_SERVER`] or [`only_copy`]).
pub fn remove_account_body(mail: &str, server: &str) -> String {
    format!(
        "Its {mail} on this computer, folders, rules and saved sign-in are removed. {server} Keys and certificates aren’t removed."
    )
}
pub const MAIL_STAYS_ON_SERVER: &str = "Mail on the server isn’t deleted.";
/// A POP3 server that deletes what it hands over has no copy left.
pub fn only_copy(mail: &str) -> String {
    format!("This server deletes mail once downloaded, so these {mail} are the only copy.")
}
