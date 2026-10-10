//! What `mailo unsubscribe` says: the ways out a message offers, and what taking one did.

use mail_core::error::CoreError;
use mail_core::unsubscribe::{Found, Outcome};
use mail_mime::Unsubscribe;
use std::fmt::Write as _;

/// Every way out the message offers, and which one `mailo unsubscribe` would take.
pub fn describe(found: &Found) -> String {
    let mut out = String::new();
    if let Some(id) = &found.list.id {
        match &id.description {
            Some(description) => {
                let _ = writeln!(out, "list    {description} <{}>", id.id);
            }
            None => {
                let _ = writeln!(out, "list    <{}>", id.id);
            }
        }
    }
    let _ = writeln!(out, "message {}", found.message);
    if found.list.unsubscribe.is_empty() {
        let _ = writeln!(out, "\n{}", CoreError::NothingOffered);
        return out;
    }
    let preferred = found.list.preferred();
    out.push('\n');
    for method in &found.list.unsubscribe {
        let mark = if Some(method) == preferred { "*" } else { " " };
        let _ = writeln!(out, "{mark} {}", method_line(method));
    }
    let _ = writeln!(
        out,
        "\n* is what `mailo unsubscribe {}` does",
        found.message
    );
    out
}

fn method_line(method: &Unsubscribe) -> String {
    match method {
        Unsubscribe::OneClick { url } => format!("one-click  POST {url}"),
        Unsubscribe::Mailto(mailto) => {
            let to: Vec<&str> = mailto.to.iter().map(|a| a.email.as_str()).collect();
            let mut line = format!("mail       to {}", to.join(", "));
            if !mailto.subject.is_empty() {
                let _ = write!(line, ", subject {:?}", mailto.subject);
            }
            line
        }
        Unsubscribe::Web { url } => format!("web page   {url} (shown, never opened)"),
    }
}

/// What taking the way out did, as the command says it.
pub fn report(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Unsubscribed { url } => {
            format!("unsubscribed: the list's server at {url} accepted it\n")
        }
        Outcome::Queued { draft } => {
            format!("queued an unsubscribe message ({draft}); it leaves on the next `mailo sync`\n")
        }
        Outcome::Page { url } => format!(
            "this list can only be left on its web page, which mailo does not open:\n  {url}\n"
        ),
    }
}
