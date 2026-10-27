//! The signature on the page: which of the identity's it is, the Signature row that picks
//! another, and the change From makes to it.
//!
//! The signature is the paragraphs under the `-- ` separator (`mail_core::signature`). A draft is
//! kept as text until the page writes it, so a signature arrives here as its words alone; the
//! page puts its formatting back as it opens, when its words are one of the identity's. The row
//! is drawn while the identity has a signature to pick, so an account with none is asked nothing.

use dioxus::prelude::*;
use mail_client::Client;
use mail_client::rich_text::{Caret, Doc, apply};
use mail_client::signature::{self, Placement};
use mail_domain::{Draft, Signature, SignatureId};

use super::super::menu::{MenuItem, Right};
use super::float::commit;
use super::page::{Float, Page};
use super::props::ChoiceRow;
use crate::ui::copy;

/// The menu key of "None", which takes the signature off.
const NONE_KEY: &str = "signature:none";

/// The page's identity's signatures, or none when they cannot be read: the row is then not drawn,
/// and the message is signed as it was opened.
fn signatures(client: &Client, page: &Page) -> Vec<Signature> {
    // Unreadable is none to pick: the row goes, and the signature on the page stays as it is.
    client
        .signing()
        .of_identity(page.identity)
        .unwrap_or_default()
}

/// Where a signature the page has none of goes: the person's setting.
fn placement() -> Placement {
    crate::ui::prefs::current()
        .compose
        .signature_above_quote
        .placement()
}

/// `page`, just opened on `draft`, knowing which of its identity's signatures it has, with that
/// signature's own formatting in place of its words.
pub(in crate::ui) fn adopt(client: &Client, page: &mut Page) {
    let known = signatures(client, page);
    let Some((id, at)) = signature::recognise(&page.session.doc, &known) else {
        return;
    };
    page.signature = Some(id);
    let Some(rich) = known.iter().find(|signature| signature.id == id) else {
        return;
    };
    let mut nodes = vec![mail_client::rich_text::Node::Object(
        mail_client::rich_text::Object::Signature,
    )];
    nodes.extend(signature::nodes_of(rich));
    // Not an edit: the draft says this already, so nothing is to be saved and nothing undone.
    page.session.doc.nodes.splice(at, nodes);
}

/// Make the page's signature `next`, or take it off with `None`, as one edit Undo takes back.
fn put(page: &mut Page, known: &[Signature], next: Option<&Signature>) {
    page.float = Float::Closed;
    let swapped = signature::swap(&page.session.doc, known, next, placement());
    if let Some((op, at)) = swapped {
        let mut after: Doc = page.session.doc.clone();
        if apply(&mut after, op.clone()).is_ok() {
            // The caret stays where it was, unless the signature took the place it was in.
            let caret = page.session.caret;
            let keep = caret.pos.node < at && caret.pos.node < after.nodes.len();
            let caret = if keep {
                caret
            } else {
                Caret::at(at.saturating_sub(1), 0)
            };
            commit(page, vec![op], caret);
        }
    }
    page.signature = next.map(|signature| signature.id);
}

/// From moved the draft to `moved`'s identity: the signature becomes that identity's default, in
/// place of the one the page had from the identity it left.
pub(in crate::ui) fn follow_from(client: &Client, page: &mut Page, moved: &Draft) {
    if page.identity == moved.identity {
        return;
    }
    let known = signatures(client, page);
    // A default that cannot be read is none: the signature comes off rather than staying the
    // identity's the draft left.
    let next = client
        .signing()
        .default_for(moved.identity, signature::purpose_of(moved))
        .ok()
        .flatten();
    page.identity = moved.identity;
    put(page, &known, next.as_ref());
}

/// The Signature row's menu: "None", then each of the identity's, the one on the page checked.
fn items(known: &[Signature], on: Option<SignatureId>) -> Vec<MenuItem> {
    let none = MenuItem {
        right: Right::Check(on.is_none()),
        ..MenuItem::plain(NONE_KEY, copy::menus::NO_SIGNATURE)
    };
    std::iter::once(none)
        .chain(known.iter().map(|signature| MenuItem {
            right: Right::Check(on == Some(signature.id)),
            ..MenuItem::plain(signature.id.to_string(), signature.name.clone())
        }))
        .collect()
}

/// The Signature row: the identity's signature on the page, and the menu that picks another.
/// Drawn only while the identity has one to pick.
#[component]
pub(in crate::ui) fn SignatureRow(page: Signal<Page>) -> Element {
    let client = crate::ui::live::client();
    let known = signatures(&client, &page.read());
    if known.is_empty() {
        return rsx! {};
    }
    let on = page.read().signature;
    let value = known
        .iter()
        .find(|signature| Some(signature.id) == on)
        .map_or_else(
            || copy::menus::NO_SIGNATURE.to_owned(),
            |signature| signature.name.clone(),
        );
    let menu = items(&known, on);
    rsx! {
        ChoiceRow {
            page,
            label: copy::fields::SIGNATURE,
            row: "signature",
            value,
            float: Float::Signature,
            items: menu,
            on_pick: move |key: String| {
                let mut page = page;
                let next = known.iter().find(|signature| signature.id.to_string() == key);
                put(&mut page.write(), &known, next);
            },
        }
    }
}
