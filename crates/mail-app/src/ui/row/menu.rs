//! A conversation row's menu: every action the row has, in one list a person asks for.
//!
//! The row draws no action buttons. A button that sits where the pointer rests is pressed by a
//! click that meant to open the conversation, and an archive or a trash nobody asked for is
//! worse than one more click for the one they did. So the actions live here, opened by a right
//! click at the pointer or by the row's single "More actions" button (quire's `RowMore`, the ⋯
//! in the row's tail), and a stray click can only ever open this menu.
//!
//! The rows are data ([`groups`], [`rows`]) and pure, so what the menu lists is a table test;
//! every pick goes back through the row's own `press`, which the old strip used, so a pick on a
//! picked row still acts on the whole selection, takes the keyboard back first (FINDINGS F172),
//! asks before deleting forever, and lands on the undo stack like any other op.

use super::act::{Pressed, op_icon, strip_label};
use dioxus::prelude::*;
use ds::components::menus::item::item::{MenuImage, MenuItem};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::{OpKind, ReadState, Star};

/// A row of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Pick {
    /// Open in new window.
    Window,
    /// One of the row's actions, as the old strip pressed it.
    Press(Pressed),
    /// Remind me if no reply…: the follow-up menu, where this menu stood.
    Remind,
}

/// What asked for the menu, which says where it hangs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Opener {
    /// A right click: a context menu at this point.
    Pointer(Rect),
    /// The ⋯ button: a pop-up under its measured rect.
    More(Rect),
}

impl Opener {
    /// The rect a menu opened from here hangs from: the point, or the measured button.
    pub(super) fn place(self) -> Rect {
        match self {
            Opener::Pointer(at) | Opener::More(at) => at,
        }
    }

    fn placement(self) -> MenuPlacement {
        match self {
            Opener::Pointer(_) => MenuPlacement::Context,
            Opener::More(_) => MenuPlacement::Popup,
        }
    }
}

/// The menu's groups, a rule between each, in the Mac's order: open it, answer it, mark it,
/// put it off or file it away, and last what takes it out of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Open,
    Respond,
    Mark,
    Organise,
    Remove,
}

/// Where `pick` sits: its group, then its place inside it.
fn slot(pick: Pick) -> (Group, u8) {
    let kind = match pick {
        Pick::Window => return (Group::Open, 0),
        Pick::Press(Pressed::MoveTo) => return (Group::Organise, 3),
        Pick::Remind => return (Group::Organise, 1),
        Pick::Press(Pressed::Op(kind)) => kind,
    };
    match kind {
        OpKind::Reply => (Group::Respond, 0),
        OpKind::ReplyAll => (Group::Respond, 1),
        OpKind::Forward => (Group::Respond, 2),
        OpKind::MarkRead | OpKind::MarkUnread => (Group::Mark, 0),
        OpKind::Star | OpKind::Unstar => (Group::Mark, 1),
        OpKind::Pin => (Group::Mark, 2),
        OpKind::Mute => (Group::Mark, 3),
        OpKind::Snooze => (Group::Organise, 0),
        // The remind row stands for it: both mean the follow-up menu.
        OpKind::FollowUp => (Group::Organise, 1),
        OpKind::AddLabel | OpKind::RemoveLabel => (Group::Organise, 2),
        OpKind::Archive => (Group::Remove, 0),
        OpKind::Restore => (Group::Remove, 1),
        OpKind::Spam => (Group::Remove, 2),
        OpKind::Trash => (Group::Remove, 3),
        OpKind::Destroy => (Group::Remove, 4),
    }
}

/// The menu for a conversation in `read` and `star` state whose row offers `offered` (what the
/// strip offered: the view's hover actions, and Delete forever in a bin), grouped.
///
/// Mark read or unread, star or unstar, mute, remind and move are always there, whatever the
/// view's strip named; a view only narrowed what fitted on a strip.
pub(super) fn groups(offered: &[OpKind], read: ReadState, star: Star) -> Vec<Vec<Pick>> {
    let read = match read {
        ReadState::Unread => OpKind::MarkRead,
        ReadState::Read => OpKind::MarkUnread,
    };
    let star = match star {
        Star::Unstarred => OpKind::Star,
        Star::Starred => OpKind::Unstar,
    };
    let mut picks = vec![
        Pick::Window,
        Pick::Remind,
        Pick::Press(Pressed::MoveTo),
        Pick::Press(Pressed::Op(read)),
        Pick::Press(Pressed::Op(star)),
        Pick::Press(Pressed::Op(OpKind::Mute)),
    ];
    for kind in offered {
        let pick = Pick::Press(Pressed::Op(*kind));
        let already = picks.iter().any(|have| slot(*have) == slot(pick));
        if !already {
            picks.push(pick);
        }
    }
    picks.sort_by_key(|pick| slot(*pick));
    let mut out: Vec<Vec<Pick>> = Vec::new();
    for pick in picks {
        match out.last_mut() {
            Some(group) if group.first().map(|first| slot(*first).0) == Some(slot(pick).0) => {
                group.push(pick);
            }
            _ => out.push(vec![pick]),
        }
    }
    out
}

