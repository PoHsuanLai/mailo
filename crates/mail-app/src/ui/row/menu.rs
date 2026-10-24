//! A row's menu: every action the row has, in one list a person asks for.
//!
//! The row draws no action buttons. A button that sits where the pointer rests is pressed by a
//! click that meant to open the conversation, and an archive or a trash nobody asked for is
//! worse than one more click for the one they did. So the actions live here, opened by a right
//! click at the pointer, and a click on the row only ever opens it.
//!
//! A conversation's menu is fixed: about ten rows, built from the conversation's own state and
//! nothing else, so it is the same in every place and every search that lists the conversation.
//! The quieter actions (Mute, Mark as Spam, a window of its own, Print, Download) are the reader's
//! ⋯ alone, and Remind is Send ▾'s. A draft's holds Delete Draft, or for one held to leave at a
//! time, Cancel Sending ([`draft_rows`]).
//!
//! The rows are data ([`groups`], [`rows`]) and pure, so what the menu lists is a table test;
//! every pick goes back through the row's own `press`, so a pick on a picked row still acts on
//! the whole selection, takes the keyboard back first, asks before deleting
//! forever, and lands on the undo stack like any other op.

use super::act::{Pressed, op_icon};
use crate::ui::copy::menus;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::components::menus::item::item::{MenuImage, MenuItem};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::view::{offers, says};
use mail_domain::{MailboxRole, OpKind, ReadState, Star, ThreadSummary};

/// A row of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Pick {
    /// One of the row's actions, as the old strip pressed it.
    Press(Pressed),
    /// Download: the conversation's messages as the server sent them, an `.eml` each, into the
    /// downloads directory. The reader's ⋯ only.
    Download,
    /// Print…: its ways to print, a further menu. The reader's ⋯ only.
    Print,
    /// View: the peek modes and a window of its own, a further menu. The reader's ⋯ only, and
    /// only in the pane.
    View,
    /// Unsubscribe…: the popover that says what leaving the list will do. The reader's ⋯ only,
    /// and only while the conversation offers a way out.
    Unsubscribe,
    /// Block Sender: a rule that files the sender's mail in Spam, and this conversation there
    /// now. The reader's ⋯ only, beside Mark as Spam.
    BlockSender,
    /// Delete Draft, with an undo. A draft's row only.
    DeleteDraft,
    /// Cancel Sending: a message held for its time goes back to being a draft. Its row only.
    CancelSending,
    /// Reply to List: an answer at the list's own address. The reader's ⋯ only, and only while
    /// the newest message names one (`List-Post`).
    ReplyList,
    /// Redirect…: the newest message, unchanged, to someone else, asked for in a field. The
    /// reader's ⋯ only.
    Redirect,
    /// Edit as New (Send Again, for a conversation only in Sent): a new message filled in from
    /// the newest. The reader's ⋯ only.
    Again,
}

fn op(kind: OpKind) -> Pick {
    Pick::Press(Pressed::Op(kind))
}

/// The menu for `summary`, a rule between each group, in the Mac's order: answer it; mark it;
/// put it off or file it away; take it out of the list.
///
/// Each pair is on the side the conversation needs: Mark as Read or Unread, Star or Unstar, Pin
/// or Unpin. Archive is Move to Inbox (Not Spam in Spam) once the conversation is out of the
/// inbox, and nothing in a place mail is only filed in, as Sent. Move to Trash is Delete Forever…
/// for a conversation already in Trash, which asks first.
pub(in crate::ui) fn groups(summary: &ThreadSummary) -> Vec<Vec<Pick>> {
    let read = match summary.read {
        ReadState::Unread => OpKind::MarkRead,
        ReadState::Read => OpKind::MarkUnread,
    };
    let star = match summary.star {
        Star::Unstarred => OpKind::Star,
        Star::Starred => OpKind::Unstar,
    };
    let file = [OpKind::Archive, OpKind::Restore]
        .into_iter()
        .find(|kind| offers(summary, *kind));
    let bin = if offers(summary, OpKind::Trash) {
        OpKind::Trash
    } else {
        OpKind::Destroy
    };
    vec![
        vec![op(OpKind::Reply), op(OpKind::ReplyAll), op(OpKind::Forward)],
        vec![op(read), op(star), op(OpKind::Pin)],
        vec![
            op(OpKind::Snooze),
            op(OpKind::AddLabel),
            Pick::Press(Pressed::MoveTo),
        ],
        file.into_iter().chain([bin]).map(op).collect(),
    ]
}

