//! The recipient box: addresses written as a person types them, and back.

use mail_domain::Address;

/// Render addresses back into something a text box can hold.
pub fn join_addresses(list: &[Address]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(name) if !name.is_empty() => format!("{name} <{}>", a.email),
            _ => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a recipient box into addresses.
///
/// Accepts `a@b.test`, `Name <a@b.test>` and `"Name" <a@b.test>`, separated by commas or
/// semicolons. Empty entries are skipped, because a trailing comma is what typing looks like.
///
/// An entry with no `@` is an error rather than a guess. The alternative — dropping it, or
/// appending a default domain — means the user sees their recipient vanish, or sends to
/// someone they did not name. Both are silent; an error is not.
pub fn parse_addresses(input: &str) -> Result<Vec<Address>, String> {
    let mut out: Vec<Address> = Vec::new();
    for entry in split_entries(input) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let address = parse_one(entry)?;
        // First spelling wins, matching `mail_mime::posting`. Two RCPT TO lines for one mailbox
        // are two deliveries.
        if !out
            .iter()
            .any(|kept| kept.email.eq_ignore_ascii_case(&address.email))
        {
            out.push(address);
        }
    }
    Ok(out)
}

/// Split a recipient box on separators that are not inside a display name or an address.
///
/// Not `split([',', ';'])`. `"Lovelace, Ada" <ada@example.test>` is one recipient, and every
/// other mail client writes a display name containing a comma exactly that way — so a naive
/// split turns a pasted recipient into two, one of which is not an address at all. The comma
/// inside quotes is the single most common character this has to *not* split on.
fn split_entries(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut angled) = (0usize, false, false);
    for (i, ch) in input.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            // An unbalanced '<' would otherwise swallow the rest of the box; '>' only closes
            // what a '<' opened, so a stray '>' cannot re-enable splitting that was never off.
            '<' if !quoted => angled = true,
            '>' if !quoted => angled = false,
            ',' | ';' if !quoted && !angled => {
                out.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&input[start..]);
    out
}

fn parse_one(entry: &str) -> Result<Address, String> {
    let (name, email) = match (entry.rfind('<'), entry.rfind('>')) {
        (Some(open), Some(close)) if close > open => {
            let name = entry[..open].trim().trim_matches('"').trim();
            (
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_owned())
                },
                entry[open + 1..close].trim(),
            )
        }
        _ => (None, entry),
    };
    if email.is_empty() {
        return Err(format!("{entry:?} has no address"));
    }
    // One `@`, with something either side. Not a full RFC 5322 validation: that accepts things
    // no mail server does, and rejecting what a user's own server accepts is worse than letting
    // the server answer. This catches the mistake people actually make, which is a missing `@`.
    let mut halves = email.split('@');
    let (local, domain) = (halves.next().unwrap_or(""), halves.next().unwrap_or(""));
    if local.is_empty() || domain.is_empty() || halves.next().is_some() {
        return Err(format!("{email:?} is not an email address"));
    }
    if email.contains(char::is_whitespace) {
        return Err(format!("{email:?} contains a space"));
    }
    Ok(Address {
        name,
        email: email.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    #[test]
    fn a_recipient_box_accepts_the_shapes_people_type() {
        assert_eq!(
            parse_addresses("ada@example.test").unwrap(),
            vec![addr(None, "ada@example.test")]
        );
        assert_eq!(
            parse_addresses("Ada Lovelace <ada@example.test>").unwrap(),
            vec![addr(Some("Ada Lovelace"), "ada@example.test")]
        );
        // Quoted display names are what other clients paste in.
        assert_eq!(
            parse_addresses("\"Lovelace, Ada\" <ada@example.test>").unwrap(),
            vec![addr(Some("Lovelace, Ada"), "ada@example.test")]
        );
    }

    #[test]
    fn a_comma_inside_a_display_name_does_not_split_the_recipient() {
        // Every other client writes "Last, First" this way, so this arrives by paste constantly.
        // Splitting here turns one recipient into two, one of which is not an address at all.
        let parsed = parse_addresses("\"Lovelace, Ada\" <ada@x.test>, bob@x.test").unwrap();
        assert_eq!(parsed.len(), 2, "{parsed:?}");
        assert_eq!(parsed[0].name.as_deref(), Some("Lovelace, Ada"));
        assert_eq!(parsed[0].email, "ada@x.test");
        assert_eq!(parsed[1].email, "bob@x.test");
    }

    #[test]
    fn a_separator_inside_the_address_itself_does_not_split_either() {
        let parsed = parse_addresses("<a;b@x.test>").unwrap();
        assert_eq!(parsed.len(), 1, "{parsed:?}");
    }

    #[test]
    fn commas_and_semicolons_both_separate() {
        // Outlook produces semicolons. A user pasting from it should not have to know.
        let by_comma = parse_addresses("a@x.test, b@x.test").unwrap();
        let by_semi = parse_addresses("a@x.test; b@x.test").unwrap();
        assert_eq!(by_comma, by_semi);
        assert_eq!(by_comma.len(), 2);
    }

    #[test]
    fn a_trailing_comma_is_what_typing_looks_like() {
        // The box is parsed on every keystroke to show errors. If "a@x.test," were an error,
        // the composer would shout at the user in the middle of typing the second recipient.
        assert_eq!(parse_addresses("a@x.test,").unwrap().len(), 1);
        assert_eq!(parse_addresses("  ,, a@x.test , ,").unwrap().len(), 1);
        assert!(parse_addresses("").unwrap().is_empty());
        assert!(parse_addresses("   ").unwrap().is_empty());
    }

    #[test]
    fn something_that_is_not_an_address_is_an_error_not_a_guess() {
        // Dropping it makes the recipient vanish; appending a default domain sends to someone
        // the user did not name. Both are silent, which is what makes them worse than an error.
        for bad in ["ada", "ada@", "@example.test", "a@b@c", "ada example.test"] {
            assert!(
                parse_addresses(bad).is_err(),
                "{bad:?} was accepted as an address"
            );
        }
    }

    #[test]
    fn a_bad_entry_fails_the_whole_box_rather_than_half_of_it() {
        // Sending to "everyone I could parse" is the failure mode this prevents.
        let err = parse_addresses("good@x.test, nonsense, other@x.test").unwrap_err();
        assert!(err.contains("nonsense"), "{err}");
    }

    #[test]
    fn one_mailbox_twice_is_one_recipient() {
        let parsed = parse_addresses("Ada <ada@x.test>, ADA@X.TEST").unwrap();
        assert_eq!(parsed.len(), 1, "{parsed:?}");
        // The first spelling wins, so the display name the user typed survives.
        assert_eq!(parsed[0].name.as_deref(), Some("Ada"));
    }

    #[test]
    fn what_the_box_shows_parses_back_to_what_it_held() {
        let original = vec![
            addr(Some("Ada Lovelace"), "ada@example.test"),
            addr(None, "bob@example.test"),
        ];
        assert_eq!(
            parse_addresses(&join_addresses(&original)).unwrap(),
            original
        );
    }
}
