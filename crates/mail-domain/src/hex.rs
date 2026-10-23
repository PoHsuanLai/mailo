//! Hexadecimal digits, read strictly. `u8::from_str_radix` also takes a leading `+`, so `+F`
//! would read as a byte: in a fingerprint that is a second spelling of a key, and in a `%`
//! escape a byte the writer never wrote.

/// The byte two hex digits spell, in either case; `None` for anything else, a sign included.
pub fn byte(pair: &[u8]) -> Option<u8> {
    match pair {
        [high, low] => Some((digit(*high)? << 4) | digit(*low)?),
        _ => None,
    }
}

/// The value of one hex digit.
pub fn digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::byte;

    #[test]
    fn two_hex_digits_and_nothing_else_are_a_byte() {
        let cases: [(&[u8], Option<u8>); 9] = [
            (b"00", Some(0)),
            (b"fF", Some(0xff)),
            (b"A9", Some(0xa9)),
            (b"+F", None),
            (b"-1", None),
            (b" 1", None),
            (b"g0", None),
            (b"1", None),
            (b"123", None),
        ];
        for (pair, want) in cases {
            assert_eq!(byte(pair), want, "{:?}", String::from_utf8_lossy(pair));
        }
    }
}
