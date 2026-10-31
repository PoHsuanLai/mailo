//! What can be said about who sent a message, as the reader's head shows it: one glyph beside the
//! sender's name when the message is signed or encrypted, which opens a popover listing what is
//! known, and nothing at all for ordinary mail. Only what is wrong is said on the screen at rest:
//! a sender whose own domain says the server that sent it is not its gets one line under the
//! head, and a signature that does not check is the seal's banner (`pgp::Seal`).
//!
//! What the receiving server checked is in the stored raw message ([`mail_client::auth`]), so
//! finding it reads a blob. That is done off the thread that draws, once per message and body,
//! and remembered in the app's [`Looks`](mail_client::message::Looks): the reader and the card ask
//! for the same message and the second finds it already known. A message whose body is not here
//! shows nothing — fetching it to find out would be a POP3 `RETR`, which marks it read. What its
//! OpenPGP or S/MIME says is the seal's to find; this reads what the seal kept.

use crate::ui::menu::anchor_at;
use crate::ui::pgp::{Look, Mark, Said, Tone};
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::style::tokens::control_size::ControlSize;
use mail_client::auth::{Standing, standing};
use mail_domain::{BlobId, MessageId};
use mail_mime::AuthResults;
use mail_ui_model::copy;

/// What the head says about a message's sender, worked out from what is known so far.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::ui) struct Trust {
    /// The glyph, for a signed or encrypted message; none for ordinary mail.
    pub mark: Option<Mark>,
    /// What the glyph's popover lists: what the protection says, then what the receiving server
    /// checked.
    pub lines: Vec<Said>,
    /// What the receiving server's checks come to, once they are known.
    pub standing: Option<Standing>,
}

impl Trust {
    /// Whether the sender's domain says this message is not its own: the one thing about a
    /// sender said under the head.
    pub(in crate::ui) fn forged(&self) -> bool {
        self.standing == Some(Standing::Failed)
    }
}

/// What the head says for a message whose seal found `look` and whose receiving server checked
/// `results`, each `None` while it is not known. Pure, so every case is a table test.
///
/// A message that is neither signed nor encrypted has no glyph, whatever its checks say: a pass
/// is what mail is, and a failure is the line under the head. The protection's plain facts ("Not
/// encrypted", "Not signed") are left out of the popover: it lists what is, not what isn't.
pub(in crate::ui) fn trust_of(look: Option<&Look>, results: Option<&AuthResults>) -> Trust {
    let (mark, mut lines) = match look {
        Some(Look::Opened(opened)) => (
            Some(opened.mark),
            opened
                .said
                .iter()
                .filter(|said| said.tone != Tone::Plain)
                .cloned()
                .collect(),
        ),
        _ => (None, Vec::new()),
    };
    if let Some(results) = results {
        lines.push(Said {
            tone: Tone::Detail,
            text: copy::status::sender_checks(&mail_client::auth::sentence(results)),
        });
    }
    Trust {
        mark,
        lines,
        standing: results.map(standing),
    }
}

/// What the receiving server checked about `message` holding `body`: `None` until the answer is
/// known, then `Some(None)` for a message without a believed field. Looked up off the thread
/// that draws the first time; the caller is keyed on the message and its body, so a body
/// arriving asks again and one message's answer is never drawn under another.
pub(in crate::ui) fn use_checks(
    message: MessageId,
    body: Option<BlobId>,
) -> Signal<Option<Option<AuthResults>>> {
    let looks = super::reading::use_looks();
    let known = use_signal({
        let looks = looks.clone();
        move || body.and_then(|raw| looks.checks_cached(message, raw))
    });
    let _look = use_resource(move || {
        let client = crate::ui::live::client();
        let looks = looks.clone();
        let mut known = known;
        async move {
            let Some(raw) = body else {
                return;
            };
            if known.peek().is_some() {
                return;
            }
            let results =
                tokio::task::spawn_blocking(move || client.looking(&looks).checks(message, raw))
                    .await
                    .ok()
                    .flatten();
            known.set(Some(results));
        }
    });
    known
}

/// The glyph's name, as its tip and the popover's title say it.
fn named(mark: Mark) -> (&'static str, &'static str) {
    match mark {
        Mark::Encrypted => (copy::tooltips::ENCRYPTED, copy::titles::ENCRYPTED),
        Mark::Signed => (copy::tooltips::SIGNED, copy::titles::SIGNED),
        Mark::Unconfirmed => (copy::tooltips::NOT_VERIFIED, copy::titles::NOT_VERIFIED),
    }
}

/// The glyph for `mark`: a lock for what was encrypted, a key for a signature, and a plain note
/// for what could not be confirmed.
fn icon(mark: Mark) -> Icon {
    match mark {
        Mark::Encrypted => Icon::Lock,
        Mark::Signed => Icon::Key,
        Mark::Unconfirmed => Icon::Info,
    }
}

/// The `data-mark` word, for the stylesheet and the tests.
fn word(mark: Mark) -> &'static str {
    match mark {
        Mark::Encrypted => "encrypted",
        Mark::Signed => "signed",
        Mark::Unconfirmed => "unconfirmed",
    }
}

