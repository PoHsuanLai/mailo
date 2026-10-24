//! The small wordings every front end and every crate above this one writes the same way: what
//! stands in for a missing subject, and how a byte count reads. One copy, so they cannot drift.

use std::borrow::Cow;

/// What stands where a subject would be, when there is none.
pub const NO_SUBJECT: &str = "(no subject)";

/// `subject` trimmed, or [`NO_SUBJECT`] when nothing is left of it.
///
/// Trimmed because a subject of spaces is as empty to the eye as none, and the copies that did
/// not trim showed a blank row.
pub fn subject_or_none(subject: &str) -> Cow<'_, str> {
    match subject.trim() {
        "" => Cow::Borrowed(NO_SUBJECT),
        said => Cow::Borrowed(said),
    }
}

/// A size a person can read.
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    match bytes {
        0..KB => format!("{bytes} B"),
        KB..MB => format!("{:.1} kB", bytes as f64 / KB as f64),
        _ => format!("{:.1} MB", bytes as f64 / MB as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_subject_is_the_stand_in_and_a_real_one_is_trimmed() {
        assert_eq!(subject_or_none(""), NO_SUBJECT);
        assert_eq!(subject_or_none("  \t"), NO_SUBJECT);
        assert_eq!(subject_or_none("  lunch "), "lunch");
    }

    #[test]
    fn sizes_step_up_at_a_kilobyte_and_a_megabyte() {
        const CASES: &[(u64, &str)] = &[
            (0, "0 B"),
            (512, "512 B"),
            (1024, "1.0 kB"),
            (1536, "1.5 kB"),
            (1024 * 1024, "1.0 MB"),
            (5 * 1024 * 1024 + 512 * 1024, "5.5 MB"),
        ];
        for (bytes, shown) in CASES {
            assert_eq!(human_size(*bytes), *shown, "{bytes}");
        }
    }
}