/// The open conversation's ⋯ in the reader: the row's menu without what the reader's toolbar
/// already holds (`toolbar`, its Reply, Archive and Delete), with the quieter actions that are
/// the reader's alone (Reply to List after Reply All where the list names its address (`list`),
/// Redirect… and Edit as New after Forward, Mute beside Pin, Mark as Spam and Block Sender where
/// the conversation can be), View first where the reader is a pane (`view`), Unsubscribe… with
/// what takes the conversation away while its list offers a way out (`leave`), and Print and
/// Download last.
pub(in crate::ui) fn reader_groups(
    summary: &ThreadSummary,
    toolbar: &[OpKind],
    view: bool,
    (leave, list): (bool, bool),
) -> Vec<Vec<Pick>> {
    let mut picks = groups(summary);
    if list {
        picks[0].insert(2, Pick::ReplyList);
    }
    picks[0].extend([Pick::Redirect, Pick::Again]);
    picks[1].push(op(OpKind::Mute));
    if offers(summary, OpKind::Spam) {
        picks[3].push(op(OpKind::Spam));
        picks[3].push(Pick::BlockSender);
    }
    if leave {
        picks[3].push(Pick::Unsubscribe);
    }
    let open = view.then(|| vec![Pick::View]);
    open.into_iter()
        .chain(picks.into_iter().map(|group| {
            group
                .into_iter()
                .filter(|pick| match pick {
                    Pick::Press(Pressed::Op(kind)) => !toolbar.contains(kind),
                    _ => true,
                })
                .collect::<Vec<_>>()
        }))
        .filter(|group| !group.is_empty())
        .chain([vec![Pick::Print, Pick::Download]])
        .collect()
}

/// What a row is called: the side of its action the conversation is offered
/// ([`mail_core::view::says`]), so an archived one says Move to Inbox and a pinned one Unpin. One
/// that opens a further menu or a sheet ends in "…", as the Mac's do.
pub(super) fn name(pick: Pick, summary: &ThreadSummary, now: DateTime<Utc>) -> &'static str {
    match pick {
        Pick::Download => menus::DOWNLOAD,
        Pick::Print => menus::PRINT,
        Pick::View => menus::VIEW,
        Pick::Unsubscribe => menus::UNSUBSCRIBE,
        Pick::BlockSender => menus::BLOCK_SENDER,
        Pick::DeleteDraft => menus::DELETE_DRAFT,
        Pick::CancelSending => menus::CANCEL_SENDING,
        Pick::ReplyList => menus::REPLY_TO_LIST,
        Pick::Redirect => menus::REDIRECT,
        Pick::Again => again(summary),
        Pick::Press(Pressed::MoveTo) => menus::MOVE_TO,
        Pick::Press(Pressed::Op(kind)) => menus::offered(says(kind, summary, now)),
    }
}

/// Edit as New, or Send Again for a conversation that is only in Sent: there the new message is
/// the old one going out again.
pub(in crate::ui) fn again(summary: &ThreadSummary) -> &'static str {
    if summary.mailboxes == mail_domain::MailboxSet::only(MailboxRole::Sent) {
        menus::SEND_AGAIN
    } else {
        menus::EDIT_AS_NEW
    }
}

