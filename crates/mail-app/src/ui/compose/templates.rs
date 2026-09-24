//! Templates in the window: "Save as template…" and "Start from a template" in the `/` menu, and
//! the rows every list of templates draws, here and in Ctrl T.
//!
//! Every write goes through `crate::template`, the module `mailo template` uses. Templates are
//! local only; see `mail_domain::template`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{Draft, Template, TemplateId};
use mail_store::SqliteStore;

use super::super::field::{Field, FieldKind};
use super::super::menu::{Menu, MenuItem, MenuKey, Right, Tile, menu_key};
use super::super::motion::{Follow, tell};
use super::desk::{self, Desk};
use super::float::{commit, query, slash_items};
use super::life;
use super::page::{Float, Page, PageKind, Phase, address};
use crate::editor::{Caret, Node, Op, Range, runs_text};
use crate::view::Shell;
use ds::{Glyph, Icon};

/// The `/` row that keeps the message as a template.
pub(in crate::ui) const SAVE_KEY: &str = "template:save";
/// The `/` row that starts the message from one, offered while the body is empty.
pub(in crate::ui) const START_KEY: &str = "template:start";

/// How much of a template's first line its row shows.
const FIRST_LINE: usize = 60;

/// The `/` menu on this page: the editor's rows, then what can be done with the whole message.
pub(in crate::ui) fn page_slash_items(page: &Page) -> Vec<MenuItem> {
    let typed = query(page).unwrap_or_default();
    let wanted = typed.trim().to_lowercase();
    let mut items = slash_items(&typed);
    let mut offer = |key: &str, name: &str, help: &str, words: &[&str]| {
        let hit = wanted.is_empty()
            || name.to_lowercase().starts_with(&wanted)
            || words.iter().any(|word| word.starts_with(&wanted));
        if hit {
            items.push(MenuItem {
                key: key.to_owned(),
                tile: Tile::Icon(Icon::FilePen),
                name: name.to_owned(),
                help: Some(help.to_owned()),
                right: Right::None,
                group: wanted.is_empty().then(|| "This message".to_owned()),
                marks: Vec::new(),
                title: Vec::new(),
                detail: Vec::new(),
            });
        }
    };
    if page.kind == PageKind::New && body_is_empty(page, &typed) {
        offer(
            START_KEY,
            "Start from a template",
            "A message you kept, in place of this empty one",
            &["template", "start"],
        );
    }
    offer(
        SAVE_KEY,
        "Save as template…",
        "Keep this message to start others from",
        &["template", "save", "keep"],
    );
    items
}

/// Whether the body holds nothing but the `/` being typed.
fn body_is_empty(page: &Page, typed: &str) -> bool {
    let mut text = String::new();
    for node in &page.session.doc.nodes {
        match node {
            Node::Para { runs, .. } => text.push_str(&runs_text(runs)),
            Node::Object(_) => return false,
        }
    }
    text.trim() == format!("/{typed}").trim()
}

/// A template row was picked from `/`: the typed `/…` goes, and the name field or the list
/// opens in its place. `false` for any other row, which is the editor's.
pub(in crate::ui) fn pick(page: &mut Page, key: &str) -> bool {
    let next = match key {
        SAVE_KEY => Float::SaveTemplate(String::new()),
        START_KEY => Float::Templates { active: 0 },
        _ => return false,
    };
    if let Float::Slash { anchor, .. } = page.float {
        let range = Range {
            start: anchor,
            end: page.session.caret.pos,
        };
        commit(
            page,
            vec![Op::Delete { range }],
            Caret::at(anchor.node, anchor.offset),
        );
    }
    page.float = next;
    true
}

