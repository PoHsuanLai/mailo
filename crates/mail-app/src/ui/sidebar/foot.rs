//! The sidebar's one menu, under the chevron at the right of the foot.
//!
//! The foot keeps one quiet icon on the left (Downloads), the Space dots between, and this
//! chevron: everything else the sidebar used to spread over its body and foot is a row here.
//! Today's recent conversations, with Clear Today under them, the drafts put aside and the
//! messages waiting for their time, then History, then New Space, Settings and the sidebar's
//! own toggle. What the rows are is data ([`menu_items`]) and pure, so a table test can say what the
//! menu lists; [`MoreMenu`] gathers the data, draws the button and acts on a pick.

use crate::ui::actions::tip_standard;
use crate::ui::appearance::WindowDirs;
use crate::ui::compose::{Desk, cancel_waiting, reopen, waiting};
use crate::ui::menus::when_words;
use crate::ui::space::SpaceId;
use crate::ui::text::sender;
use crate::ui::today::Today;
use crate::ui::view::{Shell, Source};
use dioxus::prelude::*;
use ds::base::press::Press;
use ds::components::app::spaces::Epoch;
use ds::components::content::avatar::{AvatarFace, AvatarShape, AvatarSize, AvatarTone};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::person::PersonSwatch;
use mail_core::{SqliteStore, Store};
use mail_domain::{DraftId, ThreadId};
use std::sync::Arc;

/// The most Today conversations the menu lists: the newest, so the menu stays a menu and the
/// rest are one "Show All History" away.
pub(super) const TODAY_SHOWN: usize = 8;

/// What a row of the menu asks for.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Pick {
    /// Open a Today conversation.
    Thread(ThreadId),
    /// Forget this Space's Today conversations.
    ClearToday,
    /// Open a draft put aside.
    Draft(DraftId),
    /// Take a message waiting for its time back.
    CancelSend(DraftId),
    /// Select the History place.
    History,
    /// Add a Space and name it.
    NewSpace,
    /// Open the settings window.
    Settings,
    /// Hide the sidebar, or show it again.
    ToggleSidebar,
}

/// One Today conversation as the menu draws it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Recent {
    pub thread: ThreadId,
    pub title: String,
    pub face: AvatarFace,
}

/// Everything the menu lists, gathered.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Foot {
    /// This Space's Today conversations, newest first.
    pub today: Vec<Recent>,
    /// The drafts put aside in this Space, newest first, by title.
    pub parked: Vec<(DraftId, String)>,
    /// The messages waiting for their time, soonest first: draft, title and when.
    pub waiting: Vec<(DraftId, String, String)>,
    /// Whether the sidebar is pinned open or hidden, which names the toggle.
    pub sidebar: Shown,
}

/// The menu's rows, a rule between each group that has any: Today, drafts put aside, messages
/// waiting, History, then the window's own.
pub(super) fn menu_items(foot: &Foot) -> Vec<MenuItem<Pick>> {
    let mut groups: Vec<Vec<MenuItem<Pick>>> = Vec::new();
    if !foot.today.is_empty() {
        let mut rows = vec![MenuItem::Header("Today".to_owned())];
        rows.extend(foot.today.iter().take(TODAY_SHOWN).map(|recent| {
            MenuItem::new(Pick::Thread(recent.thread), recent.title.clone())
                .with_image(MenuImage::Avatar(recent.face.clone()))
        }));
        rows.push(
            MenuItem::new(Pick::ClearToday, "Clear Today").with_image(MenuImage::Icon(Icon::X)),
        );
        groups.push(rows);
    }
    if !foot.parked.is_empty() {
        let mut rows = vec![MenuItem::Header("Drafts put aside".to_owned())];
        rows.extend(foot.parked.iter().map(|(draft, title)| {
            MenuItem::new(Pick::Draft(*draft), title.clone()).with_image(MenuImage::Icon(Icon::Pen))
        }));
        groups.push(rows);
    }
    if !foot.waiting.is_empty() {
        let mut rows = vec![MenuItem::Header("Waiting to be sent".to_owned())];
        rows.extend(foot.waiting.iter().map(|(draft, title, when)| {
            MenuItem::new(Pick::CancelSend(*draft), format!("Cancel sending {title}"))
                .with_image(MenuImage::Icon(Icon::Clock))
                .with_hint(when.clone())
        }));
        groups.push(rows);
    }
    groups.push(vec![
        MenuItem::new(Pick::History, "Show All History")
            .with_image(MenuImage::Icon(Icon::RotateLeft)),
    ]);
    let toggle = match foot.sidebar {
        Shown::Visible => "Hide Sidebar",
        Shown::Hidden => "Show Sidebar",
    };
    groups.push(vec![
        MenuItem::new(Pick::NewSpace, "New Space").with_image(MenuImage::Icon(Icon::Plus)),
        MenuItem::new(Pick::Settings, "Settings\u{2026}")
            .with_image(MenuImage::Icon(Icon::Settings))
            .with_key(tip_standard(chordkit::StandardAction::Settings)),
        MenuItem::new(Pick::ToggleSidebar, toggle)
            .with_image(MenuImage::Icon(Icon::PanelLeft))
            .with_key(tip_standard(chordkit::StandardAction::ToggleSidebar)),
    ]);
    let mut out = Vec::new();
    for (n, group) in groups.into_iter().enumerate() {
        if n > 0 {
            out.push(MenuItem::Separator);
        }
        out.extend(group);
    }
    out
}

