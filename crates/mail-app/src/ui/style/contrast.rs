//! WCAG contrast, so "unreadable in one theme" is a test failure rather than a bug report.

/// The WCAG 2.1 contrast ratio between two literal hex colours (`#rgb` or `#rrggbb`,
/// any case). `None` when either is not one: the caller must treat that as a failure
/// naming the token, never as a pair to skip.
pub(super) fn ratio(fore: &str, back: &str) -> Option<f64> {
    let fore = relative_luminance(fore)?;
    let back = relative_luminance(back)?;
    let (hi, lo) = if fore > back {
        (fore, back)
    } else {
        (back, fore)
    };
    Some((hi + 0.05) / (lo + 0.05))
}

fn relative_luminance(hex: &str) -> Option<f64> {
    let (red, green, blue) = channels(hex)?;
    Some(0.2126 * linearized(red) + 0.7152 * linearized(green) + 0.0722 * linearized(blue))
}

/// sRGB channel to linear light, WCAG 2.1.
fn linearized(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn channels(input: &str) -> Option<(u8, u8, u8)> {
    let hex = input.strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            Some((
                double_nibble(chars.next()?)?,
                double_nibble(chars.next()?)?,
                double_nibble(chars.next()?)?,
            ))
        }
        6 => Some((
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        )),
        _ => None,
    }
}

fn double_nibble(c: char) -> Option<u8> {
    let nibble = c.to_digit(16)?;
    u8::try_from(nibble * 16 + nibble).ok()
}

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
