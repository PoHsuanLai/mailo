//! The Folders section as data: the tree, which section a mailbox is drawn in, and what the
//! window says. The store and the frame are in `folder_store_tests.rs`.

use super::folder_act::{child_path, refused, renamed_path, told};
use super::folder_parts::actions;
use super::folder_tree::{Kind, Mailboxes, Node, Section, Show, arrange, tree};
use crate::folder::Refusal;
use mail_domain::*;

pub(super) const IMAP: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f1"));
pub(super) const POP: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f2"));

pub(super) fn folder(path: &str, delimiter: Option<char>) -> Folder {
    Folder {
        account: IMAP,
        path: path.to_owned(),
        delimiter,
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    }
}

/// `path (children…)`, so a whole tree fits on one line of a table.
pub(super) fn shape(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|node| {
            let mark = if node.kind == Kind::Implied { "~" } else { "" };
            if node.children.is_empty() {
                format!("{mark}{}", node.name)
            } else {
                format!("{mark}{} ({})", node.name, shape(&node.children))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn folders_nest_by_their_delimiter() {
    // `~` marks a level the server did not list.
    const CASES: &[(&[&str], Option<char>, &str)] = &[
        (
            &["Work", "Work/2026", "Home"],
            Some('/'),
            "Home, Work (2026)",
        ),
        (
            &["收件匣", "專案", "專案/二〇二六"],
            Some('/'),
            "專案 (二〇二六), 收件匣",
        ),
        (
            &["[Gmail]", "[Gmail]/Sent Mail", "[Gmail]/Trash"],
            Some('/'),
            "[Gmail] (Sent Mail, Trash)",
        ),
        (&["Work.2026", "Work"], Some('.'), "Work (2026)"),
        // No hierarchy: a slash is just a character in a name.
        (&["Work/2026", "Work"], None, "Work, Work/2026"),
        // A child whose parent was not listed still nests, under a level named from its path.
        (&["Projects/2026/Q3"], Some('/'), "~Projects (~2026 (Q3))"),
        (&["A/B", "A/C", "D"], Some('/'), "~A (B, C), D"),
    ];
    for (paths, delimiter, want) in CASES {
        let folders: Vec<Folder> = paths.iter().map(|p| folder(p, *delimiter)).collect();
        let got = shape(&tree(&folders, &|_| None));
        assert_eq!(got, *want, "{paths:?} with {delimiter:?}");
    }
}

#[test]
fn an_implied_level_keeps_the_full_path_so_it_can_be_acted_on() {
    let nodes = tree(&[folder("Projects/2026", Some('/'))], &|_| None);
    assert_eq!(nodes[0].path, "Projects");
    assert_eq!(nodes[0].children[0].path, "Projects/2026");
    // Only "New folder inside" makes sense of a name the server never listed.
    let keys: Vec<String> = actions(&nodes[0]).into_iter().map(|i| i.key).collect();
    assert_eq!(keys, ["new"]);
    let keys: Vec<String> = actions(&nodes[0].children[0])
        .into_iter()
        .map(|i| i.key)
        .collect();
    assert_eq!(keys, ["new", "rename", "unfollow", "delete"]);
}

fn gmail(folders: Vec<Folder>, labels: Vec<Label>) -> Vec<super::folder_tree::AccountFolders> {
    vec![super::folder_tree::AccountFolders {
        account: IMAP,
        address: "me@example.test".to_owned(),
        mailboxes: Mailboxes::Many {
            folders,
            labels,
            server_labels: ServerLabels::Supported,
        },
    }]
}

fn label(name: &str, origin: LabelOrigin) -> Label {
    Label {
        id: LabelId::generate(),
        account: IMAP,
        name: name.to_owned(),
        color: None,
        origin,
    }
}

#[test]
fn places_and_unfollowed_folders_are_left_out() {
    let mut sent = folder("[Gmail]/Sent Mail", Some('/'));
    sent.special = Some(SpecialUse::Sent);
    let mut holder = folder("[Gmail]", Some('/'));
    holder.holds = Holds::FoldersOnly;
    let mut old = folder("Old", Some('/'));
    old.subscription = Subscription::Unsubscribed;
    let folders = vec![
        folder("INBOX", Some('/')),
        holder,
        sent,
        folder("Work", Some('/')),
        old,
    ];
    let section = arrange(&gmail(folders.clone(), vec![]), Show::Followed).unwrap();
    // The inbox and Sent are Places, and `[Gmail]` holds nothing once they are gone.
    assert_eq!(shape(&section.trees[0].nodes), "Work");
    assert_eq!(section.hidden, 1);
    let all = arrange(&gmail(folders, vec![]), Show::All).unwrap();
    assert_eq!(shape(&all.trees[0].nodes), "Old, Work");
    assert_eq!(all.hidden, 0);
}

#[test]
fn a_label_that_is_a_mailbox_is_drawn_once_as_a_folder() {
    let work = label("Work", LabelOrigin::Provider);
    let mine = label("Later", LabelOrigin::User);
    let section = arrange(
        &gmail(
            vec![folder("INBOX", Some('/')), folder("Work", Some('/'))],
            vec![work.clone(), mine.clone()],
        ),
        Show::Followed,
    )
    .unwrap();
    assert_eq!(section.labels, [work.id], "only the server's label moves");
    assert!(matches!(
        section.trees[0].nodes[0].kind,
        Kind::Listed { label: Some(id), .. } if id == work.id
    ));

    // Without labels-as-mailboxes a label and a folder that share a name are two things.
    let mut plain = gmail(
        vec![folder("INBOX", Some('/')), folder("Work", Some('/'))],
        vec![work],
    );
    if let Mailboxes::Many { server_labels, .. } = &mut plain[0].mailboxes {
        *server_labels = ServerLabels::LocalOnly;
    }
    let section = arrange(&plain, Show::Followed).unwrap();
    assert!(section.labels.is_empty());
}

#[test]
fn pop3_and_never_listed_accounts_have_no_section() {
    let pop = super::folder_tree::AccountFolders {
        account: POP,
        address: "you@example.test".to_owned(),
        mailboxes: Mailboxes::One,
    };
    assert_eq!(arrange(std::slice::from_ref(&pop), Show::All), None);
    assert_eq!(arrange(&gmail(vec![], vec![]), Show::All), None);
    let mixed = [pop, gmail(vec![folder("INBOX", None)], vec![]).remove(0)];
    let section: Section = arrange(&mixed, Show::All).unwrap();
    assert_eq!(section.trees.len(), 1, "only the IMAP account has a tree");
}

/// A parent, a typed name, the delimiter, and the path it makes or the words of the refusal.
type NameCase = (
    Option<&'static str>,
    &'static str,
    Option<char>,
    Result<&'static str, &'static str>,
);

#[test]
fn a_name_is_one_level() {
    const CASES: &[NameCase] = &[
        (None, "Receipts", Some('/'), Ok("Receipts")),
        (Some("Projects"), " 2026 ", Some('/'), Ok("Projects/2026")),
        (Some("專案"), "收據", Some('/'), Ok("專案/收據")),
        (None, "a/b", Some('/'), Err("cannot contain “/”")),
        (None, "a/b", None, Ok("a/b")),
        (Some("Work"), "x", None, Err("top level")),
        (None, "  ", Some('/'), Err("Type a name")),
    ];
    for (parent, name, delimiter, want) in CASES {
        let got = child_path(*parent, name, *delimiter);
        match (want, &got) {
            (Ok(path), Ok(got)) => assert_eq!(got, path),
            (Err(words), Err(got)) => assert!(got.contains(words), "{got}"),
            _ => panic!("{parent:?} {name:?}: {got:?}"),
        }
    }
    assert_eq!(
        renamed_path("Projects/2026", "2027", Some('/')).as_deref(),
        Ok("Projects/2027")
    );
}

#[test]
fn refusals_are_said_in_words() {
    let special = Refusal::Folder(FolderError::Special {
        path: "[Gmail]/Sent Mail".to_owned(),
        special: SpecialUse::Sent,
    });
    assert!(refused(&special).contains("account's Sent folder"));
    let pop = Refusal::Folder(FolderError::SingleMailbox);
    assert!(refused(&pop).contains("POP3"));
    let exists = Refusal::Folder(FolderError::Exists("Work".to_owned()));
    assert_eq!(refused(&exists), "There is already a folder called “Work”.");
}

#[test]
fn the_toast_names_the_leaf() {
    let work = FolderWork::Subscribe {
        path: "A/B".to_owned(),
        subscription: Subscription::Subscribed,
    };
    assert_eq!(told(&work, Some('/')), "Following “B”");
    assert_eq!(told(&work, None), "Following “A/B”");
}
