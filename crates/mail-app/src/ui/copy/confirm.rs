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

/// What Tidy Up will archive: the read Inbox conversations older than the age chosen.
pub fn tidy_up(count: usize) -> String {
    let stay = "Starred, pinned, snoozed and waiting ones aren’t archived.";
    match count {
        0 => "No read conversations that old in the Inbox.".to_owned(),
        1 => format!("1 read conversation that old in the Inbox. {stay}"),
        _ => format!("{count} read conversations that old in the Inbox. {stay}"),
    }
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
