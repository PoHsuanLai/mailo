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

/// A size a person can read, in the largest unit it fills, to a tenth: "512 B", "1.5 kB",
/// "5.5 MB", "15.0 GB".
///
/// One convention for every size mailo writes, attachments and storage alike: a kB is 1024 bytes,
/// as the files in the person's own file manager count them. The unit is chosen after rounding to
/// a tenth, so a size that rounds up to 1024 of one unit is said in the next, never as "1024.0 kB".
pub fn human_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("kB", 1 << 10),
        ("MB", 1 << 20),
        ("GB", 1 << 30),
        ("TB", 1 << 40),
    ];
    if bytes < UNITS[0].1 {
        return format!("{bytes} B");
    }
    // The tenths of a unit that `bytes` comes to, rounded to nearest.
    let tenths = |size: u64| (u128::from(bytes) * 10 + u128::from(size) / 2) / u128::from(size);
    let (unit, tenths) = UNITS
        .iter()
        .map(|&(unit, size)| (unit, tenths(size)))
        .find(|&(_, tenths)| tenths < 1024 * 10)
        // Past what the units name, the last says it.
        .unwrap_or_else(|| (UNITS[3].0, tenths(UNITS[3].1)));
    format!("{}.{} {unit}", tenths / 10, tenths % 10)
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
    fn sizes_step_up_at_each_unit_and_never_say_1024_of_one() {
        const CASES: &[(u64, &str)] = &[
            (0, "0 B"),
            (512, "512 B"),
            (1023, "1023 B"),
            (1024, "1.0 kB"),
            (1536, "1.5 kB"),
            (300 << 20, "300.0 MB"),
            // Rounds to 1023.9 kB, and one byte more to a whole MB, never "1024.0 kB".
            (1_048_524, "1023.9 kB"),
            (1_048_525, "1.0 MB"),
            (1024 * 1024, "1.0 MB"),
            (5 * 1024 * 1024 + 512 * 1024, "5.5 MB"),
            (1288 << 20, "1.3 GB"),
            ((1 << 30) - 1, "1.0 GB"),
            (15 << 30, "15.0 GB"),
            ((1 << 40) - 1, "1.0 TB"),
            (2 << 40, "2.0 TB"),
            (1500 << 40, "1500.0 TB"),
            (u64::MAX, "16777216.0 TB"),
        ];
        for (bytes, shown) in CASES {
            assert_eq!(human_size(*bytes), *shown, "{bytes}");
        }
    }
}
