//! The size of the window for each step: from the step's content, so it differs between steps.

use porter_core::sheet::{
    FieldKind, FieldSpec, Protocol, Review, ReviewView, SheetView, SignInFault, SignInView,
    manual_form,
};
use porter_core::{AccountLabel, ProviderId};

use super::map::{Sheet, Step, shown, step_of};
use super::size::{extent, fitted, least, opening};
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

#[test]
fn each_step_is_its_own_size_and_the_form_grows_with_its_fields() {
    let rows = ["google", "microsoft", "fastmail", "icloud", "yahoo", "gmx"]
        .map(|provider| porter_core::sheet::ProviderRow {
            auth: Default::default(),
            id: id(provider),
            label: provider.to_owned(),
            mark: "mail".to_owned(),
            kind: porter_core::sheet::RowKind::Provider,
            mark_face: None,
            group: None,
        })
        .to_vec();
    let providers = extent(Some(&step(SheetView::Providers(rows))));
    let first = extent(Some(&step(sign_in(vec![
        spec(FieldKind::Address),
        spec(FieldKind::Password),
    ]))));
    let imap = extent(Some(&step(sign_in(manual_form(
        Protocol::Imap,
        Some("example.test"),
    )))));
    let jmap = extent(Some(&step(sign_in(manual_form(
        Protocol::Jmap,
        Some("example.test"),
    )))));
    let working = extent(Some(&step(SheetView::Working {
        provider: id("generic-imap"),
        row: None,
    })));
    let failed = extent(Some(&step(SheetView::Failed {
        row: None,
        provider: id("generic-imap"),
        fault: SignInFault::Unreachable,
    })));
    let review = extent(Some(&step(SheetView::Review(ReviewView {
        row: None,
        provider: id("generic-imap"),
        review: Review {
            label: AccountLabel("ada@example.test".to_owned()),
            services: vec![],
            endpoints: vec![],
        },
        allow: None,
        allow_label: None,
    }))));

    // The window opens at the list, the first step.
    assert_eq!(opening(None).start(), providers);
    assert_eq!(opening(None).least(), Some(least()));
    assert!(providers.height > first.height, "{providers:?} {first:?}");
    // The short form is not the list's size; the server form is taller than it and than JMAP's.
    assert!(imap.height > first.height, "{imap:?} {first:?}");
    assert!(imap.height > jmap.height, "{imap:?} {jmap:?}");
    assert!(jmap.height > working.height, "{jmap:?} {working:?}");
    // The server form is read in parts, side by side: wider than the others.
    assert!(imap.width > first.width);
    assert_eq!(jmap.width, imap.width);
    // A status is small, and a window with nothing to draw is no bigger.
    assert!(working.height < first.height);
    assert_eq!(working, failed);
    assert!(review.height < providers.height);
    assert!(extent(None).height <= working.height);
    // Every size is one a person can use.
    for size in [providers, first, imap, jmap, working, failed, review] {
        assert!((320..=900).contains(&size.width) && (120..=900).contains(&size.height));
    }
}

fn spec(kind: FieldKind) -> FieldSpec {
    FieldSpec {
        kind,
        entry: porter_core::sheet::Entry::Plain,
        presence: porter_core::sheet::Presence::Required,
        prefill: None,
    }
}

#[test]
fn a_step_is_capped_to_most_of_the_screen_and_never_below_the_least() {
    let big = Extent::new(560, 645);
    // A roomy screen leaves it; a small one takes 85% of it; the least still holds.
    assert_eq!(fitted(big, Some(Extent::new(1920, 1080))), big);
    assert_eq!(fitted(big, None), big);
    assert_eq!(
        fitted(big, Some(Extent::new(1000, 600))),
        Extent::new(560, 510)
    );
    assert_eq!(
        fitted(Extent::new(100, 50), Some(Extent::new(1920, 1080))),
        least()
    );
    // On a small screen the window opens capped too.
    assert_eq!(
        opening(Some(Extent::new(1000, 400))).start(),
        Extent::new(480, 340)
    );
}
