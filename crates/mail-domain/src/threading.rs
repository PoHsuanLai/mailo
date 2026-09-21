//! JWZ conversation threading (RFC 5256).
//!
//! Written here rather than taken as a dependency: the only crate offering it has a few
//! thousand lifetime downloads and one unknown author, which is the wrong trade for a core
//! correctness algorithm we will want to tune against our own corpus.
//!
//! Runs for every account, including those whose server supplies thread ids
//! ([`crate::ServerThreads::ProviderId`]), so POP3 and IMAP share one set of
//! [`ThreadId`] rules and a provider id is only a hint.
//!
//! # Shape of the implementation
//!
//! Two structures are maintained side by side over one arena of containers:
//!
//! * a **forest** — each container has at most one parent, exactly as JWZ describes. It
//!   carries the shape of the conversation and decides which message is a thread's root.
//! * a **relatedness partition** (union-find) — every id named together in one `References`
//!   header is in one class, whether or not the forest could hang them off each other.
//!
//! The partition, not the forest, is what a thread is. The forest refuses links that would
//! make a cycle and re-parents containers as better evidence arrives; both of those can split
//! a tree in two without meaning that the messages stopped being one conversation. Grouping
//! off the partition also makes the answer independent of the order the inputs arrive in.

use crate::id::ThreadId;
use crate::message::Message;
use std::collections::HashMap;

/// The headers threading needs, without requiring a fully-built [`Message`].
///
/// Ids here are *conventionally* normalized, but nothing depends on the caller having done it:
/// every read below runs [`normalize_id`] defensively, so an un-normalized input threads the
/// same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInput<'a> {
    /// This message's own `Message-ID`, normalized.
    pub message_id: Option<&'a str>,
    pub in_reply_to: Option<&'a str>,
    /// `References`, oldest first, normalized.
    pub references: &'a [String],
    /// Used only by the subject fallback for clients that emit no references at all.
    pub subject: &'a str,
}

