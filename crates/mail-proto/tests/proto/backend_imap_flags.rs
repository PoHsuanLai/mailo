//! Flag colours and labels as keywords over IMAP, as bytes on the wire.
//!
//! Apple Mail's colour is `\Flagged` and three keywords (`$MailFlagBit0` the lowest); a label on
//! a server without labels of its own is a keyword, written only where `PERMANENTFLAGS` lists
//! `\*`. The traces pin exactly what is stored and what a survey makes of what it reads.

use crate::common;

use common::{Driven, password_backend, replay};
use mail_domain::*;
use mail_proto::ProtoOutcome;

fn caps(labels: ServerLabels) -> AccountCaps {
    AccountCaps {
        labels,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        ..common::caps()
    }
}

fn at(uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 1,
        uid,
    }
}

const SIGN_IN: &str = concat!(
    "S: * OK ready\n",
    "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
    "S: a001 OK authenticated\n",
);

const SELECTED: &str = concat!(
    "C: a002 SELECT \"INBOX\"\n",
    "S: * OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted.\n",
    "S: * OK [UIDVALIDITY 1] UIDs valid.\n",
    "S: a002 OK [READ-WRITE] SELECT completed\n",
);

/// What `op` sends against a server keeping labels as `labels`, checked line by line by the
/// harness: `stores` are the client's `UID STORE` lines after the select.
fn stores(labels: ServerLabels, op: ProtoOp, stores: &[&str]) {
    let mut trace = format!("{SIGN_IN}{SELECTED}");
    for (n, line) in stores.iter().enumerate() {
        let tag = format!("a{:03}", n + 3);
        trace.push_str(&format!("C: {tag} UID STORE {line}\nS: {tag} OK done\n"));
    }
    trace.push_str("DONE\n");
    let mut driven = Driven::new(password_backend(caps(labels)), op);
    assert!(
        matches!(replay(&mut driven, &trace).unwrap(), ProtoOutcome::Applied),
        "{stores:?}"
    );
}

/// Each colour sets its bits and clears the others, beside `\Flagged`, so a colour set before it
/// by another client is gone.
#[test]
fn a_colour_is_stored_as_apple_mail_stores_it() {
    let cases: [(Hue, &[&str]); 3] = [
        (
            Hue::Orange,
            &[
                "7 +FLAGS (\\Flagged $MailFlagBit0)",
                "7 -FLAGS ($MailFlagBit1 $MailFlagBit2)",
            ],
        ),
        (
            Hue::Gray,
            &[
                "7 +FLAGS (\\Flagged $MailFlagBit1 $MailFlagBit2)",
                "7 -FLAGS ($MailFlagBit0)",
            ],
        ),
        (
            Hue::Red,
            &[
                "7 +FLAGS (\\Flagged)",
                "7 -FLAGS ($MailFlagBit0 $MailFlagBit1 $MailFlagBit2)",
            ],
        ),
    ];
    for (hue, lines) in cases {
        stores(
            ServerLabels::Keywords,
            ProtoOp::SetFlag {
                remotes: vec![at(7)],
                hue,
            },
            lines,
        );
    }
}

/// A server that keeps no keywords keeps the flag, and is sent nothing it might refuse: the
/// colour stays on this computer.
#[test]
fn a_server_without_keywords_is_sent_the_flag_alone() {
    stores(
        ServerLabels::LocalOnly,
        ProtoOp::SetFlag {
            remotes: vec![at(7)],
            hue: Hue::Blue,
        },
        &["7 +FLAGS (\\Flagged)"],
    );
    stores(
        ServerLabels::LocalOnly,
        ProtoOp::SetFlags {
            remotes: vec![at(7)],
            read: None,
            star: Some(Star::Unstarred),
        },
        &["7 -FLAGS (\\Flagged)"],
    );
}

/// Gmail turns every keyword into a label of its own, so a colour's bits would show up there as
/// labels: it is sent the flag alone, and what a survey reads from its keywords is no colour.
#[test]
fn gmail_is_sent_the_flag_alone_and_its_keywords_are_no_colour() {
    stores(
        ServerLabels::Supported,
        ProtoOp::SetFlag {
            remotes: vec![at(7)],
            hue: Hue::Orange,
        },
        &["7 +FLAGS (\\Flagged)"],
    );
    stores(
        ServerLabels::Supported,
        ProtoOp::SetFlags {
            remotes: vec![at(7)],
            read: None,
            star: Some(Star::Unstarred),
        },
        &["7 -FLAGS (\\Flagged)"],
    );
    assert_eq!(
        surveyed(ServerLabels::Supported, FETCHED).flag_colors,
        Vec::new()
    );
}

/// Unflagged, the colour goes too, as Apple Mail clears it.
#[test]
fn unflagging_clears_the_colour() {
    stores(
        ServerLabels::Keywords,
        ProtoOp::SetFlags {
            remotes: vec![at(7)],
            read: None,
            star: Some(Star::Unstarred),
        },
        &["7 -FLAGS (\\Flagged $MailFlagBit0 $MailFlagBit1 $MailFlagBit2)"],
    );
}

/// A label is its keyword: Thunderbird's tags by Thunderbird's keyword, any other name made an
/// atom, and a name with no keyword left out.
#[test]
fn labels_are_stored_as_keywords() {
    stores(
        ServerLabels::Keywords,
        ProtoOp::SetLabels {
            remotes: vec![at(7), at(9)],
            add: vec![
                "To Do".to_owned(),
                "Project X".to_owned(),
                "工作".to_owned(),
            ],
            remove: vec!["Important".to_owned()],
        },
        &["7,9 +FLAGS ($label4 Project_X)", "7,9 -FLAGS ($label1)"],
    );
}

