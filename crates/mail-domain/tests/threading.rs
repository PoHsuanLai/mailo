//! JWZ threading over the public surface of `mail_domain::threading`.
//!
//! Every case is a small mailbox and the grouping it must produce. The grouping is compared,
//! never the ids themselves: a fresh [`ThreadId`] is random by construction, and a test that
//! pinned one would be testing `uuid`.

use mail_domain::ThreadId;
use mail_domain::threading::{ThreadInput, normalize_id, thread};
use std::collections::HashMap;
use uuid::Uuid;

/// One message's threading headers, in a form that is cheap to write in a table.
#[derive(Debug, Clone, Copy)]
struct Msg {
    id: Option<&'static str>,
    irt: Option<&'static str>,
    refs: &'static [&'static str],
    subject: &'static str,
}

impl Msg {
    const fn new(id: &'static str) -> Msg {
        Msg {
            id: Some(id),
            irt: None,
            refs: &[],
            subject: "",
        }
    }

    const fn anon() -> Msg {
        Msg {
            id: None,
            irt: None,
            refs: &[],
            subject: "",
        }
    }

    const fn refs(self, refs: &'static [&'static str]) -> Msg {
        Msg { refs, ..self }
    }

    const fn irt(self, irt: &'static str) -> Msg {
        Msg {
            irt: Some(irt),
            ..self
        }
    }

    const fn subject(self, subject: &'static str) -> Msg {
        Msg { subject, ..self }
    }
}

fn run(msgs: &[Msg], existing: &dyn Fn(&str) -> Option<ThreadId>) -> Vec<ThreadId> {
    let refs: Vec<Vec<String>> = msgs
        .iter()
        .map(|m| m.refs.iter().map(|r| (*r).to_string()).collect())
        .collect();
    let inputs: Vec<ThreadInput<'_>> = msgs
        .iter()
        .zip(&refs)
        .map(|(m, r)| ThreadInput {
            message_id: m.id,
            in_reply_to: m.irt,
            references: r,
            subject: m.subject,
        })
        .collect();
    thread(&inputs, existing)
}

fn none(_: &str) -> Option<ThreadId> {
    None
}

/// Which inputs ended up together, as a canonical partition of `0..n`.
fn grouping(ids: &[ThreadId]) -> Vec<Vec<usize>> {
    let mut seen: Vec<ThreadId> = Vec::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        match seen.iter().position(|s| s == id) {
            Some(at) => out[at].push(i),
            None => {
                seen.push(*id);
                out.push(vec![i]);
            }
        }
    }
    out.sort();
    out
}

fn groups_of(msgs: &[Msg]) -> Vec<Vec<usize>> {
    grouping(&run(msgs, &none))
}

fn expect(groups: &[&[usize]]) -> Vec<Vec<usize>> {
    let mut want: Vec<Vec<usize>> = groups.iter().map(|g| g.to_vec()).collect();
    want.sort();
    want
}

// ---------------------------------------------------------------------------------------
// normalize_id
// ---------------------------------------------------------------------------------------

#[test]
fn ids_normalize() {
    // (raw, normalized). The empty string is the "no usable id" sentinel.
    const CASES: &[(&str, &str)] = &[
        ("<a@b.example>", "a@b.example"),
        ("  <A@B.Example>  ", "a@b.example"),
        ("a@b.example", "a@b.example"),
        ("< a@b.example >", "a@b.example"),
        ("<CAFÉ@b.example>", "cafÉ@b.example"), // ASCII-only lowercasing, by design
        ("<<a@b.example>>", ""),                // one layer stripped, inner brackets are junk
        ("", ""),
        ("   ", ""),
        ("<>", ""),
        ("<   >", ""),
        ("<a b@example>", ""),  // whitespace inside is not an addr-spec
        ("<a\tb@example>", ""), // nor a tab
        ("<a\u{0}b@ex>", ""),   // nor a control character
        ("<a@b>extra", ""),     // trailing junk after the bracket pair
        ("mangled<a@b>", ""),   // and leading junk
    ];
    for &(raw, want) in CASES {
        assert_eq!(normalize_id(raw), want, "normalize_id({raw:?})");
    }
}

#[test]
fn unusable_ids_never_join_two_messages() {
    // Two messages equally broken are not thereby related.
    const BROKEN: &[Msg] = &[
        Msg::new("<>").subject("Invoice 4711"),
        Msg::new("<>").subject("Dinner"),
        Msg::new("<a b@example>").subject("Standup"),
        Msg::anon().subject("Standup"),
    ];
    assert_eq!(groups_of(BROKEN), expect(&[&[0], &[1], &[2], &[3]]));
}

// ---------------------------------------------------------------------------------------
// The algorithm
// ---------------------------------------------------------------------------------------

