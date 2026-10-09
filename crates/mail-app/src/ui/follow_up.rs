//! Remind me if no reply, in the window: the sweep that brings a conversation back, the menu that
//! sets a reminder on one, the reader's tool and the line that says where a reminder stands.
//!
//! The deciding is [`mail_core::follow_up`]'s; this is when it runs and how it is shown.

use super::menu::{MenuItem, Right, Tile, anchor_for, palette_groups};
use super::menus::{snooze_help, when_words};
use super::motion::act_all;
use super::picks::with_selection;
use crate::ui::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use ds::style::tokens::control_size::ControlSize;
use mail_core::notify::Notifier;
use mail_domain::{FollowUp, Op, ThreadId};
use mail_store::SqliteStore;
use std::sync::Arc;
use std::time::Duration;

/// Where the window says a reminder came back: the desktop's notifications in the launched
/// window, while notifications are on; a recorder in a test. A window with none says it only in
/// its list.
#[derive(Clone)]
pub struct Notices(pub Arc<dyn Notifier>);

impl std::fmt::Debug for Notices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notices(..)")
    }
}

/// The longest the window sleeps between sweeps. A sleep is measured on a clock that stops while
/// the machine is suspended, so a reminder due during a suspend is caught up at most this long
/// after the machine wakes.
const LONGEST: Duration = Duration::from_secs(300);