/// The open conversation's ⋯ in the reader: the row's menu without what the reader's toolbar
/// already holds (Move, Mute, Remind, and the window, which the view menu offers), and with the
/// replies and the quieter actions a conversation always has.
pub(in crate::ui) fn reader_groups(
    offered: &[OpKind],
    read: ReadState,
    star: Star,
) -> Vec<Vec<Pick>> {
    let always = [
        OpKind::Reply,
        OpKind::ReplyAll,
        OpKind::Forward,
        OpKind::Pin,
        OpKind::Snooze,
        OpKind::AddLabel,
    ];
    let offered: Vec<OpKind> = always.iter().chain(offered).copied().collect();
    groups(&offered, read, star)
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .filter(|pick| {
                    !matches!(
                        pick,
                        Pick::Window
                            | Pick::Remind
                            | Pick::Press(Pressed::MoveTo)
                            | Pick::Press(Pressed::Op(OpKind::Mute | OpKind::FollowUp))
                    )
                })
                .collect::<Vec<_>>()
        })
        .filter(|group| !group.is_empty())
        .collect()
}

/// What a row is called. One that opens a further menu or a sheet ends in "…", as the Mac's do.
pub(super) fn name(pick: Pick, muted: Muted) -> String {
    let muted = muted == Muted::Yes;
    match pick {
        Pick::Window => crate::ui::window::OPEN_NAME.to_owned(),
        Pick::Remind => "Remind me if no reply…".to_owned(),
        Pick::Press(Pressed::MoveTo) => "Move to…".to_owned(),
        Pick::Press(Pressed::Op(kind)) => match kind {
            OpKind::MarkRead => "Mark as read".to_owned(),
            OpKind::MarkUnread => "Mark as unread".to_owned(),
            OpKind::Snooze => "Snooze…".to_owned(),
            OpKind::AddLabel => "Label…".to_owned(),
            OpKind::Destroy => "Delete forever…".to_owned(),
            other => strip_label(other, muted).to_owned(),
        },
    }
}

/// Whether the conversation is muted, which names its mute row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Muted {
    Yes,
    No,
}

fn icon(pick: Pick) -> Icon {
    match pick {
        Pick::Window => Icon::Window,
        Pick::Remind => Icon::Bell,
        Pick::Press(Pressed::MoveTo) => Icon::FolderInput,
        Pick::Press(Pressed::Op(kind)) => op_icon(kind),
    }
}

/// The groups as quire's menu rows, a rule between each group.
pub(in crate::ui) fn rows(groups: &[Vec<Pick>], muted: Muted) -> Vec<MenuItem<Pick>> {
    let mut out = Vec::new();
    for (n, group) in groups.iter().enumerate() {
        if n > 0 {
            out.push(MenuItem::Separator);
        }
        for pick in group {
            out.push(
                MenuItem::new(*pick, name(*pick, muted)).with_image(MenuImage::Icon(icon(*pick))),
            );
        }
    }
    out
}

