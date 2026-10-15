//! A message's header in the reader: the sender's avatar, their name (their address its tooltip)
//! with the date at the line's end, and under it whom it was sent to, unless that was only you.
//!
//! A conversation of one message has one header. In a longer one every message keeps a compact
//! header (a small avatar, the name, the date) and only the message the conversation opens on,
//! the newest, says whom it went to and carries the trust glyph beside the sender's name
//! (`ui::checks`), with one line under the head when the sender may not be who they say.

use super::super::checks::{TrustMark, trust_of, use_checks};
use super::super::text::{address, from_name};
use super::sender_face;
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use mail_domain::{Address, BlobId, Message, MessageId};

/// How much a message's header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Detail {
    /// The newest message: whom it went to, and what is known about its sender.
    Full,
    /// An earlier message: who wrote it and when.
    Compact,
}

/// The detail for the message at `at` of `count`: the last is the full one.
pub(super) fn detail_at(at: usize, count: usize) -> Detail {
    if at + 1 == count {
        Detail::Full
    } else {
        Detail::Compact
    }
}

/// How many names a line of recipients spells out before it counts the rest.
const NAMED: usize = 2;

/// A line of recipients: `prefix` and the first [`NAMED`] of `people` by name (their address
/// when they gave none), the rest as a count; none for nobody.
pub(super) fn recipients(prefix: &str, people: &[Address]) -> Option<String> {
    if people.is_empty() {
        return None;
    }
    let names: Vec<&str> = people
        .iter()
        .take(NAMED)
        .map(|person| {
            person
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(person.email.as_str())
        })
        .collect();
    let more = people.len().saturating_sub(NAMED);
    let named = names.join(", ");
    Some(match more {
        0 => format!("{prefix}: {named}"),
        more => format!("{prefix}: {named} +{more}"),
    })
}

/// The To line, unless the message went to nobody but `yours` (the person's own addresses,
/// lowercased), which is what a message in their inbox goes to and needs no saying.
pub(super) fn to_line(message: &Message, yours: &[String]) -> Option<String> {
    let only_you = message.cc.is_empty()
        && message.to.len() == 1
        && yours.contains(&message.to[0].email.trim().to_lowercase());
    if only_you {
        return None;
    }
    recipients("To", &message.to)
}

/// What the newest message's header is keyed by in the reader: the message and its body, which
/// the trust glyph and the logo are looked up for, and the reader's `landed`, moved when the
/// seal finds what the message's protection says.
#[derive(Clone, PartialEq)]
pub(super) struct Extras {
    pub checked: (MessageId, Option<BlobId>),
    pub landed: Signal<u64>,
}

/// One message's header.
#[component]
pub(super) fn MessageHead(
    message: Message,
    detail: Detail,
    /// When it came, briefly, and in full for the tooltip.
    when: (String, String),
    #[props(default)] extras: Option<Extras>,
    /// The person's own addresses, lowercased: a To line of only them is not drawn.
    #[props(default)]
    yours: Vec<String>,
) -> Element {
    let (short, full) = when;
    let date = when_label(short.clone(), full.clone());
    let face = sender_face(&message);
    let (name, _) = named(&message);
    match detail {
        Detail::Compact => rsx! {
            header { class: "msg-head", "data-detail": "compact",
                Avatar { initial: face.initial, size: AvatarSize::Size22, tone: face.tone }
                div { class: "msg-line",
                    Label { text: name, style: LabelStyle::Headline, common: crate::ui::common::classed("ds-truncate") }
                    {date}
                }
            }
        },
        Detail::Full => match extras {
            Some(Extras {
                checked: (id, raw),
                landed,
            }) => rsx! {
                FullHead { key: "{id}-{raw:?}", message, when: (short, full), yours, landed }
            },
            None => full_head(&message, date, &yours, None, None),
        },
    }
}

