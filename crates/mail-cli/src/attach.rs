//! What the attachment commands say: a message's attachments, and where one was written.

use mail_core::attach::{Listed, human_size};
use std::fmt::Write as _;
use std::path::Path;

/// The attachments on a message, as `mailo attachments` prints them.
pub fn listing(attachments: &[Listed]) -> String {
    if attachments.is_empty() {
        return "no attachments on that message\n".to_owned();
    }
    let mut out = String::new();
    for (index, attachment) in attachments.iter().enumerate() {
        let _ = writeln!(
            out,
            "{index}  {:>9}  {:<24}  {}",
            human_size(attachment.size),
            attachment.mime,
            attachment.name
        );
    }
    let _ = writeln!(
        out,
        "\nsave one with: mailo save <message-id> <number> [dir]"
    );
    out
}

/// `mailo save`, once the part is here and written: where it went.
pub fn saved(path: &Path) -> String {
    format!("Saved to {}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(name: &str, mime: &str, size: u64) -> Listed {
        Listed {
            name: name.to_owned(),
            mime: mime.to_owned(),
            size,
        }
    }

    #[test]
    fn the_listing_says_how_to_get_one() {
        let out = listing(&[listed("escape.pdf", "application/pdf", 2048)]);
        assert!(out.contains("escape.pdf"), "{out}");
        assert!(out.contains("application/pdf"), "{out}");
        assert!(out.contains("2.0 kB"), "{out}");
        assert!(out.contains("mailo save"), "it says how to get one: {out}");
    }

    #[test]
    fn a_message_with_nothing_attached_says_so() {
        let out = listing(&[]);
        assert!(out.contains("no attachments"), "{out}");
        assert!(!out.contains("mailo save"), "nothing to save: {out}");
    }

    #[test]
    fn the_saved_line_names_the_path() {
        assert_eq!(
            saved(Path::new("/tmp/report.pdf")),
            "Saved to /tmp/report.pdf"
        );
    }
}