const CHAIN_REFS: &[Msg] = &[
    Msg::new("<a@x>").subject("Design review"),
    Msg::new("<b@x>")
        .refs(&["<a@x>"])
        .subject("Re: Design review"),
    Msg::new("<c@x>")
        .refs(&["<a@x>", "<b@x>"])
        .subject("Re: Design review"),
];

const CHAIN_IRT: &[Msg] = &[
    Msg::new("<a@x>").subject("Design review"),
    Msg::new("<b@x>").irt("<a@x>").subject("Re: Design review"),
    Msg::new("<c@x>").irt("<b@x>").subject("Re: Design review"),
];

const CHAIN_MIXED: &[Msg] = &[
    Msg::new("<a@x>"),
    Msg::new("<b@x>").refs(&["<a@x>"]).irt("<a@x>"),
    Msg::new("<c@x>").refs(&["<a@x>"]).irt("<b@x>"),
];

const REPLIES_ARRIVE_FIRST: &[Msg] = &[
    Msg::new("<c@x>").refs(&["<a@x>", "<b@x>"]),
    Msg::new("<b@x>").refs(&["<a@x>"]),
    Msg::new("<a@x>"),
];

const NO_MESSAGE_ID: &[Msg] = &[
    Msg::new("<a@x>").subject("Deploy window"),
    // No `Message-ID` of its own, but it still says what it is replying to.
    Msg::anon().refs(&["<a@x>"]).subject("Re: Deploy window"),
    // No id, no references, nothing to go on: its own thread.
    Msg::anon().subject("Unrelated"),
];

const MISSING_PARENT: &[Msg] = &[
    Msg::new("<b@x>").refs(&["<ghost@x>"]),
    Msg::new("<c@x>").refs(&["<ghost@x>"]),
    Msg::new("<d@x>").refs(&["<other-ghost@x>"]),
];

const SELF_REFERENCE: &[Msg] = &[Msg::new("<a@x>").refs(&["<a@x>"]), Msg::new("<b@x>")];

const TWO_CYCLE: &[Msg] = &[
    Msg::new("<a@x>").refs(&["<b@x>"]),
    Msg::new("<b@x>").refs(&["<a@x>"]),
];

const THREE_CYCLE: &[Msg] = &[
    Msg::new("<a@x>").refs(&["<b@x>"]),
    Msg::new("<b@x>").refs(&["<c@x>"]),
    Msg::new("<c@x>").refs(&["<a@x>"]),
];

const CYCLE_INSIDE_ONE_HEADER: &[Msg] = &[
    Msg::new("<m@x>").refs(&["<a@x>", "<b@x>", "<a@x>", "<b@x>"]),
    Msg::new("<n@x>").refs(&["<b@x>", "<a@x>"]),
];

const SAME_SUBJECT_NO_REFS: &[Msg] = &[
    Msg::new("<a@x>").subject("hello"),
    Msg::new("<b@x>").subject("hello"),
    Msg::new("<c@x>").subject("Hello"),
];

const SUBJECT_REPAIR: &[Msg] = &[
    Msg::new("<a@x>").subject("Q3 budget"),
    Msg::new("<b@x>").refs(&["<a@x>"]).subject("Re: Q3 budget"),
    // A client that emits no references at all. The repair this fallback exists for.
    Msg::new("<c@x>").subject("Re: Q3 budget"),
    Msg::new("<d@x>").subject("Fwd: Q3 budget"),
];

const SUBJECT_AMBIGUOUS: &[Msg] = &[
    Msg::new("<a@x>").subject("Q3 budget"),
    Msg::new("<b@x>").subject("Q3 budget"),
    // Two live conversations of this name: which one the reply belongs to is unknowable.
    Msg::new("<c@x>").subject("Re: Q3 budget"),
];

const SUBJECT_EMPTY_BASE: &[Msg] = &[
    Msg::new("<a@x>").subject(""),
    Msg::new("<b@x>").subject("Re:"),
    Msg::new("<c@x>").subject("   "),
];

const SUBJECT_NEVER_BEATS_REFERENCES: &[Msg] = &[
    Msg::new("<a@x>").subject("Q3 budget"),
    // Replying to something we have not seen. It is a reply, so it is not an anchor, and it
    // names a reference, so it is not an orphan either: no subject merging applies.
    Msg::new("<b@x>")
        .refs(&["<ghost@x>"])
        .subject("Re: Q3 budget"),
];

const DUPLICATE_IDS: &[Msg] = &[
    // A mailer that stamps one `Message-ID` on everything must not collapse the mailbox.
    Msg::new("<same@x>").subject("Invoice 1"),
    Msg::new("<same@x>").subject("Invoice 2"),
    // ... but a duplicate that carries the references still lands in the right thread.
    Msg::new("<same@x>")
        .refs(&["<other@x>"])
        .subject("Invoice 3"),
    Msg::new("<other@x>").subject("Invoice 3"),
];