impl<'a> ThreadInput<'a> {
    /// Borrow the threading-relevant headers out of a message.
    pub fn of(msg: &'a Message) -> ThreadInput<'a> {
        ThreadInput {
            message_id: msg.rfc_message_id.as_deref(),
            in_reply_to: msg.in_reply_to.as_deref(),
            references: &msg.references,
            subject: &msg.subject,
        }
    }
}

/// Normalize a `Message-ID`: strip angle brackets and surrounding space, ASCII-lowercase.
///
/// Every comparison in this module and every [`crate::MessageKey::Rfc`] goes through it.
///
/// One layer of angle brackets is removed, and only one: `<<a@b>>` keeps an inner `<a@b>`.
///
/// # The unusable id
///
/// An id that is empty, or that still contains a character an `addr-spec` may never hold —
/// ASCII whitespace, a control character, `<` or `>` — normalizes to the **empty string**.
/// The empty string is a sentinel meaning *no usable id*, never a value: [`thread`] never
/// enters it in its id table, never links on it and never hands it to `existing`, so a
/// message carrying one is threaded by its references alone and can never be joined to
/// another message by id. Two messages with unusable ids are therefore not related to each
/// other — which is the point, because "broken the same way" is not evidence of anything.
///
/// Callers that persist ids (for example [`crate::MessageKey::Rfc`]) must treat an empty
/// result as *absent* and fall back to [`crate::MessageKey::Synthetic`].
pub fn normalize_id(raw: &str) -> String {
    let trimmed = raw.trim();
    // `<` and `>` are ASCII, so slicing either end stays on a char boundary.
    let core = match (
        trimmed.starts_with('<'),
        trimmed.ends_with('>'),
        trimmed.len(),
    ) {
        (true, true, len) if len >= 2 => &trimmed[1..len - 1],
        _ => trimmed,
    };
    let core = core.trim();
    let unusable = core.is_empty()
        || core
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>');
    if unusable {
        return String::new();
    }
    core.to_ascii_lowercase()
}

/// Group messages into conversations.
///
/// Returns one [`ThreadId`] per input, positionally. `existing` supplies ids already assigned
/// to messages we have seen before, so that an incremental sync joins new messages onto
/// existing threads instead of renumbering them.
///
/// Subject-based joining is the RFC 5256 fallback and applies only to messages with no usable
/// references — it is a repair for broken clients, not a general rule.
///
/// # Order of operations
///
/// 1. Build the id table, one container per distinct `Message-ID`, and link each message under
///    the last entry of `References` + `In-Reply-To`.
/// 2. Refuse any link that would close a cycle; see [`Forest::link`].
/// 3. Prune: drop childless containers that hold no message, splice the children of the rest
///    up to their grandparent, and promote a root that holds no message and has one child.
/// 4. Gather each relatedness class into a thread.
/// 5. Subject fallback, last and narrowly; see [`subject_merge`].
/// 6. Assign ids: reuse what `existing` reports, otherwise [`ThreadId::generate`].
///
/// # Cost
///
/// Linear in the total number of ids, up to the ancestor walk each link performs. Termination
/// does not depend on the input being well-formed: the forest is acyclic by construction, so
/// every walk is bounded by the depth of a tree, and the walk is additionally capped at the
/// arena size so that a bug here can never become a hang.
pub fn thread<'a>(
    inputs: &[ThreadInput<'a>],
    existing: &dyn Fn(&str) -> Option<ThreadId>,
) -> Vec<ThreadId> {
    let mut forest = Forest::default();

    // 1 & 2. Id table, parent links, cycle refusal.
    let mut home = Vec::with_capacity(inputs.len()); // container holding input i
    let mut rootless = Vec::with_capacity(inputs.len()); // input i named no usable reference
    for (i, input) in inputs.iter().enumerate() {
        let own = input.message_id.map(normalize_id).unwrap_or_default();
        let here = forest.seat(&own, i);
        let chain = forest.chain(input);

        for pair in chain.windows(2) {
            // JWZ: an existing link is evidence we already accepted; do not overwrite it.
            if forest.nodes[pair[1]].parent.is_none() {
                forest.link(pair[0], pair[1]);
            }
        }
        // The message's own header outranks whatever another message's `References` guessed,
        // so this link does overwrite.
        if let Some(&last) = chain.last() {
            forest.link(last, here);
        }
        // Everything named in one `References` header is one conversation even where the
        // forest could not express it.
        for &c in &chain {
            forest.merge(here, c);
        }

        home.push(here);
        rootless.push(chain.is_empty());
    }

    // 3. Prune.
    forest.prune();

    // 4 & 5. Classes, then the subject fallback over their roots.
    subject_merge(&mut forest, inputs, &rootless);

    // 6. Thread ids.
    assign(&mut forest, &home, existing)
}

// ---------------------------------------------------------------------------------------
// The container arena
// ---------------------------------------------------------------------------------------

/// One node of the JWZ forest: a `Message-ID` we have heard of, with or without the message.
#[derive(Debug, Default)]
struct Container {
    /// The normalized id this container stands for. Empty for a *private* container, which is
    /// one that is deliberately unreachable through the id table.
    id: String,
    /// Index into `inputs` of the message seated here, if we have it.
    msg: Option<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
    /// Pruned away: it held no message and carried no structure worth keeping.
    dead: bool,
}

#[derive(Debug, Default)]
struct Forest {
    nodes: Vec<Container>,
    table: HashMap<String, usize>,
    /// Union-find over `nodes`, representative = lowest index in the class.
    class: Vec<usize>,
}

impl Forest {
    fn push(&mut self, id: &str) -> usize {
        let at = self.nodes.len();
        self.nodes.push(Container {
            id: id.to_string(),
            ..Container::default()
        });
        self.class.push(at);
        at
    }

    /// The container for `id`, creating an empty one the first time the id is named.
    fn intern(&mut self, id: &str) -> usize {
        match self.table.get(id) {
            Some(&at) => at,
            None => {
                let at = self.push(id);
                self.table.insert(id.to_string(), at);
                at
            }
        }
    }

    /// Seat input `i` in a container.
    ///
    /// A message with no usable id gets a private container, and so does a message whose id is
    /// already occupied. Two messages sharing one `Message-ID` are *not* forced together:
    /// a mailer that stamps one id on everything it sends is a real failure mode, and
    /// collapsing a mailbox into a single thread is worse than leaving a duplicate apart. The
    /// duplicate still lands in the right thread whenever it carries the same references.
    fn seat(&mut self, id: &str, i: usize) -> usize {
        if !id.is_empty() {
            let at = self.intern(id);
            if self.nodes[at].msg.is_none() {
                self.nodes[at].msg = Some(i);
                return at;
            }
        }
        let at = self.push("");
        self.nodes[at].msg = Some(i);
        at
    }