/// The newest message's header in a reader: the logo, and what is known about the sender, looked
/// up off the thread that draws. Keyed by its parent on the message and its body, so a body
/// arriving asks again and one message's answer is never drawn under another.
#[component]
fn FullHead(
    message: Message,
    when: (String, String),
    yours: Vec<String>,
    landed: Signal<u64>,
) -> Element {
    let checks = use_checks(message.id, message.body.raw());
    let looks = super::use_looks();
    // Moved by the seal once it knows; what it found is kept in `looks`, and read here.
    let _ = landed();
    let look = crate::ui::pgp::last(&looks, message.id, message.body.raw());
    let results = checks();
    let trust = trust_of(look.as_ref(), results.as_ref().and_then(Option::as_ref));
    let (short, full) = when;
    let date = when_label(short, full);
    let face = sender_face(&message);
    let avatar = rsx! {
        super::super::brand::ReaderAvatar {
            key: "{message.id}-{message.body.raw():?}",
            message: message.id,
            body: message.body.raw(),
            from: message.from.email.clone(),
            face,
        }
    };
    let forged = trust
        .forged()
        .then(|| crate::ui::copy::errors::may_not_be_from(&named(&message).0));
    rsx! {
        {full_head(&message, date, &yours, Some(avatar), Some(trust))}
        if let Some(said) = forged {
            div { class: "trust-warn",
                InlineBanner { severity: Severity::Warn, icon: Some(Icon::TriangleAlert), text: said }
            }
        }
    }
}

/// What a header calls the sender, and the address its name's tooltip gives: a sender who gave
/// no name is named by their address, once, with no tooltip.
fn named(message: &Message) -> (String, String) {
    let name = from_name(message);
    let addr = address(message);
    if name.trim().is_empty() {
        (addr, String::new())
    } else {
        (name, addr)
    }
}

/// The date at the end of a header's line: brief, and in full as its tooltip and its name.
fn when_label(short: String, full: String) -> Element {
    rsx! {
        Tooltip { text: full.clone(),
            // The full date is the time's name as well as its tip: what a screen reader says.
            time { class: "msg-when", aria_label: full,
                Label { text: short, role: LabelRole::Tertiary, style: LabelStyle::Footnote }
            }
        }
    }
}

