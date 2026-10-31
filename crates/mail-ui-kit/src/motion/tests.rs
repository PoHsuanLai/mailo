//! The parts of motion that need no window: where a drag may drop, and how an Undo pressed
//! before its action is done waits for it.

use super::drag::{drop_op, view_kind};
use mail_domain::*;

#[test]
fn what_each_place_accepts() {
    let places = crate::view::default_places();
    let label = LabelId::generate();
    let cases: Vec<(&str, Option<Op>)> = vec![
        ("Inbox", Some(Op::Restore)),
        ("Archive", Some(Op::Archive)),
        ("Trash", Some(Op::Trash)),
        ("Spam", Some(Op::Spam)),
        // Saved searches and places nothing is filed into refuse.
        ("Starred", None),
        ("Pinned", None),
        ("History", None),
        ("Snoozed", None),
        ("Waiting", None),
        ("Sent", None),
        ("Drafts", None),
    ];
    for (name, want) in cases {
        let place = places
            .iter()
            .find(|place| place.name == name)
            .unwrap_or_else(|| panic!("no place {name}"));
        assert_eq!(drop_op(&view_kind(place)), want, "{name}");
    }
    let labelled = crate::view::Place {
        name: "spec".to_owned(),
        source: crate::view::Source::Mail(Filter::HasLabel(label)),
        unread: None,
    };
    assert_eq!(
        drop_op(&view_kind(&labelled)),
        Some(Op::Label(label, Membership::In))
    );
}

/// An Undo pressed before the write it means is done waits for it, and takes that gesture back,
/// not the one before; with nothing underway it is the stack's.
#[test]
fn an_undo_pressed_before_its_action_is_done_waits_for_it() {
    let mut underway = super::Underway::default();
    assert!(
        !underway.undo_pressed(),
        "nothing underway: the stack's undo"
    );
    let star = underway.asked();
    assert!(underway.undo_pressed(), "the action is not done: wait");
    assert!(underway.done(star), "done: the waiting undo is made now");
    assert!(!underway.done(star), "and only once");
    assert!(!underway.undo_pressed(), "nothing underway again");
}

/// An Undo is owed to the newest gesture asked, and made when that one is done, whichever
/// gesture the client finishes first: with a star and then a mute underway, a Ctrl-Z owes the
/// mute, so the star finishing last does not spend it on the star.
#[test]
fn an_undo_is_owed_to_the_newest_gesture_whichever_is_done_first() {
    let mut underway = super::Underway::default();
    let star = underway.asked();
    let mute = underway.asked();
    assert!(underway.undo_pressed());
    assert!(underway.done(mute), "the mute was owed the undo");
    assert!(!underway.done(star), "the star was not");

    let star = underway.asked();
    let mute = underway.asked();
    assert!(underway.undo_pressed());
    assert!(
        !underway.done(star),
        "the star finishing first owes nothing"
    );
    assert!(
        underway.done(mute),
        "the mute finishing last makes the undo"
    );
}

/// Two Undos pressed while two gestures are underway take one each, newest first.
#[test]
fn each_undo_waits_for_its_own_gesture() {
    let mut underway = super::Underway::default();
    let first = underway.asked();
    let second = underway.asked();
    assert!(underway.undo_pressed());
    assert!(underway.undo_pressed());
    assert!(
        !underway.undo_pressed(),
        "both are owed one: the third is the stack's"
    );
    assert!(underway.done(second));
    assert!(underway.done(first));
}