    /// `References` oldest first, with `In-Reply-To` appended when it is not already there,
    /// normalized, with unusable and repeated ids dropped.
    fn chain(&mut self, input: &ThreadInput<'_>) -> Vec<usize> {
        let mut ids: Vec<String> = Vec::with_capacity(input.references.len() + 1);
        let refs = input.references.iter().map(String::as_str);
        for raw in refs.chain(input.in_reply_to) {
            let id = normalize_id(raw);
            if !id.is_empty() && !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.iter().map(|id| self.intern(id)).collect()
    }

    /// Make `child` a child of `parent`, unless that would close a cycle.
    ///
    /// Mail in the wild contains reference loops, both accidental (a mailing list that rewrites
    /// `Message-ID` and reinjects) and deliberate. A JWZ implementation that trusts the headers
    /// builds a cyclic graph here and then hangs in the very next traversal. We refuse instead:
    /// the forest stays acyclic by construction, so every later walk terminates. Refusing loses
    /// nothing, because `merge` has already recorded that the two are one conversation.
    fn link(&mut self, parent: usize, child: usize) {
        if self.descends_from(parent, child) {
            return;
        }
        if let Some(old) = self.nodes[child].parent {
            if old == parent {
                return;
            }
            self.nodes[old].children.retain(|&c| c != child);
        }
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
        self.merge(parent, child);
    }

    /// Is `ancestor` on the parent chain of `node`, or `node` itself?
    ///
    /// The step cap is belt and braces: the forest is acyclic, so the walk already terminates.
    fn descends_from(&self, node: usize, ancestor: usize) -> bool {
        let mut at = Some(node);
        for _ in 0..=self.nodes.len() {
            match at {
                Some(n) if n == ancestor => return true,
                Some(n) => at = self.nodes[n].parent,
                None => return false,
            }
        }
        true // unreachable while the acyclic invariant holds; refuse rather than loop.
    }

    fn root_of(&mut self, mut at: usize) -> usize {
        while self.class[at] != at {
            self.class[at] = self.class[self.class[at]];
            at = self.class[at];
        }
        at
    }

    /// Record that two containers belong to one conversation.
    fn merge(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.root_of(a), self.root_of(b));
        if ra == rb {
            return;
        }
        // Lowest index wins, so the representative does not depend on the merge order.
        let (keep, drop) = if ra < rb { (ra, rb) } else { (rb, ra) };
        self.class[drop] = keep;
    }

    /// JWZ `prune_empties`, iterative so that a deep reference chain cannot blow the stack.
    ///
    /// A container with no message and no children is dropped. A container with no message but
    /// with children is spliced out, its children rising to its parent — except at the root,
    /// where that is done only for a single child, so that an unseen parent still holds its
    /// siblings together.
    fn prune(&mut self) {
        let roots: Vec<usize> = (0..self.nodes.len())
            .filter(|&n| self.nodes[n].parent.is_none())
            .collect();
        let mut kept: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];

