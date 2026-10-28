//! The window's width for each step, and the size it opens at. Its height is measured from the
//! step as it is laid out (`tests/app/native_add_account.rs`).

use porter_core::ProviderId;
use porter_sheet::{
    FieldKind, FieldSpec, Protocol, SheetView, SignInFault, SignInView, manual_form,
};

use super::map::{Sheet, Step, shown, step_of};
use super::size::{least, opening, width};
use ds_blitz::Extent;

fn id(text: &str) -> ProviderId {
    ProviderId::parse(text).unwrap()
}

fn sign_in(fields: Vec<FieldSpec>) -> SheetView {
    SheetView::SignIn(SignInView {
        row: None,
        provider: id("generic-imap"),
        fields,
        problem: None,
    })
}

fn step(view: SheetView) -> Step {
    step_of(&shown(Sheet::default(), view).0).expect("a step")
}

fn spec(kind: FieldKind) -> FieldSpec {
    FieldSpec {
        kind,
        entry: porter_sheet::Entry::Plain,
        presence: porter_sheet::Presence::Required,
        prefill: None,
    }
}

#[test]
fn the_server_form_is_wider_than_every_other_step() {
    let short = sign_in(vec![spec(FieldKind::Address), spec(FieldKind::Password)]);
    let imap = sign_in(manual_form(Protocol::Imap, Some("example.test")));
    let jmap = sign_in(manual_form(Protocol::Jmap, Some("example.test")));
    let failed = SheetView::Failed {
        row: None,
        provider: id("generic-imap"),
        fault: SignInFault::NeedsClientId,
    };
    let narrow = width(Some(&step(short)));
    assert_eq!(narrow, width(Some(&step(failed))));
    assert_eq!(narrow, width(None));
    assert!(width(Some(&step(imap))) > narrow);
    assert_eq!(
        width(Some(&step(jmap))),
        width(Some(&step(sign_in(manual_form(
            Protocol::Imap,
            Some("example.test")
        )))))
    );
}

#[test]
fn the_window_opens_at_the_list_capped_to_the_screen_and_never_below_the_least() {
    let roomy = opening(Some(Extent::new(1920, 1080)));
    assert_eq!(roomy.start(), Extent::new(480, 493));
    assert_eq!(roomy.least(), Some(least()));
    assert_eq!(opening(None).start(), roomy.start());
    // On a small screen the window opens at 85% of it.
    assert_eq!(
        opening(Some(Extent::new(1000, 400))).start(),
        Extent::new(480, 340)
    );
}