const CASE_NORMALIZED_LINKS: &[Msg] = &[
    Msg::new("<A@X>").subject("Release notes"),
    Msg::new("<b@x>")
        .refs(&["  <a@x>  "])
        .subject("Re: Release notes"),
    Msg::new("<c@x>").irt("<A@X>").subject("Re: Release notes"),
];

const UNUSABLE_REFS_ARE_DROPPED: &[Msg] = &[
    Msg::new("<a@x>").subject("Topic"),
    Msg::new("<b@x>")
        .refs(&["<>", "   ", "<a@x>"])
        .subject("Re: Topic"),
    // Nothing usable left after normalization, and its subject anchors nothing.
    Msg::new("<c@x>").refs(&["<>"]).subject("Unrelated"),
];

/// A named mailbox and the partition of input positions it must produce.
type Case = (&'static str, &'static [Msg], &'static [&'static [usize]]);

#[test]
fn threading_groups_messages() {
    const CASES: &[Case] = &[
        (
            "three-message chain via References",
            CHAIN_REFS,
            &[&[0, 1, 2]],
        ),
        ("chain via In-Reply-To only", CHAIN_IRT, &[&[0, 1, 2]]),
        (
            "References and In-Reply-To agree",
            CHAIN_MIXED,
            &[&[0, 1, 2]],
        ),
        (
            "replies before their parent",
            REPLIES_ARRIVE_FIRST,
            &[&[0, 1, 2]],
        ),
        ("no Message-ID at all", NO_MESSAGE_ID, &[&[0, 1], &[2]]),
        (
            "parent we have never seen",
            MISSING_PARENT,
            &[&[0, 1], &[2]],
        ),
        (
            "a message referencing itself",
            SELF_REFERENCE,
            &[&[0], &[1]],
        ),
        ("reference cycle a -> b -> a", TWO_CYCLE, &[&[0, 1]]),
        (
            "reference cycle a -> b -> c -> a",
            THREE_CYCLE,
            &[&[0, 1, 2]],
        ),
        (
            "a loop inside one header",
            CYCLE_INSIDE_ONE_HEADER,
            &[&[0, 1]],
        ),
        (
            "same subject, no references",
            SAME_SUBJECT_NO_REFS,
            &[&[0], &[1], &[2]],
        ),
        (
            "subject repairs a reference-less reply",
            SUBJECT_REPAIR,
            &[&[0, 1, 2, 3]],
        ),
        (
            "two anchors make the subject useless",
            SUBJECT_AMBIGUOUS,
            &[&[0], &[1], &[2]],
        ),
        (
            "an empty subject joins nothing",
            SUBJECT_EMPTY_BASE,
            &[&[0], &[1], &[2]],
        ),
        (
            "a reply with references is not an orphan",
            SUBJECT_NEVER_BEATS_REFERENCES,
            &[&[0], &[1]],
        ),
        (
            "a repeated Message-ID",
            DUPLICATE_IDS,
            &[&[0], &[1], &[2, 3]],
        ),
        (
            "ids compare normalized",
            CASE_NORMALIZED_LINKS,
            &[&[0, 1, 2]],
        ),
        (
            "unusable references are dropped",
            UNUSABLE_REFS_ARE_DROPPED,
            &[&[0, 1], &[2]],
        ),
    ];

    for &(name, msgs, want) in CASES {
        assert_eq!(groups_of(msgs), expect(want), "case: {name}");
    }
}

#[test]
fn threading_is_deterministic() {
    const CASES: &[(&str, &[Msg])] = &[
        ("three-message chain via References", CHAIN_REFS),
        ("replies before their parent", REPLIES_ARRIVE_FIRST),
        ("parent we have never seen", MISSING_PARENT),
        ("reference cycle a -> b -> c -> a", THREE_CYCLE),
        ("subject repairs a reference-less reply", SUBJECT_REPAIR),
        ("two anchors make the subject useless", SUBJECT_AMBIGUOUS),
        ("a repeated Message-ID", DUPLICATE_IDS),
    ];
    for &(name, msgs) in CASES {
        let first = groups_of(msgs);
        for round in 1..4 {
            assert_eq!(groups_of(msgs), first, "case: {name}, round {round}");
        }
    }
}