        for root in roots {
            let mut work = vec![(root, false)];
            while let Some((at, done)) = work.pop() {
                if !done {
                    work.push((at, true));
                    work.extend(self.nodes[at].children.iter().map(|&c| (c, false)));
                    continue;
                }
                let mut out: Vec<usize> = Vec::new();
                for child in std::mem::take(&mut self.nodes[at].children) {
                    let below = std::mem::take(&mut kept[child]);
                    if self.nodes[child].msg.is_some() {
                        self.nodes[child].children = below;
                        out.push(child);
                    } else {
                        self.nodes[child].dead = true;
                        for &up in &below {
                            self.nodes[up].parent = Some(at);
                        }
                        out.extend(below);
                    }
                }
                kept[at] = out;
            }

            let below = std::mem::take(&mut kept[root]);
            match (self.nodes[root].msg, below.len()) {
                (Some(_), _) => self.nodes[root].children = below,
                (None, 0) => self.nodes[root].dead = true,
                (None, 1) => {
                    self.nodes[root].dead = true;
                    self.nodes[below[0]].parent = None;
                }
                (None, _) => self.nodes[root].children = below,
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Subject fallback
// ---------------------------------------------------------------------------------------

/// Join an orphaned reply onto the conversation it is plainly a reply to.
///
/// This exists for clients that emit neither `References` nor `In-Reply-To`. It is the last
/// step and the narrowest one, because a wrong join is far more damaging than a missed one:
/// two colleagues who both write "hello" must not end up in one conversation, and once merged
/// the user has no way to take it back.
///
/// A class is an **anchor** when it has exactly one surviving root, that root holds a message,
/// that message names no usable reference, and its subject carries no reply prefix.
///
/// A class is an **orphan** when it holds exactly one message in total, that message names no
/// usable reference, and its subject is a reply prefix over a non-empty base.
///
/// Orphans join the anchor with the same base subject — and only when that base subject has
/// **exactly one** anchor. Two live "Q3 budget" threads make the right answer unknowable, so
/// nothing is merged. Two non-replies never merge with each other, whatever they are called;
/// an identical subject is not evidence that two people started one conversation.
fn subject_merge(forest: &mut Forest, inputs: &[ThreadInput<'_>], rootless: &[bool]) {
    // Per class: its surviving roots, and how many messages it holds.
    let mut roots: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut count: HashMap<usize, usize> = HashMap::new();
    for n in 0..forest.nodes.len() {
        let class = forest.root_of(n);
        if !forest.nodes[n].dead && forest.nodes[n].parent.is_none() {
            roots.entry(class).or_default().push(n);
        }
        if forest.nodes[n].msg.is_some() {
            *count.entry(class).or_default() += 1;
        }
    }

    let mut anchors: HashMap<String, Vec<usize>> = HashMap::new();
    let mut orphans: HashMap<String, Vec<usize>> = HashMap::new();
    for (&class, tops) in &roots {
        let [top] = tops.as_slice() else { continue };
        let Some(i) = forest.nodes[*top].msg else {
            continue;
        };
        if !rootless[i] {
            continue;
        }
        let (base, replied) = base_subject(inputs[i].subject);
        if base.is_empty() {
            continue;
        }
        match replied {
            false => anchors.entry(base).or_default().push(class),
            true if count.get(&class).copied() == Some(1) => {
                orphans.entry(base).or_default().push(class)
            }
            true => {}
        }
    }

    // Collect and sort rather than merging as we go: a `HashMap` iterates in an unspecified
    // order, and the merges must not vary between runs.
    let mut joins: Vec<(usize, usize)> = Vec::new();
    for (base, classes) in &orphans {
        let Some([anchor]) = anchors.get(base).map(Vec::as_slice) else {
            continue;
        };
        joins.extend(classes.iter().map(|&c| (*anchor, c)));
    }
    joins.sort_unstable();
    for (anchor, orphan) in joins {
        forest.merge(anchor, orphan);
    }
}

/// A subject with its reply prefixes stripped, and whether any were there.
///
/// `Re:`, `Fwd:` and `Fw:` are removed repeatedly and case-insensitively, tolerating the
/// `Re[2]:` and `Re(2):` counters some mailers add and space before the colon. The remainder
/// is lowercased and its internal whitespace collapsed, so that a subject re-wrapped in
/// transit still compares equal.
fn base_subject(subject: &str) -> (String, bool) {
    let mut rest = subject.trim();
    let mut replied = false;
    'strip: loop {
        for tag in ["re", "fwd", "fw"] {
            let Some(after) = strip_prefix_ascii_ci(rest, tag) else {
                continue;
            };
            let after = strip_counter(after.trim_start());
            if let Some(tail) = after.strip_prefix(':') {
                rest = tail.trim_start();
                replied = true;
                continue 'strip;
            }
        }
        break;
    }
    let base = rest
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    (base, replied)
}

/// Drop a `[2]` or `(2)` reply counter, if that is exactly what comes next.
fn strip_counter(rest: &str) -> &str {
    let close = match rest.as_bytes().first() {
        Some(b'[') => ']',
        Some(b'(') => ')',
        _ => return rest,
    };
    let digits = &rest[1..];
    let after = digits.trim_start_matches(|c: char| c.is_ascii_digit());
    match after.strip_prefix(close) {
        // At least one digit, or `Re[]:` would parse as a counter.
        Some(tail) if after.len() < digits.len() => tail.trim_start(),
        _ => rest,
    }
}

fn strip_prefix_ascii_ci<'a>(haystack: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = haystack.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

// ---------------------------------------------------------------------------------------
// Id assignment
// ---------------------------------------------------------------------------------------

/// Give every class a [`ThreadId`], preferring one it already owns.
///
/// `existing` is asked about every id in the class, including ids we have only ever seen
/// inside a `References` header. That is what makes an incremental sync work: a new reply
/// whose own id is unknown still lands on its parent's thread, because the parent's id is in
/// the class.
///
/// # When a class already owns two ids
///
/// That is a thread merge: two conversations we had kept apart turn out to be one, typically
/// because the message that references both has only now arrived. The survivor is the
/// **lowest [`ThreadId`]** in the class, ordered as a UUID. The rule is deliberately a
/// property of the set and not of the traversal, so it does not matter which message arrived
/// first, in what order the mailbox was walked, or how many times we re-thread: the same set
/// of conversations always collapses onto the same id. An id that moves renumbers a row the
/// user is looking at, so stability here is a UI property, not a cosmetic one.
fn assign(
    forest: &mut Forest,
    home: &[usize],
    existing: &dyn Fn(&str) -> Option<ThreadId>,
) -> Vec<ThreadId> {
    let mut known: HashMap<usize, ThreadId> = HashMap::new();
    for n in 0..forest.nodes.len() {
        if forest.nodes[n].id.is_empty() {
            continue;
        }
        let Some(tid) = existing(&forest.nodes[n].id) else {
            continue;
        };
        let class = forest.root_of(n);
        known
            .entry(class)
            .and_modify(|held| *held = (*held).min(tid))
            .or_insert(tid);
    }

    let mut out = Vec::with_capacity(home.len());
    let mut fresh: HashMap<usize, ThreadId> = HashMap::new();
    for &at in home {
        let class = forest.root_of(at);
        let tid = match known.get(&class) {
            Some(&tid) => tid,
            None => *fresh.entry(class).or_insert_with(ThreadId::generate),
        };
        out.push(tid);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_strip_prefixes() {
        const CASES: &[(&str, &str, bool)] = &[
            ("Q3 budget", "q3 budget", false),
            ("Re: Q3 budget", "q3 budget", true),
            ("RE: re: FwD: Q3 budget", "q3 budget", true),
            ("Re[2]: Q3 budget", "q3 budget", true),
            ("Re(15): Q3 budget", "q3 budget", true),
            ("Re : Q3 budget", "q3 budget", true),
            ("Fw: Q3   budget", "q3 budget", true),
            ("  Fwd:Q3 budget  ", "q3 budget", true),
            ("Re:", "", true),
            ("", "", false),
            // Not a prefix: no colon, or the word merely starts with `re`.
            ("Review of Q3", "review of q3", false),
            ("Rebuild: the plan", "rebuild: the plan", false),
            ("Re[x]: Q3", "re[x]: q3", false),
        ];
        for &(subject, base, replied) in CASES {
            assert_eq!(
                base_subject(subject),
                (base.to_string(), replied),
                "{subject:?}"
            );
        }
    }

    #[test]
    fn prune_drops_empty_leaves_and_promotes_a_lone_child() {
        // phantom -> real: the phantom root has one child, so the child becomes the root.
        let mut f = Forest::default();
        let ghost = f.intern("ghost@x");
        let seen = f.seat("seen@x", 0);
        f.link(ghost, seen);
        f.prune();
        assert!(f.nodes[ghost].dead);
        assert_eq!(f.nodes[seen].parent, None);

        // A phantom root with two real children stays: it is what holds them together.
        let mut f = Forest::default();
        let ghost = f.intern("ghost@x");
        let a = f.seat("a@x", 0);
        let b = f.seat("b@x", 1);
        f.link(ghost, a);
        f.link(ghost, b);
        f.prune();
        assert!(!f.nodes[ghost].dead);
        assert_eq!(f.nodes[ghost].children, vec![a, b]);

        // A phantom in the middle is spliced out whatever its child count.
        let mut f = Forest::default();
        let top = f.seat("top@x", 0);
        let ghost = f.intern("ghost@x");
        let a = f.seat("a@x", 1);
        let b = f.seat("b@x", 2);
        f.link(top, ghost);
        f.link(ghost, a);
        f.link(ghost, b);
        f.prune();
        assert!(f.nodes[ghost].dead);
        assert_eq!(f.nodes[top].children, vec![a, b]);
        assert_eq!(f.nodes[a].parent, Some(top));
    }

    #[test]
    fn a_link_that_would_close_a_cycle_is_refused() {
        let mut f = Forest::default();
        let a = f.seat("a@x", 0);
        let b = f.seat("b@x", 1);
        f.link(a, b);
        f.link(b, a); // would close a -> b -> a
        assert_eq!(f.nodes[a].parent, None);
        assert_eq!(f.nodes[b].parent, Some(a));
        // Refused as a link, still recorded as one conversation.
        assert_eq!(f.root_of(a), f.root_of(b));
    }

    #[test]
    fn a_container_is_never_its_own_parent() {
        let mut f = Forest::default();
        let a = f.seat("a@x", 0);
        f.link(a, a);
        assert_eq!(f.nodes[a].parent, None);
        assert!(f.nodes[a].children.is_empty());
    }
}
