//! Signatures: the accounts that send in a list on the left ([`super::account_split`]), and on the
//! right a pane of two levels for the chosen one. Its root has the signatures of each address the
//! account sends from, ending in New Signature (a group for each only when the account has more
//! than one address, aliases); a signature's row pushes its page.
//!
//! A signature's page is its name, its words, written in the composer's own body (`compose::Body`
//! over a page of its own that is never a draft), whether it is the default for new messages and
//! for replies, and Delete Signature. Everything is kept as it is changed: a name as it is typed,
//! the words with every edit, a switch when it is pressed; a change the client would not take is
//! said under the words, and the next one tries again. Delete Signature says so in a toast whose
//! Undo puts the signature back. The composer reads the client when it opens a message, so the
//! next one is signed with what was kept here.

use super::account_split::{AccountSplit, NoAccountsPage, shown};
use crate::ui::compose::{Body, Page};
use crate::ui::copy;
use crate::ui::data::{AccountRow, account_rows};
use crate::ui::motion::{Follow, Restore, tell};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::ButtonRole;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::Logged;
use mail_client::rich_text::{Doc, Node, ParaKind, Session};
use mail_client::signature;
use mail_domain::{Draft, Identity, IdentityId, IsDefault, Signature, SignatureId, SignatureUse};
use porter_core::AccountId;

/// The stack's element id: its back button is `signatures-pane-back-detail`. The accounts' rows
/// are `signatures-pane-open-<account>`.
const STACK: &str = "signatures-pane";

/// A page of the pane: the list, or one signature's page over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignaturesAt {
    List,
    Signature(SignatureId),
}

/// The element id of the row that pushes `signature`, where the keyboard goes back to.
fn row_id(signature: SignatureId) -> String {
    format!("signatures-pane-open-{signature}")
}

/// The element id of an identity's New Signature row.
fn new_row_id(identity: IdentityId) -> String {
    format!("signatures-pane-new-{identity}")
}

/// What an identity's group is headed: its name and address, or the address alone.
fn heading(identity: &Identity) -> String {
    match identity.from.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => format!("{name} <{}>", identity.from.email),
        _ => identity.from.email.clone(),
    }
}

/// The signatures whose last change was refused, for the pages to say under their words. Kept
/// with the pane and keyed to the signature, not to the page that drew it: the store moving draws
/// a page again, and a refusal must outlast that, even when what moved it was the signature
/// going.
#[derive(Clone, Copy)]
struct Refusals(Signal<Vec<SignatureId>>);

impl Refusals {
    fn has(self, id: SignatureId) -> bool {
        self.0.read().contains(&id)
    }

    fn set(mut self, id: SignatureId, refused: bool) {
        let mut all = self.0.write();
        all.retain(|held| *held != id);
        if refused {
            all.push(id);
        }
    }
}

/// One signature's [`Refusals`] entry.
#[derive(Clone, Copy)]
struct Unkept {
    refusals: Refusals,
    id: SignatureId,
}

impl Unkept {
    fn get(self) -> bool {
        self.refusals.has(self.id)
    }

    fn set(self, refused: bool) {
        self.refusals.set(self.id, refused);
    }
}

/// The page: the accounts that send, and the chosen one's signatures beside them.
#[component]
pub(super) fn Signatures(shell: Signal<Shell>) -> Element {
    use_context_provider(|| Refusals(Signal::new(Vec::new())));
    // Read again when an account comes or goes.
    let _ = crate::ui::live::moved_for(mail_client::Topics::ACCOUNTS);
    let client = crate::ui::live::client();
    let sending = client.compose().sending_accounts();
    let rows: Vec<AccountRow> = account_rows(&client)
        .into_iter()
        .filter(|row| sending.iter().any(|(_, id)| *id == row.id))
        .collect();
    if rows.is_empty() {
        return rsx! { NoAccountsPage { title: copy::titles::SIGNATURES } };
    }
    let held = shell.read().signatures_account.clone();
    let chosen = shown(&rows, held.as_ref()).cloned();
    rsx! {
        AccountSplit {
            pane: STACK,
            label: copy::titles::SIGNATURES,
            rows: rows.clone(),
            chosen: chosen.as_ref().map(|row| row.id.clone()),
            onpick: move |id: AccountId| {
                let mut shell = shell;
                shell.write().signatures_account = Some(id);
            },
            if let Some(row) = chosen {
                AccountSignatures { key: "{row.id}", shell, row }
            }
        }
    }
}

