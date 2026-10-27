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

/// Asked on a POP3 account's page before Remove from Server is set to anything but Never.
pub const REMOVE_FROM_SERVER: &str = "Remove Mail From the Server?";

/// What choosing Remove from Server after `days` days does: a day count, never zero.
pub fn remove_from_server_after(days: u16) -> String {
    let days = days.max(1);
    format!(
        "Once a message is {days} {} old and kept on this computer, the next check removes the server’s copy. {CANT_BE_UNDONE}",
        if days == 1 { "day" } else { "days" }
    )
}

/// What choosing Remove from Server when deleted here does.
pub const REMOVE_FROM_SERVER_WHEN_DELETED: &str =
    "A message you delete here is removed from the server at the next check. This can’t be undone.";

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

/// Delete Contact…'s question.
pub fn delete_contact(name: &str) -> String {
    match name.trim() {
        "" => "Delete This Contact?".to_owned(),
        name => format!("Delete “{name}”?"),
    }
}

/// Delete Group…'s question.
pub fn delete_group(name: &str) -> String {
    match name.trim() {
        "" => "Delete This Group?".to_owned(),
        name => format!("Delete “{name}”?"),
    }
}

/// What deleting a card leaves.
pub const DELETE_CONTACT_KEEPS: &str = "Mail from and to them is kept. This can’t be undone.";
/// What deleting a group leaves.
pub const DELETE_GROUP_KEEPS: &str = "The people in it stay in Contacts. This can’t be undone.";

/// Asked before Apply Rules on conversations when a rule of the account replies or forwards.
pub const APPLY_RULES_SENDS: &str = "Apply Rules and Send Mail?";

/// What applying rules that reply or forward does.
pub const APPLY_RULES_SENDS_BODY: &str = "Some of your rules reply to or forward mail. Applying them to these conversations can send mail. Mail that comes from a list, a machine or yourself is never answered.";
