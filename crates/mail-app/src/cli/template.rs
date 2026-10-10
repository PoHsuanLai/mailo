//! What the template commands say: the template kept, the draft started from one, and the list.

use mail_domain::{Address, Draft, Template};
use std::fmt::Write as _;

/// `mailo template save`.
pub fn saved(kept: &Template) -> String {
    format!(
        "template {}\n  name    {}\n\nstart a message from it with: mailo template use {}\n",
        kept.id, kept.name, kept.id
    )
}

/// `mailo template use`: the new draft's id first, on a line of its own.
pub fn started(draft: &Draft) -> String {
    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    let _ = writeln!(out, "  subject {}", or_none(&draft.subject));
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        let _ = writeln!(
            out,
            "\nthe template names nobody to send to: start it again with --to someone@example.com"
        );
    } else {
        let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    }
    out
}

/// `mailo template list`, from `(account address, template)` pairs.
pub fn listed(all: &[(String, Template)]) -> String {
    if all.is_empty() {
        return "no templates. Keep a draft as one with: mailo template save <draft-id>\n"
            .to_owned();
    }
    let mut out = String::new();
    for (address, template) in all {
        let _ = writeln!(
            out,
            "{}  {}  ({}) {}",
            template.id,
            template.name,
            address,
            or_none(&template.subject)
        );
    }
    out
}

fn or_none(subject: &str) -> &str {
    if subject.is_empty() {
        "(no subject)"
    } else {
        subject
    }
}

fn addresses(list: &[Address]) -> String {
    list.iter()
        .map(|a| a.email.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
