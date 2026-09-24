//! The Folders section as data: which mailboxes it draws, nested, and which labels it takes
//! over from the Labels section.
//!
//! Pure. What the store holds is gathered by `folder_act::load`; everything decided about it is
//! decided here, where a table can check it.

use crate::view::Shell;
use mail_domain::folder::delimiter_of;
use mail_domain::{
    AccountId, Folder, Holds, Label, LabelId, LabelOrigin, MailboxRef, ServerLabels, Subscription,
};

/// Whether folders the user does not follow are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Show {
    Followed,
    All,
}

/// One level of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Node {
    /// The full path, as the server names it.
    pub path: String,
    /// The last level of it, which is what the row says.
    pub name: String,
    pub kind: Kind,
    pub children: Vec<Node>,
}

/// What a node stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Kind {
    /// A mailbox the server listed.
    Listed {
        subscription: Subscription,
        holds: Holds,
        /// The server's label of the same name, through which its mail is listed and counted.
        label: Option<LabelId>,
    },
    /// A level the server did not list, known only from the paths beneath it.
    Implied,
}

/// One account's folders, as the store holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct AccountFolders {
    pub account: AccountId,
    pub address: String,
    pub mailboxes: Mailboxes,
}

/// Whether an account has folders at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Mailboxes {
    /// POP3: one mailbox, and nothing to name.
    One,
    Many {
        /// The last listing. Empty until the first sync has asked.
        folders: Vec<Folder>,
        labels: Vec<Label>,
        /// Whether this server presents its labels as mailboxes, as Gmail does.
        server_labels: ServerLabels,
    },
}

/// The section, when there is one to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Section {
    pub trees: Vec<Tree>,
    /// Folders left out because nobody follows them, which "Show all" brings in.
    pub hidden: usize,
    /// The labels that are drawn here as folders, so the Labels section leaves them out.
    pub labels: Vec<LabelId>,
}

/// One account's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Tree {
    pub account: AccountId,
    pub address: String,
    /// The separator a new name is read with. `None` for a server with no hierarchy.
    pub delimiter: Option<char>,
    pub nodes: Vec<Node>,
}

/// The accounts the section is for: the pressed tile, or the Space's, where empty means all.
pub(in crate::ui) fn scope(shell: &Shell) -> Vec<AccountId> {
    match shell.account {
        Some(id) => vec![id],
        None => shell.scope.clone(),
    }
}

/// Lay out the section, or `None` when nothing in scope has folders to show.
///
/// **Which section a mailbox belongs in.** Special-use mailboxes and the inbox are Places, and
/// are never drawn here. Where the server presents its labels as mailboxes (Gmail), a mailbox
/// and the server's label of the same name are one thing seen twice; it is drawn once, here, as
/// a folder, because the folder side is the one that can be made, renamed, deleted and
/// followed, and its mail is listed and counted through the label. Labels no mailbox backs —
/// the ones made in this client, and every label on a server without labels-as-mailboxes —
/// stay in Labels.
///
/// POP3 has one mailbox, so a scope of POP3 accounts has no section. Neither does an IMAP
/// account whose folders have never been listed: a listing always has at least the inbox, so
/// an empty one means a sync has not asked yet, not that there is nothing.
pub(in crate::ui) fn arrange(accounts: &[AccountFolders], show: Show) -> Option<Section> {
    let mut trees = Vec::new();
    let mut hidden = 0;
    let mut taken = Vec::new();
    for one in accounts {
        let Mailboxes::Many {
            folders,
            labels,
            server_labels,
        } = &one.mailboxes
        else {
            continue;
        };
        if folders.is_empty() {
            continue;
        }
        let label_of = |folder: &Folder| label_for(folder, labels, *server_labels);
        let own: Vec<&Folder> = folders.iter().filter(|f| f.protected().is_none()).collect();
        taken.extend(own.iter().filter_map(|f| label_of(f)));
        let shown: Vec<Folder> = own
            .iter()
            .filter(|f| show == Show::All || f.subscription == Subscription::Subscribed)
            .map(|f| (*f).clone())
            .collect();
        hidden += own.len() - shown.len();
        trees.push(Tree {
            account: one.account,
            address: one.address.clone(),
            delimiter: delimiter_of(folders),
            nodes: prune(tree(&shown, &label_of)),
        });
    }
    (!trees.is_empty()).then_some(Section {
        trees,
        hidden,
        labels: taken,
    })
}