/// One account's signatures: its addresses' at the root, a signature's page over them.
#[component]
fn AccountSignatures(shell: Signal<Shell>, row: AccountRow) -> Element {
    let client = crate::ui::live::client();
    let mut path = use_signal(|| PanePath::new(SignaturesAt::List));
    let account = row.id.clone();
    let name = row.shown();
    let titled = client.clone();
    let title = use_callback(move |at: SignaturesAt| match at {
        SignaturesAt::List => name.clone(),
        SignaturesAt::Signature(id) => titled
            .stored()
            .signature(id)
            .map_or_else(|_| name.clone(), |kept| kept.name),
    });
    let page = use_callback(move |at: SignaturesAt| match at {
        SignaturesAt::List => rsx! { SignatureList { path, account: account.clone() } },
        SignaturesAt::Signature(id) => rsx! {
            SignaturePage { key: "{id}", shell, path, id }
        },
    });
    let opener = use_callback(|at: SignaturesAt| match at {
        SignaturesAt::List => None,
        SignaturesAt::Signature(id) => Some(row_id(id)),
    });
    rsx! {
        PaneStack::<SignaturesAt> {
            path: path(),
            title,
            page,
            opener: Some(opener),
            on_back: move |()| {
                let back = path.peek().popped();
                path.set(back);
            },
            common: Common { id: Some(STACK.to_owned()), ..Common::default() },
        }
    }
}

/// What a signature's row says after its name: which default it is, if either.
fn defaults_of(signature: &Signature) -> String {
    match (signature.new_mail, signature.replies) {
        (IsDefault::Default, IsDefault::Default) => copy::status::signature::DEFAULT.to_owned(),
        (IsDefault::Default, IsDefault::Alternate) => {
            copy::status::signature::NEW_MESSAGES.to_owned()
        }
        (IsDefault::Alternate, IsDefault::Default) => copy::status::signature::REPLIES.to_owned(),
        (IsDefault::Alternate, IsDefault::Alternate) => String::new(),
    }
}

/// The root: the account's identities, each with its signatures and New Signature, in a group of
/// its own titled by the address only where there is more than one to tell apart.
#[component]
fn SignatureList(path: Signal<PanePath<SignaturesAt>>, account: AccountId) -> Element {
    // Read again with each change: a signature made, renamed, made a default or deleted.
    let _ =
        crate::ui::live::moved_for(mail_client::Topics::IDENTITIES | mail_client::Topics::ACCOUNTS);
    let client = crate::ui::live::client();
    // Unreadable is nothing to list; the next change read again.
    let every: Vec<_> = client
        .signing()
        .every()
        .unwrap_or_default()
        .into_iter()
        .filter(|(identity, _)| identity.account == account)
        .collect();
    let aliased = every.len() > 1;
    rsx! {
        Form {
            for (identity, signatures) in every {
                FormSection { key: "{identity.id}", title: aliased.then(|| heading(&identity)),
                    List::<String> {
                        label: heading(&identity),
                        items: rows(&identity, &signatures, path),
                        style: ListStyle::Grouped,
                    }
                }
            }
        }
    }
}

/// An identity's rows: each signature, which pushes its page, then New Signature, which makes
/// one and pushes its page.
fn rows(
    identity: &Identity,
    signatures: &[Signature],
    path: Signal<PanePath<SignaturesAt>>,
) -> Vec<ListItem<String>> {
    let push = move |id: SignatureId| {
        let mut path = path;
        path.set(PanePath::new(SignaturesAt::List).pushed(SignaturesAt::Signature(id)));
    };
    let mut items: Vec<ListItem<String>> = signatures
        .iter()
        .map(|kept| {
            let id = kept.id;
            let name = kept.name.clone();
            ListItem::row(
                row_id(id),
                name.clone(),
                rsx! {
                    Row {
                        title: name.clone(),
                        accessory: Accessory::Text(defaults_of(kept)),
                        size: RowSize::Settings,
                        onclick: move |_| push(id),
                        common: Common {
                            id: Some(row_id(id)),
                            aria_label: Some(copy::aria::details_for(&name)),
                            ..Common::default()
                        },
                    }
                },
            )
        })
        .collect();
    let whose = identity.id;
    items.push(ListItem::row(
        new_row_id(whose),
        copy::controls::NEW_SIGNATURE.to_owned(),
        rsx! {
            Row {
                leading: crate::ui::common::add_mark(),
                title: copy::controls::NEW_SIGNATURE,
                size: RowSize::Settings,
                onclick: move |_| {
                    let client = crate::ui::live::client();
                    if let Ok(made) = client.signing().create(whose, "") {
                        push(made.id);
                    }
                },
                common: Common {
                    id: Some(new_row_id(whose)),
                    aria_label: Some(copy::aria::new_signature_for(&identity.from.email)),
                    ..Common::default()
                },
            }
        },
    ));
    items
}

/// The page the composer's body writes on: never a draft, never saved as one. Its document is
/// the signature's, and the rest is what a blank page from the identity would be.
fn editing(kept: &Signature, identity: &Identity) -> Page {
    let blank = Draft::blank(identity, chrono::Utc::now());
    let mut page = Page::opened(&blank, Vec::new(), Vec::new(), &[]);
    let mut nodes = signature::nodes_of(kept);
    if !matches!(nodes.last(), Some(Node::Para { .. })) {
        nodes.push(Node::plain(ParaKind::Paragraph, ""));
    }
    page.session = Session::with(Doc { nodes });
    page
}

