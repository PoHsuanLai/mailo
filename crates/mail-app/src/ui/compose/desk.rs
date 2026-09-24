//! Where drafts wait: the page that is open, the ones parked in Today, and the send that can
//! still be taken back. One context, provided by the app, so the keyboard, the sidebar and the
//! outbox pill all reach the same drafts.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::DraftId;
use mail_store::{SqliteStore, Store};

use super::super::motion::{Follow, tell};
use super::life;
use super::page::{Page, Phase, When};
use crate::appearance::WindowDirs;
use crate::space::Spaces;
use crate::today::Today;
use crate::view::Shell;
use ds::{Glyph, Icon};

/// A send that is queued and may still be taken back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Outgoing {
    pub draft: DraftId,
    /// When it may leave the outbox.
    pub due: DateTime<Utc>,
    pub when: When,
    /// The page as it was sent, so Undo brings back exactly this.
    pub page: Page,
    /// Why the last Undo or Cancel was refused, said on the pill.
    pub refused: Option<String>,
}

/// The drafts the window is holding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::ui) struct Desk {
    /// The page on screen, when one is.
    pub current: Signal<Option<Signal<Page>>>,
    /// Pages put aside, exactly as they were left.
    pub parked: Signal<Vec<Page>>,
    /// The last send, while the pill is showing it.
    pub outbox: Signal<Option<Outgoing>>,
    pub today: Signal<Today>,
    pub spaces: Signal<Spaces>,
    pub dirs: Signal<Option<WindowDirs>>,
    /// The sidebar, which focus mode hides.
    pub side_hidden: Signal<bool>,
}

/// Provide the desk to everything under the app.
pub(in crate::ui) fn use_desk(
    today: Signal<Today>,
    spaces: Signal<Spaces>,
    dirs: Option<WindowDirs>,
    side_hidden: Signal<bool>,
) -> Desk {
    use_context_provider(|| Desk {
        current: Signal::new(None),
        parked: Signal::new(Vec::new()),
        outbox: Signal::new(None),
        today,
        spaces,
        dirs: Signal::new(dirs),
        side_hidden,
    })
}

fn save_today(desk: Desk) {
    if let Some(dirs) = desk.dirs.peek().as_ref() {
        let _ = crate::today::save(&dirs.state, &desk.today.peek());
    }
}

/// Put the open page aside and close it. Esc, the park button, and anything that would redraw
/// the reader all come here.
pub(in crate::ui) fn park(desk: Desk, mut page: Signal<Page>, mut shell: Signal<Shell>) {
    keep(desk, page);
    page.write().phase = Phase::Closed;
    shell.write().close_composer();
}

/// Park the page on screen, if there is one. The app's Esc uses this.
pub(in crate::ui) fn park_current(desk: Desk, shell: Signal<Shell>) {
    let current = *desk.current.peek();
    match current {
        Some(page) => park(desk, page, shell),
        None => {
            let mut shell = shell;
            shell.write().close_composer();
        }
    }
}

/// Save the page and list it in Today, without touching what is on screen.
pub(in crate::ui) fn keep(mut desk: Desk, page: Signal<Page>) {
    let Ok(snapshot) = page.try_peek().map(|page| page.clone()) else {
        return;
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let space = desk.spaces.peek().current;
    let parked = life::park(
        &store,
        snapshot,
        &mut desk.parked.write(),
        &mut desk.today.write(),
        space,
        Utc::now(),
    );
    if let Err(why) = parked {
        eprintln!("park: {why}");
    }
    save_today(desk);
}

/// The draft is open, sent or gone: it leaves Today.
pub(in crate::ui) fn unpark(mut desk: Desk, draft: DraftId) {
    desk.today.write().unpark(draft);
    save_today(desk);
}

/// A send queued from somewhere other than a page — an unsubscribe message — on the pill like
/// any other: waiting in the outbox, with Undo bringing it back as a page.
pub(in crate::ui) fn show_queued(
    mut desk: Desk,
    store: &SqliteStore,
    draft: DraftId,
    now: DateTime<Utc>,
) {
    let Some(page) = life::load(store, draft, &mut Vec::new()) else {
        return;
    };
    desk.outbox.set(Some(Outgoing {
        draft,
        due: now,
        when: When::Now,
        page,
        refused: None,
    }));
}

/// Undo or Cancel, with a refusal said in words: on the pill, when it is showing this send, and
/// in the toast. Nothing changes when it is refused.
pub(in crate::ui) fn take_back_said(
    mut desk: Desk,
    shell: Signal<Shell>,
    draft: DraftId,
) -> Result<(), String> {
    take_back(desk, shell, draft).map_err(|why| {
        let said = refusal(&why);
        if let Some(out) = desk.outbox.write().as_mut()
            && out.draft == draft
        {
            out.refused = Some(said.clone());
        }
        tell(said.clone(), Follow::Nothing);
        said
    })
}

/// What a refused Undo or Cancel says.
pub(in crate::ui) fn refusal(why: &str) -> String {
    format!("Too late to take it back: {why}")
}

/// Withdraw a send that has not left, and open its page again: the page as it was sent when the
/// pill still holds it, else the draft as the store has it.
pub(in crate::ui) fn take_back(
    mut desk: Desk,
    mut shell: Signal<Shell>,
    draft: DraftId,
) -> Result<(), String> {
    let store = consume_context::<Arc<SqliteStore>>();
    let draft = life::unsend(&store, draft, Utc::now())?;
    let held = desk
        .outbox
        .peek()
        .clone()
        .filter(|out| out.draft == draft.id);
    let mut page = match held {
        Some(out) => {
            desk.outbox.set(None);
            out.page
        }
        None => life::load(&store, draft.id, &mut desk.parked.write())
            .ok_or_else(|| "that draft is gone".to_owned())?,
    };
    page.phase = Phase::Writing;
    // Still folding on screen: it unfolds where it is.
    let current = *desk.current.peek();
    if let Some(mut showing) = current
        && showing.peek().draft == page.draft
    {
        showing.set(page);
        return Ok(());
    }
    desk.parked.write().retain(|kept| kept.draft != page.draft);
    desk.parked.write().push(page);
    shell.write().compose(&draft);
    Ok(())
}

/// Open a parked draft again.
pub(in crate::ui) fn reopen(desk: Desk, mut shell: Signal<Shell>, draft: DraftId) {
    let store = consume_context::<Arc<SqliteStore>>();
    match store.draft(draft) {
        Ok(stored) => shell.write().compose(&stored),
        Err(_) => unpark(desk, draft),
    }
}

/// The pencil entries in Today: drafts put aside in this Space.
#[component]
pub(in crate::ui) fn ParkedDrafts(shell: Signal<Shell>, space_index: usize) -> Element {
    let Some(desk) = try_use_context::<Desk>() else {
        return rsx! {};
    };
    let entries: Vec<(DraftId, String)> = desk
        .today
        .read()
        .parked(space_index)
        .into_iter()
        .map(|parked| (parked.draft, parked.title.clone()))
        .collect();
    rsx! {
        for (draft, title) in entries {
            div {
                key: "{draft}",
                class: "item today-item draft",
                role: "button",
                tabindex: "0",
                title: "A draft you put aside",
                onclick: move |_| reopen(desk, shell, draft),
                span { class: "fav draft", Glyph { icon: Icon::Pen } }
                span { class: "t", "{title}" }
            }
        }
    }
}