/// The folders that are places of their own, each with the name its place goes by.
///
/// A folder the section draws, that holds mail, on a server whose folders are not labels: a
/// Gmail folder is listed through its label's place, and a level that only holds other levels
/// has nothing to list. Followed or not, since "Show all" draws the others and they open too.
/// Named by the last level, as the row is; ordered by account, then path.
pub(in crate::ui) fn placed(accounts: &[AccountFolders]) -> Vec<(String, MailboxRef)> {
    accounts
        .iter()
        .flat_map(|one| match &one.mailboxes {
            Mailboxes::Many {
                folders,
                server_labels,
                ..
            } if *server_labels != ServerLabels::Supported => {
                let mut own: Vec<&Folder> = folders
                    .iter()
                    .filter(|f| f.protected().is_none() && f.holds == Holds::Mail)
                    .collect();
                own.sort_by(|a, b| a.path.cmp(&b.path));
                own.into_iter()
                    .map(|f| {
                        (
                            leaf_of(f),
                            MailboxRef {
                                account: one.account,
                                path: f.path.clone(),
                            },
                        )
                    })
                    .collect()
            }
            _ => Vec::new(),
        })
        .collect()
}

fn leaf_of(folder: &Folder) -> String {
    super::folder_act::leaf(&folder.path, folder.delimiter).to_owned()
}

/// The server's label that is this mailbox, where labels are mailboxes.
fn label_for(folder: &Folder, labels: &[Label], server_labels: ServerLabels) -> Option<LabelId> {
    if server_labels != ServerLabels::Supported {
        return None;
    }
    labels
        .iter()
        .find(|l| l.origin == LabelOrigin::Provider && l.name == folder.path)
        .map(|l| l.id)
}

/// Nest `folders` by their delimiters.
///
/// A path whose parent was not listed still nests, under an [`Kind::Implied`] level named
/// from the path, so `Projects/2026` without `Projects` is not drawn as a name with a slash in
/// it. A server with no hierarchy (`delimiter: None`) has only top-level names, whatever
/// characters they contain. Siblings are in the order of their names.
pub(in crate::ui) fn tree(
    folders: &[Folder],
    label_of: &dyn Fn(&Folder) -> Option<LabelId>,
) -> Vec<Node> {
    let mut roots = Vec::new();
    for folder in folders {
        let parts: Vec<&str> = match folder.delimiter {
            Some(d) => folder.path.split(d).collect(),
            None => vec![folder.path.as_str()],
        };
        let kind = Kind::Listed {
            subscription: folder.subscription,
            holds: folder.holds,
            label: label_of(folder),
        };
        place(&mut roots, &parts, folder.delimiter, kind);
    }
    sorted(roots)
}

/// Put `kind` at `parts` under `level`, making implied levels on the way.
fn place(level: &mut Vec<Node>, parts: &[&str], delimiter: Option<char>, kind: Kind) {
    place_at(level, parts, 0, delimiter, kind);
}

fn place_at(level: &mut Vec<Node>, parts: &[&str], depth: usize, d: Option<char>, kind: Kind) {
    let path = join(&parts[..=depth], d);
    let at = match level.iter().position(|n| n.path == path) {
        Some(at) => at,
        None => {
            level.push(Node {
                path,
                name: parts[depth].to_owned(),
                kind: Kind::Implied,
                children: Vec::new(),
            });
            level.len() - 1
        }
    };
    if depth + 1 == parts.len() {
        level[at].kind = kind;
    } else {
        place_at(&mut level[at].children, parts, depth + 1, d, kind);
    }
}

fn join(parts: &[&str], delimiter: Option<char>) -> String {
    match delimiter {
        Some(d) => parts.join(&d.to_string()),
        None => parts.concat(),
    }
}

fn sorted(mut nodes: Vec<Node>) -> Vec<Node> {
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
        .into_iter()
        .map(|node| Node {
            children: sorted(node.children),
            ..node
        })
        .collect()
}

/// Drop levels that hold no mail and have nothing left beneath them.
///
/// Gmail's `[Gmail]` holds only special-use mailboxes, which are Places; once they are left
/// out it is an empty box, and a row for it is a row that does nothing.
pub(in crate::ui) fn prune(nodes: Vec<Node>) -> Vec<Node> {
    nodes
        .into_iter()
        .filter_map(|node| {
            let children = prune(node.children);
            let holds_mail = matches!(
                node.kind,
                Kind::Listed {
                    holds: Holds::Mail,
                    ..
                }
            );
            (holds_mail || !children.is_empty()).then_some(Node { children, ..node })
        })
        .collect()
}