fn icon(pick: Pick) -> Icon {
    match pick {
        Pick::ReplyList => Icon::Group,
        Pick::Redirect => Icon::ArrowUpRight,
        Pick::Again => Icon::FilePen,
        Pick::Download => Icon::Download,
        Pick::Print => Icon::Printer,
        Pick::View => Icon::Panel,
        Pick::Unsubscribe => Icon::LogOut,
        Pick::BlockSender => Icon::Hand,
        Pick::DeleteDraft => Icon::Trash,
        Pick::CancelSending => Icon::X,
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

/// A draft's menu: Cancel Sending while it is `held` for a time, Delete Draft otherwise.
pub(super) fn draft_rows(held: bool) -> Vec<MenuItem<Pick>> {
    let (pick, name) = if held {
        (Pick::CancelSending, menus::CANCEL_SENDING)
    } else {
        (Pick::DeleteDraft, menus::DELETE_DRAFT)
    };
    vec![MenuItem::new(pick, name).with_image(MenuImage::Icon(icon(pick)))]
}

/// The row's menu, a context menu at `at`, against the row's box `anchor` until a rect is known.
#[component]
pub(super) fn RowMenu(
    at: Rect,
    anchor: Option<MountedRef>,
    subject: String,
    items: Vec<MenuItem<Pick>>,
    on_pick: EventHandler<Pick>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        Menu::<Pick> {
            placement: MenuPlacement::Context,
            anchor: crate::ui::menu::anchor_for(anchor, Some(at)),
            items,
            common: Common {
                aria_label: Some(crate::ui::copy::aria::actions_for(&subject)),
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
    use crate::ui::view::Shell;
    use chrono::TimeZone;

    use mail_domain::*;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    fn summary(tweak: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut s = ThreadSummary {
            from: Address {
                name: None,
                email: "a@example.test".into(),
            },
            last_date: now(),
            ..mail_domain::testing::summary()
        };
        tweak(&mut s);
        s
    }

    /// Menu rows by name, a rule as `-`.
    fn named(groups: &[Vec<Pick>], summary: &ThreadSummary) -> Vec<String> {
        rows(groups, summary, now())
            .into_iter()
            .map(|row| match row {
                MenuItem::Item { title, .. } => title,
                _ => "-".to_owned(),
            })
            .collect()
    }

    fn names(summary: &ThreadSummary) -> Vec<String> {
        named(&groups(summary), summary)
    }

    #[test]
    fn the_menu_is_ten_rows_in_the_mac_s_order() {
        let inbox = summary(|_| {});
        let binned = summary(|s| {
            s.read = ReadState::Read;
            s.star = Star::Starred;
            s.mute = Mute::Muted;
            s.mailboxes = MailboxSet::only(MailboxRole::Trash);
        });
        let spam = summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Spam));
        let sent = summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Sent));
        /// A conversation, then the menu it gets.
        type Case<'a> = (&'a str, &'a ThreadSummary, Vec<&'a str>);
        let cases: Vec<Case> = vec![
            (
                "an unread inbox conversation",
                &inbox,
                vec![
                    m::REPLY,
                    m::REPLY_ALL,
                    m::FORWARD,
                    "-",
                    m::MARK_AS_READ,
                    m::STAR,
                    m::PIN,
                    "-",
                    m::SNOOZE,
                    m::LABEL,
                    m::MOVE_TO,
                    "-",
                    m::ARCHIVE,
                    m::MOVE_TO_TRASH,
                ],
            ),
            (
                "a read, starred, muted one in Trash",
                &binned,
                vec![
                    m::REPLY,
                    m::REPLY_ALL,
                    m::FORWARD,
                    "-",
                    m::MARK_AS_UNREAD,
                    m::UNSTAR,
                    m::PIN,
                    "-",
                    m::SNOOZE,
                    m::LABEL,
                    m::MOVE_TO,
                    "-",
                    m::MOVE_TO_INBOX,
                    m::DELETE_FOREVER,
                ],
            ),
            (
                "one in Spam",
                &spam,
                vec![
                    m::REPLY,
                    m::REPLY_ALL,
                    m::FORWARD,
                    "-",
                    m::MARK_AS_READ,
                    m::STAR,
                    m::PIN,
                    "-",
                    m::SNOOZE,
                    m::LABEL,
                    m::MOVE_TO,
                    "-",
                    m::NOT_SPAM,
                    m::MOVE_TO_TRASH,
                ],
            ),
            (
                "one only in Sent, which nothing archives",
                &sent,
                vec![
                    m::REPLY,
                    m::REPLY_ALL,
                    m::FORWARD,
                    "-",
                    m::MARK_AS_READ,
                    m::STAR,
                    m::PIN,
                    "-",
                    m::SNOOZE,
                    m::LABEL,
                    m::MOVE_TO,
                    "-",
                    m::MOVE_TO_TRASH,
                ],
            ),
        ];
        for (case, summary, want) in cases {
            assert_eq!(names(summary), want, "{case}");
        }
    }

    /// The menu is built from the conversation alone: the same conversation has the same menu in
    /// every place the sidebar lists, a saved view among them, and under a search.
    #[test]
    fn the_menu_is_the_same_in_every_view() {
        let conversations = [
            summary(|_| {}),
            summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Trash)),
            summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Spam)),
        ];
        let mut shell = Shell::default();
        let places = shell.places.len();
        for conversation in &conversations {
            let mut seen: Vec<Vec<String>> = Vec::new();
            for at in 0..places {
                for search in ["", "lunch"] {
                    shell.select(at);
                    shell.search = search.to_owned();
                    seen.push(names(conversation));
                }
            }
            assert!(seen.windows(2).all(|pair| pair[0] == pair[1]), "{seen:?}");
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
        });
        let listed = names(&archived);
        for want in [m::MOVE_TO_INBOX, m::UNPIN, m::UNSNOOZE] {
            assert!(
                listed.iter().any(|row| row == want),
                "no {want} in {listed:?}"
            );
        }
        for not in [m::ARCHIVE, m::PIN, m::SNOOZE, m::NOT_SPAM] {
            assert!(!listed.iter().any(|row| row == not), "{not} in {listed:?}");
        }
    }

    /// Remind is Send ▾'s, and Mute, Mark as Spam, a window, Print and Download are the reader's.
    #[test]
    fn the_row_menu_leaves_the_quieter_actions_to_the_reader() {
        let reminded = summary(|s| {
            s.follow_up = FollowUp::Until {
                at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
                set: now(),
            };
        });
        for conversation in [summary(|_| {}), reminded] {
            let listed = names(&conversation);
            for not in [
                m::REMIND,
                m::STOP_REMINDING,
                m::MUTE,
                m::MARK_AS_SPAM,
                m::BLOCK_SENDER,
                m::PRINT,
                m::DOWNLOAD,
                m::VIEW,
                crate::ui::window::OPEN_NAME,
            ] {
                assert!(!listed.iter().any(|row| row == not), "{not} in {listed:?}");
            }
        }
    }

    #[test]
    fn the_readers_more_menu_leaves_out_what_its_toolbar_holds() {
        let read = summary(|s| s.read = ReadState::Read);
        let toolbar = [OpKind::Reply, OpKind::Archive, OpKind::Trash];
        assert_eq!(
            named(&reader_groups(&read, &toolbar, true, (false, false)), &read),
            [
                m::VIEW,
                "-",
                m::REPLY_ALL,
                m::FORWARD,
                m::REDIRECT,
                m::EDIT_AS_NEW,
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
                m::BLOCK_SENDER,
                "-",
                m::PRINT,
                m::DOWNLOAD,
            ]
        );
    }

    /// Unsubscribe… is offered only while the list offers a way out, beside what else takes the
    /// conversation away, and never on a row's menu.
    #[test]
    fn the_readers_more_menu_offers_unsubscribe_only_when_there_is_a_way_out() {
        let read = summary(|s| s.read = ReadState::Read);
        let toolbar = [OpKind::Reply, OpKind::Archive, OpKind::Trash];
        let spam = Pick::Press(Pressed::Op(OpKind::Spam));
        let archive = Pick::Press(Pressed::Op(OpKind::Archive));
        let trash = Pick::Press(Pressed::Op(OpKind::Trash));
        // (case, what the toolbar holds, whether the list has a way out, the group Unsubscribe…
        // is in)
        type Case<'a> = (&'a str, &'a [OpKind], bool, Option<Vec<Pick>>);
        let cases: &[Case] = &[
            ("no way out", &toolbar, false, None),
            (
                "beside Mark as Spam",
                &toolbar,
                true,
                Some(vec![spam, Pick::BlockSender, Pick::Unsubscribe]),
            ),
            (
                "after Archive and Move to Trash where the toolbar has neither",
                &[],
                true,
                Some(vec![
                    archive,
                    trash,
                    spam,
                    Pick::BlockSender,
                    Pick::Unsubscribe,
                ]),
            ),
        ];
        for (case, toolbar, leave, want) in cases {
            let groups = reader_groups(&read, toolbar, true, (*leave, false));
            let found = groups
                .iter()
                .find(|group| group.contains(&Pick::Unsubscribe))
                .cloned();
            assert_eq!(found, *want, "{case}: {groups:?}");
            assert_eq!(
                groups.last().map(Vec::as_slice),
                Some(&[Pick::Print, Pick::Download][..]),
                "{case}"
            );
        }
        let row = groups(&read);
        assert!(!row.iter().flatten().any(|pick| *pick == Pick::Unsubscribe));
        // Blocking is the reader's, as Mark as Spam is.
        assert!(!row.iter().flatten().any(|pick| *pick == Pick::BlockSender));
    }

    /// Reply to List follows Reply All only where the list names its address, and Redirect… and
    /// Edit as New follow Forward always; none is on a row's menu. A conversation only in Sent
    /// calls Edit as New Send Again.
    #[test]
    fn the_readers_more_menu_answers_a_list_and_passes_a_message_on() {
        let inbox = summary(|_| {});
        let toolbar = [OpKind::Reply];
        let reply_all = Pick::Press(Pressed::Op(OpKind::ReplyAll));
        let forward = Pick::Press(Pressed::Op(OpKind::Forward));
        let first = |list: bool| reader_groups(&inbox, &toolbar, false, (false, list))[0].clone();
        assert_eq!(
            first(true),
            [
                reply_all,
                Pick::ReplyList,
                forward,
                Pick::Redirect,
                Pick::Again
            ]
        );
        assert_eq!(
            first(false),
            [reply_all, forward, Pick::Redirect, Pick::Again]
        );
        let row = groups(&inbox);
        for pick in [Pick::ReplyList, Pick::Redirect, Pick::Again] {
            assert!(
                !row.iter().flatten().any(|p| *p == pick),
                "{pick:?} on a row"
            );
        }
        let sent = summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Sent));
        assert_eq!(name(Pick::Again, &sent, now()), m::SEND_AGAIN);
        assert_eq!(name(Pick::Again, &inbox, now()), m::EDIT_AS_NEW);
    }

    #[test]
    fn print_and_download_are_the_reader_s_last_group_and_not_the_row_s() {
        let inbox = summary(|_| {});
        let reader = reader_groups(&inbox, &[], false, (false, false));
        assert_eq!(
            reader.last().map(Vec::as_slice),
            Some(&[Pick::Print, Pick::Download][..])
        );
        let row = groups(&inbox);
        assert!(
            !row.iter().flatten().any(|pick| *pick == Pick::Download),
            "{row:?}"
        );
    }

    #[test]
    fn no_row_is_listed_twice() {
        for conversation in [
            summary(|_| {}),
            summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Trash)),
        ] {
            let listed = named(
                &reader_groups(&conversation, &[], true, (true, true)),
                &conversation,
            );
            let rows: Vec<&String> = listed.iter().filter(|name| *name != "-").collect();
            for row in &rows {
                assert_eq!(
                    rows.iter().filter(|other| *other == row).count(),
                    1,
                    "{row} twice in {listed:?}"
                );
            }
        }
    }
}
