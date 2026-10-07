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
use dioxus::prelude::*;
use mail_domain::{BlobId, MailboxRole, Message, MessageId};
use mail_store::SqliteStore;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

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
/// somebody already doubted. `auth_passed` is asked only when the answer matters: under Trusted,
/// for a listed sender. Only then does the reader have to wait for a message's checks.
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

/// Whether the receiving server said DMARC passed for `domain`, by what `results` hold.
fn passed(results: Option<&mail_mime::AuthResults>, domain: &str) -> bool {
    results.is_some_and(|results| mail_mime::bimi::dmarc_passed_for(results, domain))
}

/// One message whose checks the reader is waiting on: which message, and the body they are in.
type Wanted = (MessageId, BlobId);

/// What one reader has looked up off the thread that draws, as `cache::Sent` is for renderings.
#[derive(Default)]
pub(super) struct Checked {
    /// Being read on a blocking thread: asked for again by nobody until it lands.
    going: Vec<Wanted>,
    /// Read, and whether DMARC passed for the sender's domain. Kept here as well as in
    /// `ui/checks`' cache, which holds only the last few: an answer that cache dropped before
    /// the reader drew again would otherwise be sent for again, for ever.
    landed: Vec<(Wanted, bool)>,
}

/// Whether the receiving server said DMARC passed for the domain of `message`'s From address,
/// as the reader's checks line and the brand logo read it (`ui/checks`, `ui/brand`), if that is
/// known yet. No body here, or an address with no domain: known, and not passed.
///
/// Never reads the store: only what this reader already looked up, and `ui/checks`' cache. When
/// neither knows, `None`, and the message is put in `wanted` for [`look_later`]: opening a
/// conversation must not wait on reading and parsing its raw bytes.
pub(super) fn dmarc_known(
    checked: &Checked,
    message: &Message,
    wanted: &mut Vec<Wanted>,
) -> Option<bool> {
    let Some(raw) = message.body.raw() else {
        return Some(false);
    };
    let Some(domain) = mail_core::bimi::domain_of(&message.from.email) else {
        return Some(false);
    };
    let key = (message.id, raw);
    if let Some((_, passed)) = checked.landed.iter().find(|(had, _)| *had == key) {
        return Some(*passed);
    }
    if let Some(results) = crate::ui::checks::cached(message.id, raw) {
        return Some(passed(results.as_ref(), domain));
    }
    if !wanted.contains(&key) {
        wanted.push(key);
    }
    None
}

/// [`auto_allow`] for one stored `message`, under the reader's settings. A sender whose checks
/// are not known yet is not trusted yet: the message is asked about, its checks are put in
/// `wanted`, and the reader decides again once they land.
pub(super) fn auto_allow_message(
    reading: &crate::settings::ReadingSettings,
    checked: &Checked,
    message: &Message,
    wanted: &mut Vec<Wanted>,
) -> bool {
    auto_allow(
        reading.remote_images,
        &reading.trusted_image_senders,
        &message.from.email,
        message.from.name.as_deref(),
        || dmarc_known(checked, message, wanted).unwrap_or(false),
        message.mailbox == MailboxRole::Spam,
    )
}

/// Read `wanted`'s checks on a blocking thread, as the checks line does (`ui/checks`), then move
/// `landed` so the reader decides again with them. What is already being read is not sent twice.
pub(super) fn look_later(
    store: Arc<SqliteStore>,
    messages: Vec<Message>,
    wanted: Vec<Wanted>,
    checked: Rc<RefCell<Checked>>,
    mut landed: Signal<u64>,
) {
    let fresh: Vec<Message> = {
        let mut checked = checked.borrow_mut();
        let fresh: Vec<Message> = messages
            .into_iter()
            .filter(|message| {
                message.body.raw().is_some_and(|raw| {
                    let key = (message.id, raw);
                    wanted.contains(&key) && !checked.going.contains(&key)
                })
            })
            .collect();
        checked.going.extend(
            fresh
                .iter()
                .filter_map(|message| Some((message.id, message.body.raw()?))),
        );
        fresh
    };
    if fresh.is_empty() {
        return;
    }
    spawn(async move {
        let found = tokio::task::spawn_blocking(move || {
            fresh
                .iter()
                .filter_map(|message| {
                    let raw = message.body.raw()?;
                    let domain = mail_core::bimi::domain_of(&message.from.email)?;
                    let results = crate::ui::checks::lookup(&store, message.id, raw);
                    Some(((message.id, raw), passed(results.as_ref(), domain)))
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        {
            let mut checked = checked.borrow_mut();
            checked
                .going
                .retain(|key| !found.iter().any(|(had, _)| had == key));
            checked.landed.extend(found);
        }
        landed += 1;
    });
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
