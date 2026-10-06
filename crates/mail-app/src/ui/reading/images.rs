//! Whether a message opens with its remote images loaded, before anyone presses "Show images".
//!
//! A remote image is a read receipt: fetching it tells the sender's server when the message was
//! read and from which network address. So the reader blocks them unless the person has said
//! otherwise, either for the conversation in front of them (`Shell::show_remote_images`, the
//! "Show images" press) or ahead of time, in `reading.remote_images` and the trusted senders
//! (`crate::settings::ReadingSettings`). [`auto_allow`] is the second half; the reader takes
//! either.
//!
//! What it trusts is an address, so it only believes an address it can tie to its sender: a
//! display name that claims a brand the address does not belong to ([`mail_core::trust::spoof`]),
//! or a message whose receiving server did not say DMARC passed for the From domain, is never
//! loaded on the list's word. Anyone can write any address in From; the list would otherwise be a
//! list of addresses to forge.

use crate::settings::LoadRemoteImages;
use mail_domain::{MailboxRole, Message};
use mail_store::SqliteStore;

/// Whether a sender is one whose word about who they are cannot be taken: the display name
/// claims a brand the address is not theirs, or the receiving server did not say DMARC passed for
/// the address's domain. A message with no `Authentication-Results` this client believes has not
/// passed.
pub(super) fn suspicious(display: Option<&str>, email: &str, auth_passed: bool) -> bool {
    mail_core::trust::spoof(display, email).is_some() || !auth_passed
}

/// Whether a message from `email` (named `display`) opens with its remote images loaded, under
/// `mode` and the `trusted` senders, without anyone pressing "Show images".
///
/// - `Ask`: never. Each conversation is asked about, as before the setting existed.
/// - `Always`: yes, except in Junk.
/// - `Trusted`: only for an address on the list (in any case), and only when the sender is not
///   [`suspicious`].
///
/// Junk (`in_junk`) is always asked about, whatever the mode: a message filed there is one
/// somebody already doubted. `auth_passed` is asked only when the answer matters, since finding
/// it reads the message's stored bytes.
pub(super) fn auto_allow(
    mode: LoadRemoteImages,
    trusted: &[String],
    email: &str,
    display: Option<&str>,
    auth_passed: impl FnOnce() -> bool,
    in_junk: bool,
) -> bool {
    if in_junk {
        return false;
    }
    match mode {
        LoadRemoteImages::Ask => false,
        LoadRemoteImages::Always => true,
        LoadRemoteImages::Trusted => {
            let email = email.trim();
            trusted.iter().any(|t| t.eq_ignore_ascii_case(email))
                && !suspicious(display, email, auth_passed())
        }
    }
}

/// Whether the receiving server said DMARC passed for the domain of `message`'s From address, as
/// the reader's checks line and the brand logo read it (`ui/checks`, `ui/brand`). No body here,
/// no results this client believes, or an address with no domain: not passed.
///
/// Blocking: the first ask for a message reads its stored bytes; the answer is remembered by
/// `ui/checks`, so the reader's next render, and the head's checks line, find it known.
pub(super) fn dmarc_passed(store: &SqliteStore, message: &Message) -> bool {
    let Some(raw) = message.body.raw() else {
        return false;
    };
    let Some(domain) = mail_core::bimi::domain_of(&message.from.email) else {
        return false;
    };
    crate::ui::checks::lookup(store, message.id, raw)
        .is_some_and(|results| mail_mime::bimi::dmarc_passed_for(&results, domain))
}

/// [`auto_allow`] for one stored `message`, under the reader's settings.
pub(super) fn auto_allow_message(
    store: &SqliteStore,
    reading: &crate::settings::ReadingSettings,
    message: &Message,
) -> bool {
    auto_allow(
        reading.remote_images,
        &reading.trusted_image_senders,
        &message.from.email,
        message.from.name.as_deref(),
        || dmarc_passed(store, message),
        message.mailbox == MailboxRole::Spam,
    )
}

