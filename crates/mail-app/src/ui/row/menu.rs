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

use super::act::{Pressed, op_icon};
use crate::ui::copy::menus;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::components::menus::item::item::{MenuImage, MenuItem};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::view::says;
use mail_domain::{OpKind, ReadState, Star, ThreadSummary};

/// A row of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Pick {
    /// Open in new window.
    Window,
    /// One of the row's actions, as the old strip pressed it.
    Press(Pressed),
    /// Remind me if no reply…: the follow-up menu, where this menu stood.
    Remind,
    /// Download: the conversation's messages as the server sent them, an `.eml` each, into the
    /// downloads directory. The reader's ⋯ only.
    Download,
    /// Print…: its ways to print, a further menu. The reader's ⋯ only.
    Print,
    /// View: the peek modes and a window of its own, a further menu. The reader's ⋯ only, and
    /// only in the pane.
    View,
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
/// put it off or file it away, what takes it out of the list, and last what copies it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Open,
    Respond,
    Mark,
    Organise,
    Remove,
    Keep,
}

/// Where `pick` sits: its group, then its place inside it.
fn slot(pick: Pick) -> (Group, u8) {
    let kind = match pick {
        Pick::Window => return (Group::Open, 0),
        Pick::Press(Pressed::MoveTo) => return (Group::Organise, 3),
        Pick::Remind => return (Group::Organise, 1),
        Pick::View => return (Group::Open, 1),
        Pick::Print => return (Group::Keep, 0),
        Pick::Download => return (Group::Keep, 1),
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

/// The menu for `summary`, whose row offers `offered` (the view's actions or
/// [`mail_core::view::hover_actions`], and Delete forever in a bin), grouped.
///
/// Mark read or unread, star or unstar, mute, remind and move are always there, whatever the
/// view's strip named, each on the side the conversation needs; a view only narrowed what fitted
/// on a strip.
pub(super) fn groups(offered: &[OpKind], summary: &ThreadSummary) -> Vec<Vec<Pick>> {
    let read = match summary.read {
        ReadState::Unread => OpKind::MarkRead,
        ReadState::Read => OpKind::MarkUnread,
    };
    let star = match summary.star {
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
/// already holds (`toolbar`, its Reply, Archive and Delete), without Remind (Send ▾'s alone) and
/// the window (the view's), with the replies and the quieter actions a conversation always has,
/// View where the reader is a pane (`view`), and Print and Download last.
pub(in crate::ui) fn reader_groups(
    offered: &[OpKind],
    summary: &ThreadSummary,
    toolbar: &[OpKind],
    view: bool,
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
    let mut picks: Vec<Vec<Pick>> = groups(&offered, summary);
    if let (true, Some(open)) = (view, picks.first_mut()) {
        open.push(Pick::View);
    }
    picks
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .filter(|pick| match pick {
                    Pick::Window | Pick::Remind => false,
                    Pick::Press(Pressed::Op(OpKind::FollowUp)) => false,
                    Pick::Press(Pressed::Op(kind)) => !toolbar.contains(kind),
                    _ => true,
                })
                .collect::<Vec<_>>()
        })
        .filter(|group| !group.is_empty())
        .chain([vec![Pick::Print, Pick::Download]])
        .collect()
}

/// What a row is called: the side of its action the conversation is offered
/// ([`mail_core::view::says`]), so an archived one says Move to Inbox and a pinned one Unpin. One
/// that opens a further menu or a sheet ends in "…", as the Mac's do.
pub(super) fn name(pick: Pick, summary: &ThreadSummary, now: DateTime<Utc>) -> &'static str {
    match pick {
        Pick::Window => crate::ui::window::OPEN_NAME,
        Pick::Remind => menus::offered(says(OpKind::FollowUp, summary, now)),
        Pick::Download => menus::DOWNLOAD,
        Pick::Print => menus::PRINT,
        Pick::View => menus::VIEW,
        Pick::Press(Pressed::MoveTo) => menus::MOVE_TO,
        Pick::Press(Pressed::Op(kind)) => menus::offered(says(kind, summary, now)),
    }
}

fn icon(pick: Pick) -> Icon {
    match pick {
        Pick::Window => Icon::Window,
        Pick::Remind => Icon::Bell,
        Pick::Download => Icon::Download,
        Pick::Print => Icon::Printer,
        Pick::View => Icon::Panel,
        Pick::Press(Pressed::MoveTo) => Icon::FolderInput,
        Pick::Press(Pressed::Op(kind)) => op_icon(kind),
    }
}

/// The groups as quire's menu rows, a rule between each group, named for `summary` at `now`.
pub(in crate::ui) fn rows(
    groups: &[Vec<Pick>],
    summary: &ThreadSummary,
    now: DateTime<Utc>,
) -> Vec<MenuItem<Pick>> {
    let mut out = Vec::new();
    for (n, group) in groups.iter().enumerate() {
        if n > 0 {
            out.push(MenuItem::Separator);
        }
        for pick in group {
            out.push(
                MenuItem::new(*pick, name(*pick, summary, now))
                    .with_image(MenuImage::Icon(icon(*pick))),
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
    use crate::ui::copy::menus as m;
    use chrono::TimeZone;
    use mail_core::view::hover_actions;
    use mail_domain::id::new_account_id;
    use mail_domain::*;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    fn summary(tweak: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut s = ThreadSummary {
            id: ThreadId::generate(),
            account: new_account_id(),
            subject: "s".into(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "a@example.test".into(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: now(),
            message_count: 1,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: mail_domain::Pin::Unpinned,
            mute: Mute::Unmuted,
            follow_up: FollowUp::Inactive,
        };
        tweak(&mut s);
        s
    }

    /// The menu's rows by name, a rule as `-`.
    fn names(offered: &[OpKind], summary: &ThreadSummary) -> Vec<String> {
        rows(&groups(offered, summary), summary, now())
            .into_iter()
            .map(|row| match row {
                MenuItem::Item { title, .. } => title,
                _ => "-".to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_menu_lists_every_action_in_the_mac_s_order() {
        let inbox = summary(|_| {});
        let binned = summary(|s| {
            s.read = ReadState::Read;
            s.star = Star::Starred;
            s.mute = Mute::Muted;
            s.mailboxes = MailboxSet::only(MailboxRole::Trash);
        });
        let mut bin_offers = hover_actions(&binned);
        bin_offers.push(OpKind::Destroy);
        let a_view_s_own = [OpKind::Reply, OpKind::Spam, OpKind::FollowUp];
        /// A conversation, what its row offers, then the menu it gets.
        type Case<'a> = (&'a str, &'a ThreadSummary, Vec<OpKind>, Vec<&'a str>);
        let cases: Vec<Case> = vec![
            (
                "an unread inbox conversation",
                &inbox,
                hover_actions(&inbox),
                vec![
                    crate::ui::window::OPEN_NAME,
                    "-",
                    m::FORWARD,
                    "-",
                    m::MARK_AS_READ,
                    m::STAR,
                    m::PIN,
                    m::MUTE,
                    "-",
                    m::SNOOZE,
                    m::REMIND,
                    m::LABEL,
                    m::MOVE_TO,
                    "-",
                    m::ARCHIVE,
                    m::MARK_AS_SPAM,
                    m::MOVE_TO_TRASH,
                ],
            ),
            (
                "a read, starred, muted one in Trash",
                &binned,
                bin_offers,
                vec![
                    crate::ui::window::OPEN_NAME,
                    "-",
                    m::FORWARD,
                    "-",
                    m::MARK_AS_UNREAD,
                    m::UNSTAR,
                    m::PIN,
                    m::UNMUTE,
                    "-",
                    m::SNOOZE,
                    m::REMIND,
                    m::LABEL,
                    m::MOVE_TO,
                    "-",
                    m::MOVE_TO_INBOX,
                    m::MARK_AS_SPAM,
                    m::DELETE_FOREVER,
                ],
            ),
            (
                "a view whose strip names its own, with nothing else",
                &inbox,
                a_view_s_own.to_vec(),
                vec![
                    crate::ui::window::OPEN_NAME,
                    "-",
                    m::REPLY,
                    "-",
                    m::MARK_AS_READ,
                    m::STAR,
                    m::MUTE,
                    "-",
                    m::REMIND,
                    m::MOVE_TO,
                    "-",
                    m::MARK_AS_SPAM,
                ],
            ),
        ];
        for (case, summary, offered, want) in cases {
            assert_eq!(names(&offered, summary), want, "{case}");
        }
    }

    /// The owner's report: an archived conversation's menu offered Archive. It offers the way
    /// back, and every pair by the side the conversation is not on.
    #[test]
    fn an_archived_conversation_is_offered_move_to_inbox_and_not_archive() {
        let archived = summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Archive);
            s.pin = mail_domain::Pin::Rank(1);
            s.snooze = Snooze::Until(Utc.timestamp_opt(1_800_000_000, 0).unwrap());
            s.follow_up = FollowUp::Until {
                at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
                set: now(),
            };
        });
        let listed = names(&hover_actions(&archived), &archived);
        for want in [m::MOVE_TO_INBOX, m::UNPIN, m::UNSNOOZE, m::STOP_REMINDING] {
            assert!(
                listed.iter().any(|row| row == want),
                "no {want} in {listed:?}"
            );
        }
        for not in [m::ARCHIVE, m::PIN, m::SNOOZE, m::REMIND, m::NOT_SPAM] {
            assert!(!listed.iter().any(|row| row == not), "{not} in {listed:?}");
        }
        // In Spam the way back is Not Spam, and Mark as Spam is not offered.
        let spam = summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Spam));
        let listed = names(&hover_actions(&spam), &spam);
        assert!(listed.iter().any(|row| row == m::NOT_SPAM), "{listed:?}");
        assert!(
            !listed.iter().any(|row| row == m::MARK_AS_SPAM),
            "{listed:?}"
        );
    }

    #[test]
    fn the_readers_more_menu_leaves_out_what_its_toolbar_holds() {
        let read = summary(|s| s.read = ReadState::Read);
        let inbox = [OpKind::Archive, OpKind::Trash, OpKind::Spam];
        let toolbar = [OpKind::Reply, OpKind::Archive, OpKind::Trash];
        let names: Vec<String> = rows(&reader_groups(&inbox, &read, &toolbar, true), &read, now())
            .into_iter()
            .map(|row| match row {
                MenuItem::Item { title, .. } => title,
                _ => "-".to_owned(),
            })
            .collect();
        assert_eq!(
            names,
            [
                m::VIEW,
                "-",
                m::REPLY_ALL,
                m::FORWARD,
                "-",
                m::MARK_AS_UNREAD,
                m::STAR,
                m::PIN,
                m::MUTE,
                "-",
                m::SNOOZE,
                m::LABEL,
                m::MOVE_TO,
                "-",
                m::MARK_AS_SPAM,
                "-",
                m::PRINT,
                m::DOWNLOAD,
            ]
        );
    }

    #[test]
    fn print_and_download_are_the_reader_s_last_group_and_not_the_row_s() {
        let inbox = summary(|_| {});
        let reader = reader_groups(&[], &inbox, &[], false);
        assert_eq!(
            reader.last().map(Vec::as_slice),
            Some(&[Pick::Print, Pick::Download][..])
        );
        let row = groups(&[OpKind::Archive], &inbox);
        assert!(
            !row.iter().flatten().any(|pick| *pick == Pick::Download),
            "{row:?}"
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
        let listed = names(&every, &summary(|_| {}));
        let rows: Vec<&String> = listed.iter().filter(|name| *name != "-").collect();
        for row in &rows {
            assert_eq!(
                rows.iter().filter(|other| *other == row).count(),
                1,
                "{row} twice in {listed:?}"
            );
        }
        // Read and star are listed the way the conversation needs them, whatever was offered.
        assert!(rows.iter().any(|row| *row == m::MARK_AS_READ), "{listed:?}");
        assert!(
            !rows.iter().any(|row| *row == m::MARK_AS_UNREAD),
            "{listed:?}"
        );
        assert!(!rows.iter().any(|row| *row == m::UNSTAR), "{listed:?}");
    }
}
