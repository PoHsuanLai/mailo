//! A message's header in the reader, as Mail draws it: the sender's avatar, their name and
//! address on one line with the date at its end, and under it whom it was sent to.
//!
//! A conversation of one message has one header. In a longer one every message keeps a compact
//! header (a small avatar, the name, the date) and only the message the conversation opens on,
//! the newest, says whom it went to, carries the sender's checks and the offer to leave a list.

use super::super::text::{address, from_name};
use super::super::unsubscribe::Bodies;
use super::sender_face;
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::prelude::*;
use mail_domain::{Address, BlobId, Message, MessageId, ThreadId};

/// How much a message's header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Detail {
    /// The newest message: whom it went to, the sender's checks, the list's way out.
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

/// What the newest message's header carries besides the message: the reader's checks line and
/// logo are keyed by the message and its body, the list's way out by the conversation.
#[derive(Clone, PartialEq)]
pub(super) struct Extras {
    pub checked: (MessageId, Option<BlobId>),
    pub thread: ThreadId,
    pub leave_key: String,
    pub bodies: Bodies,
    pub revision: Option<Signal<u64>>,
}

/// One message's header.
#[component]
pub(super) fn MessageHead(
    message: Message,
    detail: Detail,
    /// When it came, briefly, and in full for the tooltip.
    when: (String, String),
    #[props(default)] extras: Option<Extras>,
) -> Element {
    let name = from_name(&message);
    let addr = address(&message);
    // A sender who gave no name is named by their address, once.
    let (name, addr) = if name.trim().is_empty() {
        (addr, String::new())
    } else {
        (name, addr)
    };
    let (short, full) = when;
    let face = sender_face(&message);
    let date = rsx! {
        Tooltip { text: full.clone(),
            // The full date is the time's name as well as its tip: what a screen reader says.
            time { class: "msg-when", aria_label: full,
                Label { text: short, role: LabelRole::Tertiary, style: LabelStyle::Footnote }
            }
        }
    };
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
        Detail::Full => {
            let to = recipients("To", &message.to);
            let cc = recipients("Cc", &message.cc);
            let avatar = match &extras {
                Some(Extras {
                    checked: (id, raw), ..
                }) => rsx! {
                    super::super::brand::ReaderAvatar { key: "{id}-{raw:?}", message: *id, body: *raw, from: message.from.email.clone(), face }
                },
                None => rsx! {
                    div { class: "reader-av",
                        Avatar { initial: face.initial, size: face.size, tone: face.tone }
                    }
                },
            };
            rsx! {
                header { class: "msg-head reader-meta", "data-detail": "full",
                    {avatar}
                    div { class: "reader-who",
                        div { class: "msg-line",
                            Label { text: name, style: LabelStyle::Headline, common: crate::ui::common::classed("ds-truncate") }
                            if !addr.is_empty() {
                                Label { text: addr, role: LabelRole::Secondary, style: LabelStyle::Footnote, common: crate::ui::common::classed("ds-truncate") }
                            }
                            {date}
                        }
                        if let Some(to) = to {
                            Label { text: to, role: LabelRole::Secondary, style: LabelStyle::Footnote, common: crate::ui::common::classed("ds-truncate") }
                        }
                        if let Some(cc) = cc {
                            Label { text: cc, role: LabelRole::Secondary, style: LabelStyle::Footnote, common: crate::ui::common::classed("ds-truncate") }
                        }
                        if let Some(Extras { checked: (id, raw), .. }) = &extras {
                            super::super::checks::SenderChecks { key: "{id}-{raw:?}", message: *id, body: *raw }
                        }
                    }
                    if let Some(Extras { thread, leave_key, bodies, revision, .. }) = extras {
                        super::super::unsubscribe::Leave { key: "{leave_key}", thread, bodies, revision }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Detail, MessageHead, detail_at, recipients};
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
        assert!(page.contains(">bob@example.test<"), "{page}");
        assert!(page.contains("aria-label=\"2023-11-15 07:13\""), "{page}");
        assert!(page.contains(">07:13<"), "{page}");
        assert!(page.contains(">To: Ada, Grace +3<"), "{page}");
        assert!(page.contains(">Cc: Edsger<"), "{page}");
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