/// A signature's page: its name, its words, its defaults, and Delete Signature.
#[component]
fn SignaturePage(
    shell: Signal<Shell>,
    path: Signal<PanePath<SignaturesAt>>,
    id: SignatureId,
) -> Element {
    // Read again with each change, so the switches say what was kept.
    let _ =
        crate::ui::live::moved_for(mail_client::Topics::IDENTITIES | mail_client::Topics::ACCOUNTS);
    let client = crate::ui::live::client();
    let found = client.stored().signature(id).ok().and_then(|kept| {
        let identity = client.stored().identity(kept.identity).ok().flatten()?;
        Some((kept, identity))
    });
    let Some((kept, identity)) = found else {
        // Deleted from elsewhere while the page was shown, or its identity with it. A change
        // refused on the way is still said: it was the person's, and it was lost.
        return if use_context::<Refusals>().has(id) {
            rsx! {
                Form {
                    FormSection {
                        footer: Some(copy::errors::NOT_KEPT_HERE.to_owned()),
                        FieldRow { label: copy::empty::SIGNATURE_GONE }
                    }
                }
            }
        } else {
            rsx! {}
        };
    };
    rsx! { SignatureEditor { shell, path, kept, identity } }
}

/// [`SignaturePage`] once its signature and identity are read.
#[component]
fn SignatureEditor(
    shell: Signal<Shell>,
    path: Signal<PanePath<SignaturesAt>>,
    kept: Signature,
    identity: Identity,
) -> Element {
    let id = kept.id;
    let page = use_signal({
        let kept = kept.clone();
        move || editing(&kept, &identity)
    });
    // The name as typed: a blank one is kept as the unnamed signature, and is still blank here.
    let mut typed = use_signal(|| kept.name.clone());
    // Whether the last name or words the page tried to keep were refused: said under the words
    // until a later change is kept.
    let unkept = Unkept {
        refusals: use_context::<Refusals>(),
        id,
    };
    // The words, kept with every edit: `edits` moves with each, and only then.
    let mut kept_at = use_signal(|| 0u64);
    use_effect(move || {
        let edits = page.read().edits;
        if edits == *kept_at.peek() {
            return;
        }
        kept_at.set(edits);
        let client = crate::ui::live::client();
        // Kept, or not: the words stay on the page either way, and the next edit tries again.
        let kept = client
            .signing()
            .write(id, &page.peek().session.doc)
            .or_log("the signature's words were not kept");
        unkept.set(kept.is_none());
    });
    let default_row = |purpose: SignatureUse, label: &'static str| {
        let on = kept.default_for(purpose) == IsDefault::Default;
        ListItem::row(
            label.to_owned(),
            label.to_owned(),
            rsx! {
                Row {
                    title: label,
                    size: RowSize::Settings,
                    accessory: Accessory::Toggle {
                        value: if on { Check::On } else { Check::Off },
                        on_toggle: EventHandler::new(move |_| {
                            let client = crate::ui::live::client();
                            let next = if on { IsDefault::Alternate } else { IsDefault::Default };
                            let made = client.signing().make_default(id, purpose, next)
                                .or_log("the signature's default was not kept");
                            unkept.set(made.is_none());
                        }),
                    },
                }
            },
        )
    };
    let defaults = vec![
        default_row(
            SignatureUse::NewMail,
            copy::fields::DEFAULT_FOR_NEW_MESSAGES,
        ),
        default_row(SignatureUse::Replies, copy::fields::DEFAULT_FOR_REPLIES),
    ];
    let name = kept.name.clone();
    rsx! {
        Form {
            FormSection {
                FieldRow { label: copy::fields::NAME,
                    TextField {
                        label: copy::fields::SIGNATURE_NAME,
                        value: typed(),
                        placeholder: mail_domain::UNNAMED_SIGNATURE,
                        oninput: move |now: String| {
                            typed.set(now.clone());
                            let client = crate::ui::live::client();
                            // As above: the name stays as typed, and the next key tries again.
                            let kept = client.signing().rename(id, &now)
                                .or_log("the signature's name was not kept");
                            unkept.set(kept.is_none());
                        },
                    }
                }
            }
            FormSection {
                title: Some(copy::fields::SIGNATURE.to_owned()),
                footer: unkept.get().then(|| copy::errors::NOT_KEPT_HERE.to_owned()),
                div { class: "sig-edit",
                    Body { page, shell, on_attach: move |()| {} }
                }
            }
            FormSection {
                List::<String> {
                    label: copy::fields::SIGNATURE,
                    items: defaults,
                    style: ListStyle::Grouped,
                }
            }
            div { class: "acct-actions",
                Button {
                    label: copy::controls::DELETE_SIGNATURE,
                    role: ButtonRole::Destructive,
                    common: Common { aria_label: Some(copy::aria::delete_signature(&name)), ..Common::default() },
                    onclick: crate::ui::press::on_primary(move || {
                        let client = crate::ui::live::client();
                        let Some(had) = client.signing().delete(id)
                            .or_log("the signature was not deleted")
                        else {
                            unkept.set(true);
                            return;
                        };
                        let said = copy::toasts::signature_deleted(&had.name);
                        tell(said, Follow::Restore(Restore::Signature(had)));
                        let mut path = path;
                        path.set(PanePath::new(SignaturesAt::List));
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "signatures_tests.rs"]
mod tests;
