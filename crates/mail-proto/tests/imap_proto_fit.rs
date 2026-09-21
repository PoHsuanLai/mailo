//! Does `imap-proto` actually fit the sans-I/O design, and does it parse Gmail?
//!
//! Both are load-bearing assumptions behind replacing `imap-codec`, and neither is worth taking
//! on trust: `imap-codec` was rejected precisely because its FETCH parser fails the whole
//! untagged response on an attribute it does not know, and that is not visible from its README.

/// A response split mid-way must report `Incomplete`, not an error.
///
/// This is what lets `ImapSession` answer `Progress::Need(vec![IoNeed::Read])` and try again.
/// A parser that errored on a partial response would make every split read a protocol failure,
/// which is exactly the deadlock the SPLIT traces exist to catch.
#[test]
fn a_partial_response_is_incomplete_rather_than_an_error() {
    let partial = b"* 1 FETCH (UID 42 FLAGS (\\Seen) X-GM-MSGID 1876965017123";
    match imap_proto::parser::parse_response(partial) {
        Err(nom::Err::Incomplete(_)) => {}
        other => panic!("a split response must be Incomplete, got {other:?}"),
    }
}

/// The reason `imap-codec` was rejected: Gmail's FETCH attributes must parse.
///
/// `X-GM-MSGID` is `MessageKey::Gmail` and `X-GM-THRID` is `ServerThreads::ProviderId`. With a
/// parser that has no catch-all for unknown data items, an untagged FETCH carrying these is a
/// parse error in full — not a missing field — so both features become unimplementable.
#[test]
fn gmail_fetch_attributes_parse() {
    let whole = b"* 1 FETCH (UID 42 FLAGS (\\Seen) \
                  X-GM-MSGID 1876965017123110734 X-GM-THRID 1876965017123110734 \
                  X-GM-LABELS (\\Inbox \"travel\"))\r\n";
    let (rest, response) = imap_proto::parser::parse_response(whole)
        .unwrap_or_else(|e| panic!("Gmail FETCH must parse: {e:?}"));
    assert!(rest.is_empty(), "the whole response should be consumed");
    let rendered = format!("{response:?}");
    for needed in ["1876965017123110734", "travel"] {
        assert!(rendered.contains(needed), "lost {needed} in {rendered}");
    }
}

/// Real capability strings from our own phase-0 capture must parse.
#[test]
fn the_capability_line_our_spike_recorded_parses() {
    let capability = b"* CAPABILITY IMAP4rev1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST \
                       CHILDREN X-GM-EXT-1 XYZZY SASL-IR AUTH=XOAUTH2 AUTH=PLAIN \
                       AUTH=PLAIN-CLIENTTOKEN AUTH=OAUTHBEARER\r\n";
    imap_proto::parser::parse_response(capability)
        .unwrap_or_else(|e| panic!("the capability line we actually recorded must parse: {e:?}"));
}