/// The glyph beside the sender's name, and the popover it opens: everything known about the
/// message's protection and its sender, a line a fact.
#[component]
pub(in crate::ui) fn TrustMark(mark: Mark, lines: Vec<Said>) -> Element {
    let mut open = use_signal(|| false);
    let mut at = use_signal(|| None::<MountedRef>);
    let (tip, title) = named(mark);
    rsx! {
        span { class: "trust", "data-mark": word(mark),
            Button {
                bezel: Bezel::Toolbar,
                size: ControlSize::Small,
                image: ImagePosition::Only,
                icon: Some(IconSource::Glyph(icon(mark))),
                label: tip,
                title: Some(tip.to_owned()),
                common: Common {
                    mounted: Some(EventHandler::new(move |event: MountedEvent| at.set(Some(MountedRef(event.data()))))),
                    ..Common::default()
                },
                onclick: move |_| open.toggle(),
            }
            if open() {
                Popover {
                    anchor: anchor_at(at()),
                    placement: Placement::new(Side::Bottom, Align::Start),
                    gap: Px(4.0),
                    onclose: move |()| open.set(false),
                    common: Common {
                        extra_class: ExtraClass::parse("trust-said").ok(),
                        aria_label: Some(title.to_owned()),
                        ..Common::default()
                    },
                    Label { text: title, style: LabelStyle::Headline }
                    for (at, said) in lines.into_iter().enumerate() {
                        {said_in_popover(at, said)}
                    }
                }
            }
        }
    }
}

/// One line of the popover, in the status colour its tone has: green for what checks, amber for
/// what cannot be told or is not to be believed, red for what failed, and the help type for the
/// details under them.
fn said_in_popover(at: usize, said: Said) -> Element {
    let severity = match said.tone {
        Tone::Good => Some(Severity::Ok),
        Tone::Unknown | Tone::Warn => Some(Severity::Warn),
        Tone::Bad => Some(Severity::Danger),
        Tone::Plain | Tone::Detail => None,
    };
    match said.tone {
        Tone::Detail => rsx! {
            Label { key: "{at}", text: said.text, role: LabelRole::Tertiary, style: LabelStyle::Footnote }
        },
        _ => rsx! { Label { key: "{at}", text: said.text, severity } },
    }
}

#[cfg(test)]
mod tests {
    use super::{Trust, trust_of};
    use crate::ui::pgp::{Look, Mark, Opened, Said, Scheme, Tone};
    use mail_client::auth::Standing;
    use mail_mime::{AuthResults, Check, Verdict};

    fn said(tone: Tone, text: &str) -> Said {
        Said {
            tone,
            text: text.to_owned(),
        }
    }

    fn opened(mark: Mark, said: Vec<Said>) -> Look {
        Look::Opened(Box::new(Opened {
            scheme: Scheme::OpenPgp,
            mark,
            said,
            shown: None,
        }))
    }

    fn dmarc(verdict: Verdict) -> AuthResults {
        AuthResults {
            dmarc: Some(Check {
                verdict,
                domain: Some("example.test".to_owned()),
            }),
            ..AuthResults::default()
        }
    }

    /// Ordinary mail has no glyph, passed or failed; a protected message has its mark, and its
    /// popover lists what is, never what isn't, then the server's checks.
    #[test]
    fn only_a_protected_message_has_a_glyph() {
        let passed = dmarc(Verdict::Pass);
        let failed = dmarc(Verdict::Fail);
        let signed = opened(
            Mark::Signed,
            vec![
                said(Tone::Plain, "Not encrypted"),
                said(Tone::Good, "Good signature by Ada"),
            ],
        );
        // (case, what the seal found, what the server checked, the glyph, the popover's line
        // count, whether it warns)
        type Case<'a> = (
            &'a str,
            Option<Look>,
            Option<&'a AuthResults>,
            Option<Mark>,
            usize,
            bool,
        );
        let cases: Vec<Case> = vec![
            ("nothing known yet", None, None, None, 0, false),
            (
                "plain, passed",
                Some(Look::Plain),
                Some(&passed),
                None,
                1,
                false,
            ),
            (
                "plain, failed",
                Some(Look::Plain),
                Some(&failed),
                None,
                1,
                true,
            ),
            (
                "locked",
                Some(Look::Locked {
                    key: mail_domain::Fingerprint::V4([7; 20]),
                    tried: crate::ui::pgp::Tried::Nothing,
                }),
                None,
                None,
                0,
                false,
            ),
            (
                "failed to open",
                Some(Look::Failed("no".to_owned())),
                None,
                None,
                0,
                false,
            ),
            (
                "signed, unchecked",
                Some(signed.clone()),
                None,
                Some(Mark::Signed),
                1,
                false,
            ),
            (
                "signed, passed",
                Some(signed),
                Some(&passed),
                Some(Mark::Signed),
                2,
                false,
            ),
        ];
        for (case, look, results, mark, lines, forged) in cases {
            let trust: Trust = trust_of(look.as_ref(), results);
            assert_eq!(trust.mark, mark, "{case}");
            assert_eq!(trust.lines.len(), lines, "{case}: {:?}", trust.lines);
            assert_eq!(trust.forged(), forged, "{case}");
            assert!(
                trust.lines.iter().all(|line| line.tone != Tone::Plain),
                "{case}: {:?}",
                trust.lines
            );
        }
        let trust = trust_of(None, Some(&passed));
        assert_eq!(trust.standing, Some(Standing::Passed));
        assert!(
            trust.lines[0].text.starts_with("Sender checks: ")
                && trust.lines[0].text.contains("DMARC pass"),
            "{:?}",
            trust.lines
        );
    }
}
