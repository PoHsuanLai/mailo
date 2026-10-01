use super::*;

/// Five listed conversations, top to bottom, and one that is not listed.
fn ids() -> ([ThreadId; 5], ThreadId) {
    let make = |n: u128| ThreadId::from_uuid(uuid::Uuid::from_u128(n));
    ([make(1), make(2), make(3), make(4), make(5)], make(99))
}

/// A gesture on a `Picked`, by the row numbers (0-based) it names.
#[derive(Debug, Clone, Copy)]
enum Gesture {
    Click(usize),
    Toggle(usize),
    Range(usize),
    Extend(Toward),
    All,
}

/// Play `gestures` from nothing picked with row `open` open (`None`: nothing open), and give the
/// rows picked, in list order.
///
/// A plain click opens the row, as the window's does, so it moves `open` too.
fn play(open: Option<usize>, gestures: &[Gesture]) -> Vec<usize> {
    let (ids, _) = ids();
    let mut open = open.map(|n| ids[n]);
    let mut picked = Picked::none();
    for gesture in gestures {
        picked = match *gesture {
            Gesture::Click(n) => {
                open = Some(ids[n]);
                Picked::clicked(ids[n])
            }
            Gesture::Toggle(n) => picked.toggle(ids[n], open, &ids),
            Gesture::Range(n) => picked.range(ids[n], open, &ids),
            Gesture::Extend(toward) => picked.extend(toward, open, &ids),
            Gesture::All => Picked::all(&ids),
        };
    }
    picked
        .chosen(&ids)
        .iter()
        .map(|id| ids.iter().position(|it| it == id).unwrap())
        .collect()
}

#[test]
fn each_gesture_picks_what_a_list_that_selects_ranges_would() {
    use Gesture::*;
    use Toward::*;
    type Case = (
        &'static str,
        Option<usize>,
        &'static [Gesture],
        &'static [usize],
    );
    const CASES: &[Case] = &[
        ("nothing done picks nothing", Some(1), &[], &[]),
        (
            "a plain click opens and picks nothing",
            None,
            &[Click(2)],
            &[],
        ),
        // Toggle.
        (
            "ctrl-click with nothing open picks that row",
            None,
            &[Toggle(2)],
            &[2],
        ),
        (
            "ctrl-click adds to the open row",
            Some(0),
            &[Toggle(2)],
            &[0, 2],
        ),
        (
            "ctrl-click again takes it out",
            Some(0),
            &[Toggle(2), Toggle(2)],
            &[0],
        ),
        (
            "ctrl-click on the open row alone unpicks it",
            Some(1),
            &[Toggle(1)],
            &[],
        ),
        (
            "three ctrl-clicks, in list order",
            None,
            &[Toggle(4), Toggle(0), Toggle(2)],
            &[0, 2, 4],
        ),
        // Range.
        (
            "shift-click from the open row",
            Some(1),
            &[Range(3)],
            &[1, 2, 3],
        ),
        ("shift-click upward", Some(3), &[Range(1)], &[1, 2, 3]),
        (
            "shift-click with nothing open is that row",
            None,
            &[Range(2)],
            &[2],
        ),
        (
            "shift-click measures from the clicked row",
            None,
            &[Click(1), Range(3)],
            &[1, 2, 3],
        ),
        (
            "a second shift-click re-measures from the anchor",
            Some(2),
            &[Range(4), Range(0)],
            &[0, 1, 2],
        ),
        (
            "shift-click replaces what ctrl-click picked",
            Some(0),
            &[Toggle(4), Range(2)],
            &[2, 3, 4],
        ),
        (
            "ctrl-click moves the anchor",
            Some(0),
            &[Toggle(3), Range(4)],
            &[3, 4],
        ),
        // Extend.
        (
            "shift+j from the open row",
            Some(1),
            &[Extend(Next)],
            &[1, 2],
        ),
        (
            "shift+j twice",
            Some(1),
            &[Extend(Next), Extend(Next)],
            &[1, 2, 3],
        ),
        (
            "shift+k takes back a shift+j",
            Some(1),
            &[Extend(Next), Extend(Next), Extend(Previous)],
            &[1, 2],
        ),
        (
            "shift+k past the anchor turns the range",
            Some(2),
            &[Extend(Previous), Extend(Previous)],
            &[0, 1, 2],
        ),
        (
            "shift+j with nothing open starts at the top",
            None,
            &[Extend(Next)],
            &[0],
        ),
        (
            "shift+k with nothing open starts at the bottom",
            None,
            &[Extend(Previous)],
            &[4],
        ),
        (
            "shift+j stops at the bottom",
            Some(3),
            &[Extend(Next), Extend(Next), Extend(Next)],
            &[3, 4],
        ),
        (
            "shift+j after a click extends from it",
            Some(0),
            &[Click(2), Extend(Next)],
            &[2, 3],
        ),
        // All.
        ("select all", Some(3), &[All], &[0, 1, 2, 3, 4]),
        (
            "ctrl-click takes one out of all",
            None,
            &[All, Toggle(2)],
            &[0, 1, 3, 4],
        ),
        (
            "a click after select all clears it",
            None,
            &[All, Click(1)],
            &[],
        ),
    ];
    for (name, open, gestures, want) in CASES {
        assert_eq!(play(*open, gestures), *want, "{name}");
    }
}

#[test]
fn what_is_picked_is_read_through_the_list() {
    // Picked, then no longer listed (archived, or a search replaced the list): an action on the
    // selection must not reach a conversation nobody can see.
    let (ids, gone) = ids();
    let picked = Picked::none()
        .toggle(ids[1], None, &ids)
        .toggle(ids[3], None, &ids);
    let shorter = [ids[0], ids[1], ids[2], ids[4]];
    assert_eq!(picked.chosen(&shorter), vec![ids[1]]);
    assert!(picked.holds(ids[1], &shorter));
    assert!(!picked.holds(ids[3], &shorter));
    assert!(picked.any(&shorter));
    assert!(!picked.any(&[ids[0], gone]));
    // A range towards a row that is not listed picks nothing.
    assert_eq!(picked.range(gone, Some(ids[0]), &ids).chosen(&ids), vec![]);
    // An open row that is not listed is not picked along with a ctrl-click.
    assert_eq!(
        Picked::none().toggle(ids[2], Some(gone), &ids).chosen(&ids),
        vec![ids[2]]
    );
    // An empty list has nothing to extend over.
    assert_eq!(
        Picked::none().extend(Toward::Next, None, &[]),
        Picked::none()
    );
}