/// The rows of a list of templates, `(account address, template)`, narrowed to `typed`: the
/// name, the address it leaves from and its first line, and a × to delete it.
pub(in crate::ui) fn template_rows(all: &[(String, Template)], typed: &str) -> Vec<MenuItem> {
    let typed = typed.trim();
    all.iter()
        .filter_map(|(from, template)| {
            let marks = if typed.is_empty() {
                Vec::new()
            } else {
                crate::search::match_list(typed, &[template.name.as_str()])
                    .into_iter()
                    .next()?
                    .indices
            };
            Some(MenuItem {
                key: template.id.to_string(),
                tile: Tile::Icon(Icon::FilePen),
                name: template.name.clone(),
                help: Some(format!("{from} · {}", first_line(template))),
                right: Right::Remove(format!("Delete template “{}”", template.name)),
                group: None,
                marks,
                title: Vec::new(),
                detail: Vec::new(),
            })
        })
        .collect()
}

/// The first line a template says, else its subject: what a row recognises it by.
fn first_line(template: &Template) -> String {
    let line = template
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(template.subject.as_str());
    let line = if line.is_empty() { "(empty)" } else { line };
    if line.chars().count() > FIRST_LINE {
        format!("{}…", line.chars().take(FIRST_LINE).collect::<String>())
    } else {
        line.to_owned()
    }
}

/// The template a row's key names.
pub(in crate::ui) fn named(all: &[(String, Template)], key: &str) -> Option<TemplateId> {
    all.iter()
        .map(|(_, template)| template.id)
        .find(|id| id.to_string() == key)
}

/// Every template, or none when the store cannot say.
pub(in crate::ui) fn every(store: &SqliteStore) -> Vec<(String, Template)> {
    crate::template::all(store).unwrap_or_default()
}

/// Delete the template `key` names, returning what the toast says.
pub(in crate::ui) fn forget(store: &SqliteStore, key: &str) -> Result<String, String> {
    let id = named(&every(store), key).ok_or_else(|| "that template is gone".to_owned())?;
    let name = crate::template::delete(store, id)?;
    Ok(format!("Deleted template “{name}”"))
}

/// Keep the page's message as a template under the name typed, returning what the toast says.
/// A blank name is the subject's, as `mailo template save` has it.
pub(in crate::ui) fn save_named(
    store: &SqliteStore,
    page: &mut Page,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let Float::SaveTemplate(name) = page.float.clone() else {
        return Err("there is no name to save it under".to_owned());
    };
    let draft = life::save(store, page, now)?;
    let kept = crate::template::save(store, draft.id, &name, now)?;
    page.float = Float::Closed;
    Ok(format!("Saved as template “{}”", kept.name))
}

/// Start a message from `template` in place of this empty page: the new draft goes to whoever
/// the page is addressed to, or the template's own `To` when it is addressed to nobody, and the
/// empty draft it replaces is discarded so Today keeps no blank one.
pub(in crate::ui) fn start_from(
    store: &SqliteStore,
    page: &Page,
    template: TemplateId,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let to: Vec<_> = page.to.iter().map(address).collect();
    let started = crate::template::start(store, template, &to, now)?;
    crate::compose::discard(store, page.draft)?;
    Ok(started)
}

/// Picked from the page's list: the new draft opens as the page.
fn start_here(mut page: Signal<Page>, mut shell: Signal<Shell>, key: &str) {
    let store = consume_context::<Arc<SqliteStore>>();
    let Some(id) = named(&every(&store), key) else {
        return;
    };
    let started = start_from(&store, &page.peek(), id, Utc::now());
    match started {
        Ok(draft) => {
            let old = page.peek().draft;
            page.write().phase = Phase::Closed;
            if let Some(desk) = try_consume_context::<Desk>() {
                desk::unpark(desk, old);
            }
            shell.write().compose(&draft);
        }
        Err(why) => page.write().notice = Some(why),
    }
}