#[cfg(test)]
mod tests {
    use super::{auto_allow, suspicious};
    use crate::settings::LoadRemoteImages::{self, Always, Ask, Trusted};

    const SENDER: &str = "billing@shop.example";

    /// One row: the mode, whether the sender is on the list, whether the display name is a
    /// brand the address is not, whether DMARC passed, whether the message is in Junk, and
    /// whether its images load.
    struct Row {
        mode: LoadRemoteImages,
        listed: bool,
        spoofed: bool,
        dmarc: bool,
        junk: bool,
        loads: bool,
    }

    const fn row(
        mode: LoadRemoteImages,
        listed: bool,
        spoofed: bool,
        dmarc: bool,
        junk: bool,
        loads: bool,
    ) -> Row {
        Row {
            mode,
            listed,
            spoofed,
            dmarc,
            junk,
            loads,
        }
    }

    #[test]
    fn images_load_ahead_of_asking_only_where_the_mode_and_the_sender_allow() {
        let mut rows = Vec::new();
        for mode in [Ask, Trusted, Always] {
            for listed in [false, true] {
                for spoofed in [false, true] {
                    for dmarc in [false, true] {
                        for junk in [false, true] {
                            let loads = !junk
                                && match mode {
                                    Ask => false,
                                    Always => true,
                                    Trusted => listed && !spoofed && dmarc,
                                };
                            rows.push(row(mode, listed, spoofed, dmarc, junk, loads));
                        }
                    }
                }
            }
        }
        // The rule above, spelled out where it matters most, so a wrong rule cannot pass by
        // agreeing with itself.
        let spelled = [
            row(Ask, true, false, true, false, false),
            row(Trusted, true, false, true, false, true),
            row(Trusted, false, false, true, false, false),
            row(Trusted, true, true, true, false, false),
            row(Trusted, true, false, false, false, false),
            row(Trusted, true, false, true, true, false),
            row(Always, false, true, false, false, true),
            row(Always, true, false, true, true, false),
        ];
        rows.extend(spelled);
        let mut failures = Vec::new();
        for r in &rows {
            // "PayPal" is a brand the shop's address is not; "Shop" names none.
            let display = if r.spoofed { "PayPal" } else { "Shop" };
            let trusted = if r.listed {
                vec!["someone@else.example".to_owned(), SENDER.to_owned()]
            } else {
                vec!["someone@else.example".to_owned()]
            };
            let got = auto_allow(
                r.mode,
                &trusted,
                // The list is lowercase; the message's address need not be.
                "Billing@Shop.Example",
                Some(display),
                || r.dmarc,
                r.junk,
            );
            if got != r.loads {
                failures.push(format!(
                    "{:?} listed={} spoofed={} dmarc={} junk={}: got {got}, want {}",
                    r.mode, r.listed, r.spoofed, r.dmarc, r.junk, r.loads
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn the_checks_are_read_only_when_the_answer_turns_on_them() {
        // Reading them reads the message's bytes, so Ask, Always and an unlisted sender never do.
        let trusted = vec![SENDER.to_owned()];
        for (mode, email, junk) in [
            (Ask, SENDER, false),
            (Always, SENDER, false),
            (Trusted, "stranger@else.example", false),
            (Trusted, SENDER, true),
        ] {
            auto_allow(
                mode,
                &trusted,
                email,
                None,
                || panic!("{mode:?} {email} junk={junk} read the checks"),
                junk,
            );
        }
    }

    #[test]
    fn a_sender_is_suspicious_when_the_name_is_a_brand_or_dmarc_did_not_pass() {
        assert!(!suspicious(Some("Shop"), SENDER, true));
        assert!(!suspicious(None, SENDER, true));
        assert!(suspicious(Some("PayPal"), SENDER, true));
        assert!(suspicious(Some("Shop"), SENDER, false));
    }
}
