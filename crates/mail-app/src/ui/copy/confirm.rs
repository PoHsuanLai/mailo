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
pub const DELETE_SPACE_KEEPS: &str =
    "Your mail is not affected. Only the Space’s name, colour and accounts shown go.";

/// Asked in a folder's row before the folder goes with the mail in it.
pub fn delete_folder(name: &str, messages: &str) -> String {
    format!("Delete “{name}” and the {messages} in it?")
}

/// What Tidy Up will archive: the read Inbox conversations older than the age chosen.
pub fn tidy_up(count: usize) -> String {
    let stay = "Starred, pinned, snoozed and waiting ones stay.";
    match count {
        0 => "No read conversations in the Inbox are that old.".to_owned(),
        1 => format!("1 read conversation in the Inbox is that old. {stay}"),
        _ => format!("{count} read conversations in the Inbox are that old. {stay}"),
    }
}