/// Delete from the page's list, and say so.
fn forget_here(mut page: Signal<Page>, key: &str) {
    let store = consume_context::<Arc<SqliteStore>>();
    match forget(&store, key) {
        Ok(said) => tell(said, Follow::Nothing),
        Err(why) => page.write().notice = Some(why),
    }
    // The list is read from the store as it is drawn; this draws it again.
    let mut write = page.write();
    if let Float::Templates { active } = &mut write.float {
        *active = 0;
    }
}

/// Enter or Save in the name field.
fn save_here(mut page: Signal<Page>) {
    let store = consume_context::<Arc<SqliteStore>>();
    let saved = save_named(&store, &mut page.write(), Utc::now());
    match saved {
        Ok(said) => tell(said, Follow::Nothing),
        Err(why) => page.write().notice = Some(why),
    }
}

/// Keys the page's list takes while it is open: the arrows, Enter and Esc. `true` when taken.
pub(in crate::ui) fn key(page: Signal<Page>, shell: Signal<Shell>, name: &str) -> bool {
    let Float::Templates { active } = page.peek().float else {
        return false;
    };
    let mut page = page;
    let store = consume_context::<Arc<SqliteStore>>();
    let rows = template_rows(&every(&store), "");
    let count = rows.len().max(1);
    match menu_key(name) {
        Some(MenuKey::Down) => {
            page.write().float = Float::Templates {
                active: (active + 1) % count,
            }
        }
        Some(MenuKey::Up) => {
            page.write().float = Float::Templates {
                active: (active + count - 1) % count,
            }
        }
        Some(MenuKey::Enter) => {
            if let Some(row) = rows.get(active) {
                start_here(page, shell, &row.key);
            }
        }
        Some(MenuKey::Escape) => page.write().float = Float::Closed,
        _ => return false,
    }
    true
}

/// At the caret: the name field for "Save as template…", or the list to start from.
#[component]
pub(in crate::ui) fn TemplateFloat(page: Signal<Page>, shell: Signal<Shell>) -> Element {
    let float = page.read().float.clone();
    match float {
        Float::SaveTemplate(name) => {
            let placeholder = match page.read().subject.trim() {
                "" => "Template name".to_owned(),
                subject => subject.to_owned(),
            };
            rsx! {
                div { class: "fmenu slim tpl-save",
                    div { class: "g", "Save as template" }
                    div {
                        class: "pick-in",
                        onkeydown: move |event: KeyboardEvent| match menu_key(&event.key().to_string()) {
                            Some(MenuKey::Enter) => {
                                event.prevent_default();
                                save_here(page);
                            }
                            Some(MenuKey::Escape) => {
                                event.stop_propagation();
                                page.write().float = Float::Closed;
                            }
                            _ => {}
                        },
                        Glyph { icon: Icon::FilePen }
                        Field {
                            kind: FieldKind::Inline,
                            value: name,
                            placeholder,
                            extra: Some("tpl-name".to_owned()),
                            on_input: move |value: String| page.write().float = Float::SaveTemplate(value),
                            on_focus: |_| {},
                            on_blur: |_| {},
                        }
                    }
                    p { class: "pick-says", "Enter keeps it. The message stays as it is." }
                }
            }
        }
        Float::Templates { active } => {
            let store = consume_context::<Arc<SqliteStore>>();
            let rows = template_rows(&every(&store), "");
            if rows.is_empty() {
                return rsx! {
                    div { class: "fmenu slim tpl-list",
                        div { class: "g", "Start from a template" }
                        div { class: "none", "No templates yet. Write one, then type / and choose Save as template…" }
                    }
                };
            }
            let active = active.min(rows.len() - 1);
            rsx! {
                Menu {
                    title: "Start from a template".to_owned(),
                    items: rows,
                    filterable: false,
                    on_pick: move |key: String| start_here(page, shell, &key),
                    on_close: move |_| page.write().float = Float::Closed,
                    on_query: move |_| {},
                    slim: false,
                    active: Some(active),
                    on_remove: move |key: String| forget_here(page, &key),
                }
            }
        }
        _ => rsx! {},
    }
}