#[test]
fn a_cycle_of_a_thousand_messages_terminates() {
    // Every message references the next, and the last references the first. A JWZ
    // implementation that trusts its headers never returns from this.
    let ids: Vec<String> = (0..1000).map(|n| format!("<m{n}@x>")).collect();
    let refs: Vec<Vec<String>> = (0..1000)
        .map(|n| vec![ids[(n + 1) % 1000].clone()])
        .collect();
    let inputs: Vec<ThreadInput<'_>> = (0..1000)
        .map(|n| ThreadInput {
            message_id: Some(ids[n].as_str()),
            in_reply_to: None,
            references: &refs[n],
            subject: "loop",
        })
        .collect();

    let out = thread(&inputs, &none);
    assert_eq!(out.len(), 1000);
    assert_eq!(
        grouping(&out).len(),
        1,
        "one big broken conversation, but one"
    );
}

// ---------------------------------------------------------------------------------------
// `existing`: incremental sync
// ---------------------------------------------------------------------------------------

fn tid(n: u128) -> ThreadId {
    ThreadId::from_uuid(Uuid::from_u128(n))
}

fn known(pairs: &[(&'static str, ThreadId)]) -> impl Fn(&str) -> Option<ThreadId> + use<> {
    let map: HashMap<String, ThreadId> = pairs
        .iter()
        .map(|&(id, tid)| (id.to_string(), tid))
        .collect();
    move |id: &str| map.get(id).copied()
}

#[test]
fn new_messages_join_the_thread_their_parent_already_has() {
    let old = tid(1);
    // The parent is not in this batch at all: it is already in the store, under `old`.
    const BATCH: &[Msg] = &[
        Msg::new("<b@x>").refs(&["<a@x>"]).subject("Re: Ship it"),
        Msg::new("<c@x>")
            .refs(&["<a@x>", "<b@x>"])
            .subject("Re: Ship it"),
        Msg::new("<z@x>").subject("Something else"),
    ];
    let out = run(BATCH, &known(&[("a@x", old)]));
    assert_eq!(out[0], old);
    assert_eq!(out[1], old);
    assert_ne!(out[2], old, "an unrelated message must not be captured");
}

#[test]
fn a_known_message_keeps_its_thread_when_it_is_rethreaded() {
    let old = tid(7);
    let out = run(CHAIN_REFS, &known(&[("b@x", old)]));
    assert_eq!(out, vec![old; 3]);
}

#[test]
fn an_existing_id_is_honoured_through_the_subject_fallback() {
    let old = tid(3);
    let out = run(SUBJECT_REPAIR, &known(&[("a@x", old)]));
    assert_eq!(out, vec![old; 4]);
}

#[test]
fn two_known_threads_pulled_into_one_collapse_onto_the_lower_id() {
    let low = tid(2);
    let high = tid(9);
    // `m` cites both conversations, so they were always one and we only now can tell.
    const BATCH: &[Msg] = &[Msg::new("<m@x>").refs(&["<a@x>", "<b@x>"])];

    let out = run(BATCH, &known(&[("a@x", low), ("b@x", high)]));
    assert_eq!(out, vec![low]);

    // The survivor is a property of the set, not of the order the ids were met in.
    const SWAPPED: &[Msg] = &[Msg::new("<m@x>").refs(&["<b@x>", "<a@x>"])];
    assert_eq!(
        run(SWAPPED, &known(&[("a@x", low), ("b@x", high)])),
        vec![low]
    );
    assert_eq!(
        run(BATCH, &known(&[("a@x", high), ("b@x", low)])),
        vec![low]
    );
}

#[test]
fn a_merge_is_stable_across_runs_and_across_batch_order() {
    let low = tid(4);
    let high = tid(40);
    let store = known(&[("a@x", low), ("b@x", high)]);

    // Three messages that, between them, tie the two known threads together.
    const FORWARD: &[Msg] = &[
        Msg::new("<p@x>").refs(&["<a@x>"]),
        Msg::new("<q@x>").refs(&["<b@x>"]),
        Msg::new("<r@x>").refs(&["<p@x>", "<q@x>"]),
    ];
    const BACKWARD: &[Msg] = &[
        Msg::new("<r@x>").refs(&["<p@x>", "<q@x>"]),
        Msg::new("<q@x>").refs(&["<b@x>"]),
        Msg::new("<p@x>").refs(&["<a@x>"]),
    ];

    assert_eq!(run(FORWARD, &store), vec![low; 3]);
    assert_eq!(run(FORWARD, &store), vec![low; 3], "same batch, twice");
    assert_eq!(
        run(BACKWARD, &store),
        vec![low; 3],
        "same mailbox, other order"
    );
}

#[test]
fn threads_with_nothing_known_get_fresh_ids() {
    let out = run(MISSING_PARENT, &none);
    assert_eq!(grouping(&out), expect(&[&[0, 1], &[2]]));
    assert_ne!(out[0], out[2], "two threads, two ids");
}