/// `live` Today conversations, newest first. The kit lists them in the order they were opened,
/// but a file edited by hand need not be, and the menu's cap keeps the newest.
pub(super) fn newest_first(mut live: Vec<(ThreadId, Epoch)>) -> Vec<ThreadId> {
    live.sort_by(|a, b| b.1.cmp(&a.1));
    live.into_iter().map(|(thread, _)| thread).collect()
}

/// The first character of `name`, upper-cased, or `?` for an empty name.
pub(super) fn initial(name: &str) -> char {
    name.chars()
        .next()
        .and_then(|ch| ch.to_uppercase().next())
        .unwrap_or('?')
}

/// A Today conversation's face: one letter on `tone`, the sidebar's rounded square.
fn face(initial: char, tone: AvatarTone) -> AvatarFace {
    AvatarFace {
        initial,
        size: AvatarSize::Size16,
        tone,
        shape: AvatarShape::Square,
    }
}

/// What the menu lists now: `space`'s Today conversations that are still in the store, its
/// parked drafts, and the messages waiting for their time.
fn gather(store: &SqliteStore, today: &Today, space: SpaceId, sidebar: Shown) -> Foot {
    let now = chrono::Utc::now();
    let live = today
        .live(space, crate::ui::today::at(now))
        .into_iter()
        .map(|(entry, _)| (entry.item, entry.last_opened))
        .collect();
    let recent = newest_first(live)
        .into_iter()
        .enumerate()
        .filter_map(|(index, thread)| {
            let loaded = store.thread(thread).ok()?;
            let title = if loaded.summary.subject.trim().is_empty() {
                "(no subject)".to_owned()
            } else {
                loaded.summary.subject.clone()
            };
            Some(Recent {
                thread,
                title,
                face: face(
                    initial(&sender(&loaded.summary)),
                    AvatarTone::Account(PersonSwatch::nth(index).colour()),
                ),
            })
        })
        .collect();
    Foot {
        today: recent,
        parked: today
            .parked_in(space)
            .into_iter()
            .map(|parked| (parked.item, parked.title.clone()))
            .collect(),
        waiting: waiting(store)
            .into_iter()
            .map(|one| {
                (
                    one.draft,
                    one.title,
                    when_words(one.at, now, &chrono::Local),
                )
            })
            .collect(),
        sidebar,
    }
}

fn save(dirs: &Option<WindowDirs>, today: &Today) {
    if let Some(dirs) = dirs {
        let _ = crate::ui::today::save(&dirs.state, today);
    }
}

/// The chevron at the foot's right and the menu it opens. `on_new_space` hears where the button
/// was pressed, for the name field to open at.
#[component]
pub(in crate::ui) fn MoreMenu(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    today: Signal<Today>,
    space: SpaceId,
    dirs: Option<WindowDirs>,
    side_hidden: Signal<bool>,
    on_new_space: EventHandler<Point>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut pressed_at = use_signal(Point::default);
    let mut tool = use_signal(|| None::<MountedRef>);
    let desk = try_use_context::<Desk>();
    let items = if open() {
        // Read so a send scheduled or taken back is in the list the next time it opens.
        if let Some(desk) = desk {
            let _ = desk.outbox.read();
        }
        let store = consume_context::<Arc<SqliteStore>>();
        let sidebar = if side_hidden() {
            Shown::Hidden
        } else {
            Shown::Visible
        };
        menu_items(&gather(&store, &today.read(), space, sidebar))
    } else {
        Vec::new()
    };
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Icon::ChevronDown,
            label: "Sidebar menu",
            title: "More".to_owned(),
            shown: Some(if open() { Shown::Visible } else { Shown::Hidden }),
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    tool.set(Some(MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onclick: move |press: Press| {
                pressed_at.set(press.at);
                open.toggle();
            },
        }
        if open() {
            Menu::<Pick> {
                placement: MenuPlacement::Popup,
                anchor: crate::ui::menu::anchor_at(tool()),
                items,
                common: Common {
                    aria_label: Some("Sidebar menu".to_owned()),
                    ..Common::default()
                },
                onpick: move |pick: Pick| match pick {
                    Pick::Thread(thread) => shell.write().open_from_today(thread),
                    Pick::ClearToday => {
                        today.write().clear(space);
                        save(&dirs, &today.read());
                    }
                    Pick::Draft(draft) => {
                        if let Some(desk) = desk {
                            reopen(desk, shell, draft);
                        }
                    }
                    Pick::CancelSend(draft) => {
                        if let Some(desk) = desk {
                            // A refusal is said in the toast; the message stays as it was.
                            let _ = cancel_waiting(desk, shell, draft);
                        }
                    }
                    Pick::History => {
                        let place = shell
                            .read()
                            .places
                            .iter()
                            .position(|place| place.source == Source::History);
                        if let Some(index) = place {
                            shell.write().select(index);
                            pages.set(1);
                        }
                    }
                    Pick::NewSpace => on_new_space.call(pressed_at()),
                    Pick::Settings => crate::ui::settings_window::open(),
                    Pick::ToggleSidebar => side_hidden.set(!side_hidden()),
                },
                onclose: move |()| open.set(false),
            }
        }
    }
}

#[cfg(test)]
mod tests;
