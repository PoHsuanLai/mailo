//! Confirmations: the title and body of a sheet that asks before something is done.
//!
//! Case: The question in Title Case as a title, the body in sentence case with full stops; the buttons are [`super::controls`].

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