/// Where labels are this computer's alone, a label change says nothing to the server.
#[test]
fn local_labels_are_not_sent() {
    let mut driven = Driven::new(
        password_backend(caps(ServerLabels::LocalOnly)),
        ProtoOp::SetLabels {
            remotes: vec![at(7)],
            add: vec!["Work".to_owned()],
            remove: Vec::new(),
        },
    );
    assert!(matches!(
        replay(&mut driven, "DONE\n").unwrap(),
        ProtoOutcome::Applied
    ));
}

fn surveyed(labels: ServerLabels, fetch_lines: &str) -> Ingest {
    // Gmail's labels are asked for with the survey; no other server is asked for them.
    let asked = if matches!(labels, ServerLabels::Supported) {
        " X-GM-LABELS"
    } else {
        ""
    };
    let trace = format!(
        "{SIGN_IN}\
         C: a002 EXAMINE \"INBOX\"\n\
         S: * OK [UIDVALIDITY 1] UIDs valid.\n\
         S: a002 OK [READ-ONLY] EXAMINE completed\n\
         C: a003 UID FETCH 1:* (UID FLAGS RFC822.SIZE{asked})\n\
         {fetch_lines}\
         S: a003 OK Success\n\
         DONE\n"
    );
    let mut driven = Driven::new(
        password_backend(caps(labels)),
        ProtoOp::FetchEnvelopes {
            mailbox: MailboxRef {
                account: mail_domain::testing::account(),
                path: "INBOX".to_owned(),
            },
            since: FetchSince::Beginning,
        },
    );
    match replay(&mut driven, &trace).unwrap() {
        ProtoOutcome::Ingested(ingest) => *ingest,
        other => panic!("{other:?}"),
    }
}

const FETCHED: &str = concat!(
    "S: * 1 FETCH (UID 7 FLAGS (\\Seen \\Flagged $MailFlagBit0 $MailFlagBit1) RFC822.SIZE 100)\n",
    "S: * 2 FETCH (UID 8 FLAGS ($label4 Receipts $MDNSent NonJunk) RFC822.SIZE 100)\n",
    "S: * 3 FETCH (UID 9 FLAGS () RFC822.SIZE 100)\n",
);

/// A survey reads each message's colour from its keywords, and, where labels are keywords, its
/// labels: every message's list, an empty one included, since the list is complete.
#[test]
fn a_survey_reads_colours_and_keyword_labels() {
    let ingest = surveyed(ServerLabels::Keywords, FETCHED);
    assert_eq!(
        ingest.flag_colors,
        vec![(at(7), Hue::Green), (at(8), Hue::Red), (at(9), Hue::Red)]
    );
    assert_eq!(
        ingest.label_names,
        vec![
            (at(7), Vec::new()),
            (at(8), vec!["To Do".to_owned(), "Receipts".to_owned()]),
            (at(9), Vec::new()),
        ]
    );
}

/// A server that keeps no keywords says nothing about colours or labels, so what is held here
/// stands: an empty answer would read as every colour set back to red.
#[test]
fn a_server_without_keywords_reports_no_colours_or_labels() {
    let ingest = surveyed(ServerLabels::LocalOnly, FETCHED);
    assert_eq!(ingest.flag_colors, Vec::new());
    assert_eq!(ingest.label_names, Vec::new());
}

/// Which labels an IMAP server has: Gmail's own, keywords where `PERMANENTFLAGS` lists `\*`, and
/// none otherwise. The inbox is selected read-write for the answer: read-only, a server lists
/// no permanent flags at all.
#[test]
fn permanent_flags_decide_whether_labels_are_keywords() {
    let cases = [
        (
            "IMAP4rev1",
            "\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*",
            ServerLabels::Keywords,
        ),
        (
            "IMAP4rev1",
            "\\Answered \\Flagged \\Deleted \\Seen \\Draft $MDNSent",
            ServerLabels::LocalOnly,
        ),
        (
            "IMAP4rev1 X-GM-EXT-1",
            "\\Answered \\Flagged \\Draft \\Deleted \\Seen $NotPhishing $Phishing \\*",
            ServerLabels::Supported,
        ),
    ];
    for (advertised, permanent, want) in cases {
        let trace = format!(
            "{SIGN_IN}\
             C: a002 CAPABILITY\n\
             S: * CAPABILITY {advertised}\n\
             S: a002 OK done\n\
             C: a003 CAPABILITY\n\
             S: * CAPABILITY {advertised}\n\
             S: a003 OK done\n\
             C: a004 SELECT \"INBOX\"\n\
             S: * OK [PERMANENTFLAGS ({permanent})] Flags permitted.\n\
             S: a004 OK [READ-WRITE] SELECT completed\n\
             DONE\n"
        );
        let mut driven = Driven::new(
            password_backend(caps(ServerLabels::LocalOnly)),
            ProtoOp::FetchCaps,
        );
        let ProtoOutcome::Caps(found) = replay(&mut driven, &trace).unwrap() else {
            panic!("expected caps: {permanent}");
        };
        assert_eq!(found.labels, want, "{advertised} / {permanent}");
    }
}
