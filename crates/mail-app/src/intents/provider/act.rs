//! The actions mailo declares (`dist/intents/org.quire.Mail.toml`), as a type: the name the
//! router sends, parsed once, so that the provider matches on a closed set and the manifest's
//! list and this one are held to each other by a test.

/// What a conversation action does to the conversations it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Thread {
    Archive,
    Star,
    Unstar,
    Label,
    Unlabel,
    Snooze,
}

/// One declared action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Act {
    Search,
    Read,
    Contacts,
    Thread(Thread),
    CreateDraft,
    Send,
    Forward,
}

/// Every action, with the name the manifest gives it.
pub(super) const ALL: [(&str, Act); 12] = [
    ("mail.thread.search", Act::Search),
    ("mail.thread.read", Act::Read),
    ("mail.thread.archive", Act::Thread(Thread::Archive)),
    ("mail.thread.star", Act::Thread(Thread::Star)),
    ("mail.thread.unstar", Act::Thread(Thread::Unstar)),
    ("mail.thread.label", Act::Thread(Thread::Label)),
    ("mail.thread.unlabel", Act::Thread(Thread::Unlabel)),
    ("mail.thread.snooze", Act::Thread(Thread::Snooze)),
    ("mail.draft.create", Act::CreateDraft),
    ("mail.contact.search", Act::Contacts),
    ("mail.message.send", Act::Send),
    ("mail.message.forward", Act::Forward),
];

impl Act {
    /// The action named `name`, if it is one of ours.
    pub(super) fn named(name: &str) -> Option<Act> {
        ALL.iter()
            .find(|(declared, _)| *declared == name)
            .map(|(_, act)| *act)
    }
}
