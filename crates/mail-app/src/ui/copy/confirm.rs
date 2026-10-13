//! Confirmations: the title and body of a sheet that asks before something is done.
//!
//! Case: The question in Title Case as a title, the body in sentence case with full stops; the buttons are [`super::controls`].

/// Asked in a folder's row before the folder goes with the mail in it.
pub fn delete_folder(name: &str, messages: &str) -> String {
    format!("Delete “{name}” and the {messages} in it?")
}