/// How long to sleep from `now` until the sweep has something to do at `next`: never less than a
/// second, never more than [`LONGEST`].
///
/// A `next` already gone means the sweep just before this failed (a successful one brings every
/// due reminder back), so it is tried again after the longest wait rather than in a tight loop.
pub(in crate::ui) fn wait(next: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Duration {
    match next.map(|next| (next - now).to_std()) {
        Some(Ok(until)) => until.clamp(Duration::from_secs(1), LONGEST),
        Some(Err(_)) | None => LONGEST,
    }
}

/// Sweep the reminders now and again whenever the next one comes due, and on every revision, so
/// a reply that just arrived or a reminder just set is seen at once. A sweep that changed
/// anything moves the revision, which redraws the list.
///
/// The first sweep is the launch's: a reminder that came due while mailo was closed comes back
/// then. The sleep is quire's, so it runs on the window's clock (a harness's virtual one in a
/// test).
pub(in crate::ui) fn use_reminders(mut revision: Signal<u64>) {
    let _sweeping = use_resource(move || {
        let _ = revision();
        async move {
            loop {
                let now = super::clock::now();
                let store = consume_context::<Arc<SqliteStore>>();
                let notices = try_consume_context::<Notices>();
                let notifier = notices.as_ref().map(|notices| notices.0.as_ref());
                match mail_core::follow_up::sweep_and_announce(&store, notifier, now) {
                    // Moving the revision starts this again, with the next due time.
                    Ok(swept) if swept.changed() => {
                        revision += 1;
                        return;
                    }
                    Ok(_) => {}
                    Err(why) => eprintln!("reminders: {why}"),
                }
                ds::base::time::clock::sleep(wait(mail_core::follow_up::next_due(&store), now))
                    .await;
            }
        }
    });
}

/// The follow-up menu's key for "Don't remind me".
const OFF: &str = "off";
/// And for the time typed into its field.
const TYPED: &str = "typed";

/// The menu's rows: the named times with the date each means, a typed time when it reads as
/// one, and "Don't remind me" when there is a reminder to let go.
pub(in crate::ui) fn follow_up_items<Tz: chrono::TimeZone>(
    current: FollowUp,
    typed: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    let row = |key: String, icon, name: String, help: Option<String>| MenuItem {
        key,
        tile: Tile::Icon(icon),
        name,
        help,
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    };
    let mut items: Vec<MenuItem> = mail_core::follow_up::CHOICES
        .iter()
        .filter_map(|(key, says)| {
            let at = mail_core::follow_up::due(key, now, zone).ok()?;
            Some(row(
                (*key).to_owned(),
                Icon::Bell,
                (*says).to_owned(),
                Some(snooze_help(at, zone)),
            ))
        })
        .collect();
    if !typed.trim().is_empty() {
        let help = match mail_core::follow_up::due(typed, now, zone) {
            Ok(at) => snooze_help(at, zone),
            Err(why) => why,
        };
        items.push(row(
            TYPED.to_owned(),
            Icon::Clock,
            format!("Remind me “{}”", typed.trim()),
            Some(help),
        ));
    }
    if current != FollowUp::Inactive {
        items.push(row(
            OFF.to_owned(),
            Icon::X,
            "Don't remind me".to_owned(),
            None,
        ));
    }
    items
}

/// What a pick in the follow-up menu sets, asked for at `now`: `None` when the typed time does
/// not read as one to come.
pub(in crate::ui) fn picked<Tz: chrono::TimeZone>(
    key: &str,
    typed: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Option<FollowUp>
where
    Tz::Offset: std::fmt::Display,
{
    let choice = match key {
        OFF => return Some(FollowUp::Inactive),
        TYPED => typed,
        named => named,
    };
    let at = mail_core::follow_up::due(choice, now, zone).ok()?;
    Some(FollowUp::Until { at, set: now })
}

/// "Remind me if no reply", for a conversation and whatever is picked with it: one gesture with
/// one undo, anchored to what opened it.
#[component]
pub(in crate::ui) fn FollowUpMenu(
    id: ThreadId,
    current: FollowUp,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    anchor: Option<MountedRef>,
    #[props(default)] placed: Option<Rect>,
    on_close: EventHandler<()>,
) -> Element {
    let mut typed = use_signal(String::new);
    let now = super::clock::now();
    let items = follow_up_items(current, &typed(), now, &chrono::Local);
    rsx! {
        PickList::<String> {
            anchor: anchor_for(anchor, placed),
            label: "Remind me if no reply".to_owned(),
            placeholder: "Or type a time: fri 17:00, +3d".to_owned(),
            query: typed(),
            groups: palette_groups(&items, AvatarSize::Size22, None),
            empty: "No time matches.".to_owned(),
            oninput: move |value: String| typed.set(value),
            onpick: move |key: String| {
                let now = super::clock::now();
                let Some(wanted) = picked(&key, &typed(), now, &chrono::Local) else {
                    return;
                };
                let store = consume_context::<Arc<SqliteStore>>();
                let ops = with_selection(shell, id)
                    .into_iter()
                    .map(|thread| (thread, Op::SetFollowUp(wanted)))
                    .collect();
                act_all(&store, shell, revision, ops);
                on_close.call(());
            },
            onclose: move |()| on_close.call(()),
        }
    }
}

/// The reader's bell: pressed while the conversation has a reminder, and a press opens the menu.
#[component]
pub(in crate::ui) fn FollowUpTool(
    thread: ThreadId,
    current: FollowUp,
    shell: Signal<Shell>,
    revision: Signal<u64>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut tool = use_signal(|| None::<MountedRef>);
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            icon: Icon::Bell,
            label: "Remind me if no reply".to_owned(),
            title: Some("Reminder".to_owned()),
            value: Some(if current == FollowUp::Inactive { Check::Off } else { Check::On }),
            shown: Some(if open() { Shown::Visible } else { Shown::Hidden }),
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    tool.set(Some(MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onclick: move |_| open.toggle(),
        }
        if open() {
            FollowUpMenu {
                id: thread,
                current,
                shell,
                revision,
                anchor: tool(),
                on_close: move |_| open.set(false),
            }
        }
    }
}

/// The reader's line about a reminder, when there is one.
pub(in crate::ui) fn note_words(follow_up: FollowUp, now: DateTime<Utc>) -> Option<String> {
    match follow_up {
        FollowUp::Inactive => None,
        FollowUp::Until { at, .. } => Some(format!(
            "Reminder {} if nobody replies",
            when_words(at, now, &chrono::Local)
        )),
        FollowUp::Returned { set, .. } => Some(format!(
            "No reply yet — nobody has written since {}",
            when_words(set, now, &chrono::Local)
        )),
    }
}

/// That line, drawn.
#[component]
pub(in crate::ui) fn FollowUpNote(follow_up: FollowUp) -> Element {
    let Some(words) = note_words(follow_up, super::clock::now()) else {
        return rsx! {};
    };
    rsx! {
        div { class: "follow-up-note", role: "status",
            Glyph { icon: Icon::Bell, size: IconSize::Compact }
            span { "{words}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 28, hour, 0, 0).unwrap()
    }

    #[test]
    fn the_sweep_sleeps_until_the_next_reminder_within_bounds() {
        let now = at(12);
        let cases = [
            ("nothing waiting", None, LONGEST),
            ("an hour away", Some(at(13)), LONGEST),
            (
                "a minute away",
                Some(now + chrono::TimeDelta::minutes(1)),
                Duration::from_secs(60),
            ),
            (
                "half a second away",
                Some(now + chrono::TimeDelta::milliseconds(500)),
                Duration::from_secs(1),
            ),
            (
                "already due, after a sweep that failed",
                Some(at(11)),
                LONGEST,
            ),
        ];
        for (name, next, want) in cases {
            assert_eq!(wait(next, now), want, "{name}");
        }
    }

    #[test]
    fn a_pick_sets_a_reminder_from_now_or_lets_it_go() {
        let now = at(12);
        assert_eq!(
            picked("tomorrow", "", now, &Utc),
            Some(FollowUp::Until {
                at: Utc.with_ymd_and_hms(2026, 9, 29, 9, 0, 0).unwrap(),
                set: now
            })
        );
        assert_eq!(
            picked(TYPED, "+2h", now, &Utc),
            Some(FollowUp::Until {
                at: at(14),
                set: now
            })
        );
        assert_eq!(picked(TYPED, "yesterday-ish", now, &Utc), None);
        assert_eq!(picked(OFF, "", now, &Utc), Some(FollowUp::Inactive));
        // "Don't remind me" is offered only when there is something to let go.
        let keys = |current| -> Vec<String> {
            follow_up_items(current, "", now, &Utc)
                .into_iter()
                .map(|item| item.key)
                .collect()
        };
        assert_eq!(
            keys(FollowUp::Inactive),
            ["tomorrow", "three-days", "next-week"]
        );
        assert_eq!(
            keys(FollowUp::Until {
                at: at(14),
                set: now
            }),
            ["tomorrow", "three-days", "next-week", OFF]
        );
    }
}