/// The row's menu, hung where `opener` says, against the row's box `anchor` until a rect is
/// known.
#[component]
pub(super) fn RowMenu(
    opener: Opener,
    anchor: Option<MountedRef>,
    subject: String,
    items: Vec<MenuItem<Pick>>,
    on_pick: EventHandler<Pick>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        Menu::<Pick> {
            placement: opener.placement(),
            anchor: crate::ui::menu::anchor_for(anchor, Some(opener.place())),
            items,
            common: Common {
                aria_label: Some(format!("Actions for {subject}")),
                ..Common::default()
            },
            onpick: on_pick,
            onclose: on_close,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu's rows by name, a rule as `-`.
    fn names(offered: &[OpKind], read: ReadState, star: Star, muted: Muted) -> Vec<String> {
        rows(&groups(offered, read, star), muted)
            .into_iter()
            .map(|row| match row {
                MenuItem::Item { title, .. } => title,
                _ => "-".to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_menu_lists_every_action_in_the_mac_s_order() {
        let inbox_strip = [
            OpKind::Archive,
            OpKind::Trash,
            OpKind::MarkRead,
            OpKind::Pin,
            OpKind::Mute,
            OpKind::Snooze,
            OpKind::AddLabel,
            OpKind::Forward,
        ];
        let binned = [OpKind::Restore, OpKind::MarkUnread, OpKind::Destroy];
        let a_view_s_own = [OpKind::Reply, OpKind::Spam, OpKind::FollowUp];
        /// A conversation's state, then the menu it gets.
        type Case<'a> = (&'a str, &'a [OpKind], ReadState, Star, Muted, &'a [&'a str]);
        let cases: &[Case] = &[
            (
                "an unread inbox conversation",
                &inbox_strip,
                ReadState::Unread,
                Star::Unstarred,
                Muted::No,
                &[
                    "Open in new window",
                    "-",
                    "Forward",
                    "-",
                    "Mark as read",
                    "Star",
                    "Pin",
                    "Mute",
                    "-",
                    "Snooze…",
                    "Remind me if no reply…",
                    "Label…",
                    "Move to…",
                    "-",
                    "Archive",
                    "Trash",
                ],
            ),
            (
                "a read, starred, muted one in a bin",
                &binned,
                ReadState::Read,
                Star::Starred,
                Muted::Yes,
                &[
                    "Open in new window",
                    "-",
                    "Mark as unread",
                    "Unstar",
                    "Unmute",
                    "-",
                    "Remind me if no reply…",
                    "Move to…",
                    "-",
                    "Restore",
                    "Delete forever…",
                ],
            ),
            (
                "a view whose strip names its own, with nothing else",
                &a_view_s_own,
                ReadState::Unread,
                Star::Unstarred,
                Muted::No,
                &[
                    "Open in new window",
                    "-",
                    "Reply",
                    "-",
                    "Mark as read",
                    "Star",
                    "Mute",
                    "-",
                    "Remind me if no reply…",
                    "Move to…",
                    "-",
                    "Spam",
                ],
            ),
        ];
        for (case, offered, read, star, muted, want) in cases {
            assert_eq!(names(offered, *read, *star, *muted), *want, "{case}");
        }
    }

    #[test]
    fn the_readers_more_menu_leaves_out_what_its_toolbar_holds() {
        let inbox = [OpKind::Archive, OpKind::Trash, OpKind::Spam];
        let names: Vec<String> = rows(
            &reader_groups(&inbox, ReadState::Read, Star::Unstarred),
            Muted::No,
        )
        .into_iter()
        .map(|row| match row {
            MenuItem::Item { title, .. } => title,
            _ => "-".to_owned(),
        })
        .collect();
        assert_eq!(
            names,
            [
                "Reply",
                "Reply all",
                "Forward",
                "-",
                "Mark as unread",
                "Star",
                "Pin",
                "-",
                "Snooze…",
                "Label…",
                "-",
                "Archive",
                "Spam",
                "Trash",
            ]
        );
    }

    #[test]
    fn no_row_is_listed_twice() {
        let every = [
            OpKind::Archive,
            OpKind::Trash,
            OpKind::Restore,
            OpKind::Spam,
            OpKind::MarkRead,
            OpKind::MarkUnread,
            OpKind::Star,
            OpKind::Unstar,
            OpKind::AddLabel,
            OpKind::Snooze,
            OpKind::Pin,
            OpKind::Mute,
            OpKind::FollowUp,
            OpKind::Destroy,
            OpKind::Reply,
            OpKind::ReplyAll,
            OpKind::Forward,
        ];
        let listed = names(&every, ReadState::Unread, Star::Unstarred, Muted::No);
        let rows: Vec<&String> = listed.iter().filter(|name| *name != "-").collect();
        for row in &rows {
            assert_eq!(
                rows.iter().filter(|other| *other == row).count(),
                1,
                "{row} twice in {listed:?}"
            );
        }
        // Read and star are listed the way the conversation needs them, whatever was offered.
        assert!(rows.iter().any(|row| *row == "Mark as read"), "{listed:?}");
        assert!(
            !rows.iter().any(|row| *row == "Mark as unread"),
            "{listed:?}"
        );
        assert!(!rows.iter().any(|row| *row == "Unstar"), "{listed:?}");
    }
}
