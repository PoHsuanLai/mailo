use super::*;
use crate::view::Shell;
use mail_domain::*;

fn summary_in(roles: &[MailboxRole]) -> ThreadSummary {
    ThreadSummary {
        id: ThreadId::generate(),
        account: AccountId::generate(),
        subject: "s".to_owned(),
        snippet: String::new(),
        from: Address {
            name: None,
            email: "a@example.test".to_owned(),
        },
        participants: Vec::new(),
        recipients: Vec::new(),
        last_date: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        message_count: 1,
        read: ReadState::Read,
        star: Star::Unstarred,
        mailboxes: roles.iter().copied().collect(),
        labels: Vec::new(),
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
        mute: Mute::Unmuted,
        follow_up: mail_domain::FollowUp::Inactive,
    }
}

fn shell_at(place: &str, search: &str) -> Shell {
    let mut shell = Shell::default();
    let index = shell
        .places
        .iter()
        .position(|p| p.name == place)
        .unwrap_or_else(|| panic!("no place {place}"));
    shell.select(index);
    shell.search = search.to_owned();
    shell
}

#[test]
fn only_trash_and_spam_are_bins_and_only_without_a_search() {
    // (place, search, the bin it is)
    const CASES: &[(&str, &str, Option<Bin>)] = &[
        ("Trash", "", Some(Bin::Trash)),
        ("Spam", "", Some(Bin::Spam)),
        ("Inbox", "", None),
        ("Archive", "", None),
        ("Sent", "", None),
        ("Starred", "", None),
        // A search's results are not the bin's, even typed from there.
        ("Trash", "from:ada", None),
        ("Trash", "   ", Some(Bin::Trash)),
    ];
    for (place, search, bin) in CASES {
        assert_eq!(
            bin_shown(&shell_at(place, search)),
            *bin,
            "{place} / {search:?}"
        );
    }
}

#[test]
fn a_row_offers_delete_forever_only_with_mail_in_the_bin_shown() {
    // (bin shown, where the conversation's mail is, offered)
    let cases: &[(Option<Bin>, &[MailboxRole], bool)] = &[
        (Some(Bin::Trash), &[MailboxRole::Trash], true),
        (
            Some(Bin::Trash),
            &[MailboxRole::Trash, MailboxRole::Sent],
            true,
        ),
        (Some(Bin::Trash), &[MailboxRole::Spam], false),
        (Some(Bin::Spam), &[MailboxRole::Spam], true),
        (None, &[MailboxRole::Trash], false),
        (Some(Bin::Trash), &[MailboxRole::Inbox], false),
    ];
    for (bin, roles, want) in cases {
        assert_eq!(
            offered(*bin, &summary_in(roles)),
            *want,
            "{bin:?} {roles:?}"
        );
    }
}

fn destroying(reach: Reach, threads: usize, messages: usize) -> Destroying {
    Destroying {
        bin: Bin::Trash,
        reach,
        threads: (0..threads).map(|_| ThreadId::generate()).collect(),
        messages,
    }
}

#[test]
fn the_sheet_names_the_count_and_says_it_cannot_be_undone() {
    // (reach, conversations, messages, title, confirm)
    const CASES: &[(Reach, usize, usize, &str, &str)] = &[
        (
            Reach::Chosen,
            1,
            1,
            "Delete this conversation forever?",
            "Delete forever",
        ),
        (
            Reach::Chosen,
            3,
            5,
            "Delete 3 conversations forever?",
            "Delete forever",
        ),
        (Reach::Everything, 12, 20, "Empty Trash?", "Empty Trash"),
    ];
    for (reach, threads, messages, title, confirm) in CASES {
        let said = words(&destroying(*reach, *threads, *messages));
        assert_eq!(said.title, *title);
        assert_eq!(said.confirm, *confirm);
        let count = if *messages == 1 {
            "1 message in Trash".to_owned()
        } else {
            format!("{messages} messages in Trash")
        };
        assert!(said.body.starts_with(&count), "{}", said.body);
        assert!(said.body.contains("cannot be undone"), "{}", said.body);
    }
}
