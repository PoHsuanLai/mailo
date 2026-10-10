use super::*;
use crate::ui::view::{Grouping, PageGroup, Source, hover_actions, hover_in, places_with};
use chrono::Utc;
use mail_domain::id::new_account_id;
use mail_domain::*;

fn labels() -> Vec<(String, LabelId)> {
    vec![(
        "Travel".to_owned(),
        LabelId::from_uuid(uuid::Uuid::from_u128(7)),
    )]
}

fn summary(read: ReadState, star: Star, role: MailboxRole) -> ThreadSummary {
    ThreadSummary {
        id: ThreadId::generate(),
        account: new_account_id(),
        subject: "s".to_owned(),
        snippet: String::new(),
        from: Address {
            name: None,
            email: "a@example.test".to_owned(),
        },
        participants: Vec::new(),
        recipients: Vec::new(),
        last_date: Utc::now(),
        message_count: 1,
        read,
        star,
        mailboxes: MailboxSet::only(role),
        labels: Vec::new(),
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
        mute: Mute::Unmuted,
        follow_up: mail_domain::FollowUp::Inactive,
    }
}

fn saved(name: &str, query: &str) -> View {
    let mut draft = ViewDraft::from_search(query);
    draft.name = name.to_owned();
    draft.build(&labels(), &Utc).unwrap()
}

#[test]
fn a_view_with_no_hover_buttons_offers_the_usual_strip() {
    let thread = summary(ReadState::Unread, Star::Unstarred, MailboxRole::Inbox);
    let view = saved("Plain", "is:unread");
    assert_eq!(hover_in(Some(&view), &thread), hover_actions(&thread));
    assert_eq!(hover_in(None, &thread), hover_actions(&thread));
}

#[test]
fn a_views_hover_buttons_follow_the_conversation_they_are_drawn_on() {
    let mut view = saved("Mine", "is:unread");
    view.hover = vec![
        OpKind::MarkRead,
        OpKind::Archive,
        OpKind::Star,
        OpKind::Reply,
    ];
    let unread = summary(ReadState::Unread, Star::Unstarred, MailboxRole::Inbox);
    assert_eq!(
        hover_in(Some(&view), &unread),
        vec![
            OpKind::MarkRead,
            OpKind::Archive,
            OpKind::Star,
            OpKind::Reply
        ]
    );
    // Read, starred and already archived: the pair flip, and Archive is not offered.
    let archived = summary(ReadState::Read, Star::Starred, MailboxRole::Archive);
    assert_eq!(
        hover_in(Some(&view), &archived),
        vec![OpKind::MarkUnread, OpKind::Unstar, OpKind::Reply]
    );
}

#[test]
fn a_saved_view_is_a_place_after_the_folders_and_badged_by_its_filter() {
    let view = saved("Trips", "label:travel");
    let places = places_with(&labels(), &[], std::slice::from_ref(&view));
    let last = places.last().unwrap();
    assert_eq!(last.name, "Trips");
    assert_eq!(saved_of(last), Some(&view));
    assert_eq!(
        crate::ui::view::badge_filter(&last.source),
        Some(Filter::And(vec![
            view.filter.clone(),
            Filter::Read(ReadState::Unread)
        ]))
    );
}

#[test]
fn showing_a_view_lists_by_its_filter_and_order_and_groups_by_its_key() {
    let mut view = saved("Trips", "label:travel");
    view.sort.dir = SortDir::Asc;
    view.group_by = Some(GroupKey::Star);
    let mut shell = Shell::default();
    show_kept(&mut shell, &view);
    let query = shell.query(50);
    assert_eq!(query.filter, view.filter);
    assert_eq!(query.sort.dir, SortDir::Asc);
    assert_eq!(shell.grouping(), Grouping::Saved(GroupKey::Star));
    // The page's own menu, once chosen, wins.
    shell.group = PageGroup::Sender;
    assert_eq!(shell.grouping(), Grouping::Page(PageGroup::Sender));
    shell.group = PageGroup::None;
    // A search replaces the place, and with it the view's grouping and strip.
    shell.search = "lunch".to_owned();
    assert_eq!(shell.saved_view(), None);
    assert_eq!(shell.grouping(), Grouping::Page(PageGroup::None));
}

#[test]
fn keeping_a_view_shows_it_and_keeping_it_again_changes_it_in_place() {
    let mut shell = Shell {
        search: "label:travel".to_owned(),
        ..Shell::default()
    };
    let before = shell.places.len();
    let mut view = saved("Trips", "label:travel");
    show_kept(&mut shell, &view);
    assert_eq!(shell.places.len(), before + 1);
    assert_eq!(shell.selected, before, "the new view is shown");
    assert_eq!(shell.search, "", "the search it was made from is done with");
    view.name = "Journeys".to_owned();
    show_kept(&mut shell, &view);
    assert_eq!(
        shell.places.len(),
        before + 1,
        "an edit is not a second place"
    );
    assert_eq!(shell.places[before].name, "Journeys");
}

#[test]
fn forgetting_the_view_shown_goes_back_to_the_inbox_and_keeps_other_places_where_they_were() {
    let mut shell = Shell::default();
    let first = saved("One", "is:starred");
    let second = saved("Two", "is:unread");
    show_kept(&mut shell, &first);
    show_kept(&mut shell, &second);
    let two_at = shell.selected;
    // Showing the second, forget the first: the second is still what is shown.
    show_forgotten(&mut shell, first.id);
    assert_eq!(shell.places[shell.selected].name, "Two");
    assert_eq!(shell.selected, two_at - 1);
    // Forget the one shown: the inbox.
    show_forgotten(&mut shell, second.id);
    assert_eq!(shell.selected, 0);
    assert!(
        shell
            .places
            .iter()
            .all(|p| !matches!(p.source, Source::Saved(_)))
    );
}

#[test]
fn every_grouping_has_a_key_that_finds_it_again() {
    let choices = group_choices(&labels());
    for (key, _, group) in &choices {
        let found: Vec<_> = choices.iter().filter(|(k, _, _)| k == key).collect();
        assert_eq!(found.len(), 1, "{key} is not unique");
        assert_eq!(group_word(group.as_ref()), *key);
    }
    assert!(
        choices
            .iter()
            .any(|(_, name, _)| name == "Labelled Travel, and not")
    );
}
