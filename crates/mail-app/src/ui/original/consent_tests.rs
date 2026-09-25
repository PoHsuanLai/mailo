//! The consent the Original frame's network is held to, without a renderer: a frame is named by
//! the message its `data-frame-tag` names, as the network reads it.

use super::*;

const A: &str = "https://cdn.test/a.png";
const B: &str = "https://other.test/b.png";

fn ids() -> (ThreadId, MessageId, MessageId) {
    (
        ThreadId::generate(),
        MessageId::generate(),
        MessageId::generate(),
    )
}

/// A thread of two messages, one image each, consented to by one reader.
fn granted() -> (Consent, Holder, ThreadId, MessageId, MessageId) {
    let consent = Consent::new();
    let holder = consent.holder();
    let (thread, first, second) = ids();
    consent.hold(
        holder,
        thread,
        Some(vec![
            (first, vec![A.to_owned()]),
            (second, vec![B.to_owned()]),
        ]),
    );
    (consent, holder, thread, first, second)
}

#[test]
fn nothing_is_admitted_before_consent() {
    let consent = Consent::new();
    assert!(!consent.granted());
    assert_eq!(consent.admits(MessageId::generate(), A), None);
}

#[test]
fn a_consented_image_is_admitted_and_nothing_else() {
    let (consent, _, _, first, _) = granted();
    assert!(consent.admits(first, A).is_some());
    assert_eq!(
        consent.admits(first, "https://cdn.test/not-listed.png"),
        None
    );
    assert_eq!(
        consent.admits(first, "https://tracker.test/pixel.gif"),
        None
    );
}

#[test]
fn only_web_addresses_are_ever_admitted() {
    // A hostile list (a sanitizer miss, fed around it here) cannot open another scheme: the
    // list is read through the same web-only filter as the request.
    let consent = Consent::new();
    let holder = consent.holder();
    let (thread, message, _) = ids();
    let hostile = [
        "file:///etc/passwd",
        "cid:logo@here",
        "data:image/png;base64,iVBORw0KGgo=",
        "javascript:alert(1)",
        "ftp://files.test/a.png",
    ];
    consent.hold(
        holder,
        thread,
        Some(vec![(
            message,
            hostile.iter().map(|url| (*url).to_owned()).collect(),
        )]),
    );
    for url in hostile {
        assert_eq!(consent.admits(message, url), None, "{url} was admitted");
    }
}

#[test]
fn the_url_is_compared_as_the_renderer_asks_for_it() {
    let consent = Consent::new();
    let holder = consent.holder();
    let (thread, message, _) = ids();
    consent.hold(
        holder,
        thread,
        Some(vec![(message, vec!["HTTPS://CDN.test/a.png".to_owned()])]),
    );
    assert!(consent.admits(message, A).is_some());
}

#[test]
fn a_frame_fetches_for_its_own_message_only() {
    let (consent, _, _, first, second) = granted();
    assert!(consent.admits(first, A).is_some());
    assert_eq!(
        consent.admits(first, B),
        None,
        "one message's frame fetched another message's image"
    );
    assert!(consent.admits(second, B).is_some());
    assert_eq!(consent.admits(second, A), None);
    // A message the grant does not name fetches nothing, whatever it asks for.
    assert_eq!(consent.admits(MessageId::generate(), A), None);
}

#[test]
fn an_image_two_messages_share_is_each_ones_own() {
    let consent = Consent::new();
    let holder = consent.holder();
    let (thread, first, second) = ids();
    let logo = "https://brand.test/logo.png";
    consent.hold(
        holder,
        thread,
        Some(vec![
            (first, vec![logo.to_owned(), A.to_owned()]),
            (second, vec![logo.to_owned(), B.to_owned()]),
        ]),
    );
    assert!(consent.admits(first, logo).is_some());
    assert!(consent.admits(second, logo).is_some());
    assert!(consent.admits(first, A).is_some());
    assert_eq!(consent.admits(first, B), None, "the shared logo opened B");
}

#[test]
fn revoking_refuses_new_requests_and_stales_admitted_ones() {
    let (consent, holder, thread, first, _) = granted();
    let ticket = consent.admits(first, A).expect("admitted");
    assert!(consent.stands(ticket));
    consent.hold(holder, thread, None);
    assert!(!consent.granted());
    assert!(
        !consent.stands(ticket),
        "a fetch landing after revoking would be shown"
    );
    assert_eq!(consent.admits(first, A), None);
}

#[test]
fn the_same_grant_again_keeps_what_it_admitted() {
    let (consent, holder, thread, first, second) = granted();
    let ticket = consent.admits(first, A).expect("admitted");
    // The reader renders again with nothing changed: every keystroke does this.
    consent.hold(
        holder,
        thread,
        Some(vec![
            (first, vec![A.to_owned()]),
            (second, vec![B.to_owned()]),
        ]),
    );
    assert!(consent.stands(ticket));
    assert_eq!(
        consent.admits(first, B),
        None,
        "the frame forgot what it is"
    );
}

#[test]
fn another_thread_is_a_new_grant() {
    let (consent, holder, _, first, _) = granted();
    let ticket = consent.admits(first, A).expect("admitted");
    let (other, message, _) = ids();
    consent.hold(holder, other, Some(vec![(message, vec![B.to_owned()])]));
    assert!(!consent.stands(ticket));
    assert_eq!(consent.admits(first, A), None, "the last thread's image");
    assert!(consent.admits(message, B).is_some());
}

#[test]
fn a_closing_reader_takes_back_only_its_own_grant() {
    let (consent, holder, _, first, _) = granted();
    let other = consent.holder();
    consent.release(other);
    assert!(consent.granted(), "a reader that granted nothing revoked");
    consent.release(holder);
    assert!(!consent.granted());
    assert_eq!(consent.admits(first, A), None);
}
