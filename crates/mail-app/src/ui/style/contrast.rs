//! WCAG contrast, so "unreadable in one theme" is a test failure rather than a bug report.
//!
//! The function lives in [`crate::contrast`] because palette derivation needs it at runtime.

pub(super) use crate::contrast::ratio;

#[cfg(test)]
mod tests {
    use super::ratio;

    const CASES: &[(&str, &str, f64)] = &[
        ("#000000", "#ffffff", 21.0),
        ("#ffffff", "#ffffff", 1.0),
        ("#777777", "#ffffff", 4.48),
        ("#FFF", "#000", 21.0),
    ];

    #[test]
    fn the_reference_pairs_match_wcag() {
        for &(fore, back, expected) in CASES {
            let Some(got) = ratio(fore, back) else {
                panic!("{fore} on {back} is not a hex pair");
            };
            assert!(
                (got - expected).abs() < 0.01,
                "{fore} on {back}: {got} differs from {expected}",
            );
        }
    }

    #[test]
    fn a_non_colour_has_no_ratio() {
        assert_eq!(ratio("not a colour", "#fff"), None);
    }
}