/// The full header's markup: the avatar (`avatar`, else the sender's initial), the name with the
/// trust glyph beside it and the date at the line's end, then whom it went to. `data-standing`
/// says the sender's checks once they are known.
fn full_head(
    message: &Message,
    date: Element,
    yours: &[String],
    avatar: Option<Element>,
    trust: Option<super::super::checks::Trust>,
) -> Element {
    let (name, addr) = named(message);
    // What the right click's contact editor starts from: the name the mail gave, and whom.
    let (given, email) = (from_name(message), message.from.email.clone());
    let face = sender_face(message);
    let to = to_line(message, yours);
    let cc = recipients("Cc", &message.cc);
    let avatar = avatar.unwrap_or_else(|| {
        rsx! {
            div { class: "reader-av",
                Avatar { initial: face.initial, size: face.size, tone: face.tone }
            }
        }
    });
    let standing = trust
        .as_ref()
        .and_then(|trust| trust.standing)
        .map(|standing| standing.word());
    let mark = trust.and_then(|trust| trust.mark.map(|mark| (mark, trust.lines)));
    rsx! {
        header { class: "msg-head reader-meta", "data-detail": "full", "data-standing": standing,
            {avatar}
            div { class: "reader-who",
                div { class: "msg-line",
                    if addr.is_empty() {
                        super::super::contacts::SenderName { shown: name, in_mail: given, address: email }
                    } else {
                        Tooltip { text: addr,
                            super::super::contacts::SenderName { shown: name, in_mail: given, address: email }
                        }
                    }
                    if let Some((mark, lines)) = mark {
                        TrustMark { mark, lines }
                    }
                    {date}
                }
                if let Some(to) = to {
                    Label { text: to, role: LabelRole::Secondary, style: LabelStyle::Footnote, common: crate::ui::common::classed("ds-truncate") }
                }
                if let Some(cc) = cc {
                    Label { text: cc, role: LabelRole::Secondary, style: LabelStyle::Footnote, common: crate::ui::common::classed("ds-truncate") }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Detail, MessageHead, detail_at, recipients, to_line};
    use dioxus::prelude::*;
    use mail_domain::*;

    fn person(name: &str, email: &str) -> Address {
        Address {
            name: (!name.is_empty()).then(|| name.to_owned()),
            email: email.to_owned(),
        }
    }

    #[test]
    fn recipients_are_named_then_counted() {
        let ada = person("Ada", "ada@example.test");
        let bob = person("Bob", "bob@example.test");
        let nameless = person("", "carol@example.test");
        let dan = person("Dan", "dan@example.test");
        let eve = person(" ", "eve@example.test");
        let cases: &[(&str, Vec<Address>, Option<&str>)] = &[
            ("nobody", vec![], None),
            ("one", vec![ada.clone()], Some("To: Ada")),
            ("two", vec![ada.clone(), bob.clone()], Some("To: Ada, Bob")),
            (
                "an address for a missing name",
                vec![nameless.clone()],
                Some("To: carol@example.test"),
            ),
            (
                "the rest counted",
                vec![ada.clone(), bob.clone(), nameless, dan, eve],
                Some("To: Ada, Bob +3"),
            ),
        ];
        for (name, people, want) in cases {
            assert_eq!(recipients("To", people).as_deref(), *want, "{name}");
        }
    }

    #[test]
    fn only_the_newest_message_says_everything() {
        let cases: &[(usize, usize, Detail)] = &[
            (0, 1, Detail::Full),
            (0, 3, Detail::Compact),
            (1, 3, Detail::Compact),
            (2, 3, Detail::Full),
        ];
        for &(at, count, want) in cases {
            assert_eq!(detail_at(at, count), want, "{at} of {count}");
        }
    }

    fn message(to: Vec<Address>, cc: Vec<Address>) -> Message {
        Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: crate::ui::fixtures::acct_account(),
            key: MessageKey::Rfc("m1@example.test".to_owned()),
            date: chrono::Utc::now(),
            from: person("Bob", "bob@example.test"),
            reply_to: vec![],
            to,
            cc,
            bcc: vec![],
            subject: "hi".to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: None,
            read: ReadState::Read,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Absent,
            attachments: vec![],
        }
    }

    fn drawn(message: Message, detail: Detail) -> String {
        #[component]
        fn Head(message: Message, detail: Detail) -> Element {
            rsx! {
                ds::prelude::Ds {
                    appearance: ds::prelude::Appearance::default(),
                    material: ds::prelude::Material::Window,
                    MessageHead { message, detail, when: ("07:13".to_owned(), "2023-11-15 07:13".to_owned()) }
                }
            }
        }
        let mut dom = dioxus_core::VirtualDom::new_with_props(Head, HeadProps { message, detail });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn the_full_header_says_who_when_and_to_whom_once() {
        let to = vec![
            person("Ada", "ada@example.test"),
            person("Grace", "grace@example.test"),
            person("Alan", "alan@example.test"),
            person("", "carol@example.test"),
            person("Dan", "dan@example.test"),
        ];
        let page = drawn(
            message(to, vec![person("Edsger", "edsger@example.test")]),
            Detail::Full,
        );
        assert_eq!(page.matches("class=\"ds-avatar\"").count(), 1, "{page}");
        assert_eq!(page.matches(">Bob<").count(), 1, "{page}");
        // The address is the name's tooltip, not a word on the line.
        assert!(!page.contains(">bob@example.test<"), "{page}");
        assert!(page.contains("aria-label=\"2023-11-15 07:13\""), "{page}");
        assert!(page.contains(">07:13<"), "{page}");
        assert!(page.contains(">To: Ada, Grace +3<"), "{page}");
        assert!(page.contains(">Cc: Edsger<"), "{page}");
    }

    #[test]
    fn a_message_to_only_you_has_no_to_line() {
        let yours = vec!["me@example.test".to_owned()];
        let me = person("Me", "Me@Example.test");
        let ada = person("Ada", "ada@example.test");
        type Case<'a> = (&'a str, Vec<Address>, Vec<Address>, Option<&'a str>);
        let cases: &[Case] = &[
            ("only you", vec![me.clone()], vec![], None),
            (
                "you and another",
                vec![me.clone(), ada.clone()],
                vec![],
                Some("To: Me, Ada"),
            ),
            (
                "you, copied to another",
                vec![me.clone()],
                vec![ada.clone()],
                Some("To: Me"),
            ),
            ("someone else", vec![ada.clone()], vec![], Some("To: Ada")),
        ];
        for (name, to, cc, want) in cases {
            let got = to_line(&message(to.clone(), cc.clone()), &yours);
            assert_eq!(got.as_deref(), *want, "{name}");
        }
    }

    #[test]
    fn a_compact_header_is_who_and_when() {
        let page = drawn(
            message(vec![person("Ada", "ada@example.test")], vec![]),
            Detail::Compact,
        );
        assert_eq!(page.matches("class=\"ds-avatar\"").count(), 1, "{page}");
        assert!(page.contains(">Bob<") && page.contains(">07:13<"), "{page}");
        assert!(!page.contains("To: "), "{page}");
        assert!(!page.contains(">bob@example.test<"), "{page}");
    }
}
