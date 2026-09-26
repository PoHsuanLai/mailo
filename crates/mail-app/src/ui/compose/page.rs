//! What the composer page holds: the draft's properties, the editor session, and the state of
//! the page around them. Pure: nothing here reads the store or a clock.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use mail_domain::{AccountId, Address, Draft, DraftId, ReceiptRequest};

use super::opening::doc_of;
use crate::editor::{Person, Pos, Range, Session};

/// Whether the page stands alone in the reader column or sits under a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum PageKind {
    /// A new message or a forward: a page of its own.
    New,
    /// A reply, drawn under the conversation it answers.
    Reply,
}

/// When Send sends. A scheduled send is held in the outbox until then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum When {
    Now,
    Tomorrow,
    Monday,
    /// A time typed into "Pick a time…".
    At(DateTime<Utc>),
}

impl When {
    /// The choices the Sends menu names. [`When::At`] is typed, not listed.
    pub(in crate::ui) const ALL: [When; 3] = [When::Now, When::Tomorrow, When::Monday];

    /// What the Sends menu calls a choice, worded the way the snooze menu words its times.
    pub(in crate::ui) fn label(self) -> &'static str {
        match self {
            When::Now => "Right away",
            When::Tomorrow => "Tomorrow 08:00",
            When::Monday => "Monday 09:00",
            When::At(_) => super::later::PICK_LABEL,
        }
    }

    /// What the Sends row says: the choice's name, or for a typed time, that time.
    pub(in crate::ui) fn shown<Tz: TimeZone>(self, now: DateTime<Utc>, zone: &Tz) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        match self {
            When::At(at) => super::super::menus::when_words(at, now, zone),
            _ => self.label().to_owned(),
        }
    }

    /// The menu key.
    pub(in crate::ui) fn key(self) -> &'static str {
        match self {
            When::Now => "now",
            When::Tomorrow => "tomorrow",
            When::Monday => "monday",
            When::At(_) => super::later::PICK_KEY,
        }
    }

    pub(in crate::ui) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|when| when.key() == key)
    }

    /// The moment a scheduled send is due, in `zone`. `None` for right away.
    pub(in crate::ui) fn due<Tz: TimeZone>(
        self,
        now: DateTime<Utc>,
        zone: &Tz,
    ) -> Option<DateTime<Utc>> {
        let local = now.with_timezone(zone).date_naive();
        let (day, hour) = match self {
            When::Now => return None,
            When::At(at) => return Some(at),
            When::Tomorrow => (local.succ_opt()?, 8),
            When::Monday => {
                let ahead = 7 - local.weekday().num_days_from_monday() as i64;
                (local + Duration::days(ahead), 9)
            }
        };
        let at = day.and_hms_opt(hour, 0, 0)?;
        zone.from_local_datetime(&at)
            .earliest()
            .map(|at| at.with_timezone(&Utc))
    }
}

/// Which recipient list a chip or a typed address belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum List {
    To,
    Cc,
}

/// The Cc row is hidden until it is used, asked for, or an `@` adds someone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum CcRow {
    Hidden,
    Shown,
}

/// The one floating thing open on the page, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Float {
    Closed,
    /// The `/` menu. `anchor` is where the `/` is; the query runs from after it to the caret.
    Slash {
        anchor: Pos,
        active: usize,
    },
    /// The `@` menu, anchored the same way.
    Mention {
        anchor: Pos,
        active: usize,
    },
    /// Turn into, from the selection bubble.
    Turn,
    /// The bubble's link field, with what has been typed.
    Link(String),
    /// The ⋮⋮ menu of the object at this node.
    Object(usize),
    /// Send from.
    From,
    /// When to send.
    Sends,
    /// Whether to sign or encrypt, and with what.
    Protection,
    /// "Pick a time…" under the Sends row, with what has been typed.
    PickTime(String),
    /// "Save as template…", with the name typed so far.
    SaveTemplate(String),
    /// "Start from a template": the templates, at the caret.
    Templates {
        active: usize,
    },
    /// People matching what is typed in a recipient field.
    People {
        list: List,
        active: usize,
    },
}

/// What stopped the last Send, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Guard {
    Clear,
    /// No recipients: the To row shakes. The count restarts the animation on a second press.
    Shake(u32),
    /// An attachment is mentioned and none is attached.
    Warn,
}

/// The status dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Saved {
    /// As it was opened: nothing to save yet.
    Opened,
    /// Everything on the page is in the store, written by this page.
    Clean,
    /// Changed since the last save.
    Dirty,
}

/// Writing, or folding away after Send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Phase {
    Writing,
    Folding,
    /// Parked, sent or discarded: going away, with nothing left to keep.
    Closed,
}

/// Focus mode: the page takes the whole card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Focus {
    Off,
    On,
}

/// The folded original on a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Fold {
    Folded,
    Open,
}

/// Where the surface's input has reached the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Wire {
    /// The last input the surface handed over, numbered in order.
    pub seq: u64,
    /// Between `compositionstart` and `compositionend`: where the composition began. While set,
    /// nothing changes the document, so nothing re-renders the paragraph the IME is writing in.
    pub composing: Option<Range>,
}

/// The composer page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Page {
    pub draft: DraftId,
    pub kind: PageKind,
    pub from: AccountId,
    pub subject: String,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    /// What is typed into the To and Cc fields and not yet a chip.
    pub typed_to: String,
    pub typed_cc: String,
    pub cc_row: CcRow,
    pub when: When,
    /// Whether the message asks its recipients for a read receipt.
    pub receipt: ReceiptRequest,
    /// How the message is signed or encrypted when it is sent: OpenPGP or S/MIME, never both.
    pub protection: super::protection::Protection,
    /// What stands between that and Send, in the warning bar.
    pub seal_bar: super::seal::SealBar,
    /// What the draft carries, as `(name, size)`.
    pub attached: Vec<(String, String)>,
    pub session: Session,
    /// A selection that is not collapsed. The bubble shows while it is set.
    pub selection: Option<Range>,
    pub wire: Wire,
    /// Paragraphs whose DOM an IME wrote, keyed by node index. Bumping the generation gives the
    /// paragraph a new key, so Dioxus builds it fresh instead of patching what the IME left.
    pub fresh: BTreeMap<usize, u64>,
    pub float: Float,
    pub guard: Guard,
    pub saved: Saved,
    /// Bumped on every change, so a debounced save can tell whether it is still the latest.
    pub edits: u64,
    pub phase: Phase,
    pub focus: Focus,
    pub quoted: Fold,
    /// The address of a chip that just joined, which flashes once.
    pub flash: Option<String>,
    /// The contact book's suggestions for what is being typed now — in To, in Cc, or after an
    /// `@` — best first. Asked again on each keystroke; the fields and `@` never type at once.
    pub people: Vec<Person>,
    /// The contact groups offered above [`Page::people`] for what is typed in To or Cc, each
    /// already expanded to its members.
    pub groups: Vec<crate::ui::contacts::groups::Offer>,
    pub notice: Option<String>,
}

impl Page {
    /// The page for `draft`, with `people` to suggest and `attached` to list.
    pub(in crate::ui) fn of(
        draft: &Draft,
        people: Vec<Person>,
        attached: Vec<(String, String)>,
    ) -> Self {
        let kind = if draft.in_reply_to.is_some() {
            PageKind::Reply
        } else {
            PageKind::New
        };
        let cc: Vec<Person> = draft.cc.iter().map(person).collect();
        Self {
            draft: draft.id,
            kind,
            from: draft.account,
            subject: draft.subject.clone(),
            to: draft.to.iter().map(person).collect(),
            cc_row: if cc.is_empty() {
                CcRow::Hidden
            } else {
                CcRow::Shown
            },
            cc,
            typed_to: String::new(),
            typed_cc: String::new(),
            when: When::Now,
            receipt: draft.receipt,
            protection: super::protection::Protection::of(draft),
            seal_bar: super::seal::SealBar::Clear,
            attached,
            session: Session::with(doc_of(draft)),
            selection: None,
            wire: Wire {
                seq: 0,
                composing: None,
            },
            fresh: BTreeMap::new(),
            float: Float::Closed,
            guard: Guard::Clear,
            saved: Saved::Opened,
            edits: 0,
            phase: Phase::Writing,
            focus: Focus::Off,
            quoted: Fold::Folded,
            flash: None,
            people,
            groups: Vec::new(),
            notice: None,
        }
    }

    /// Something changed: the dot turns and the next debounced save has work.
    pub(in crate::ui) fn touch(&mut self) {
        self.saved = Saved::Dirty;
        self.edits = self.edits.wrapping_add(1);
    }

    /// The draft as this page would store it, over `base` so everything the page does not show
    /// (the identity, what it replies to, Bcc) is whatever the store says.
    pub(in crate::ui) fn apply_to(&self, base: &Draft, now: DateTime<Utc>) -> Draft {
        let doc = &self.session.doc;
        Draft {
            to: self.to.iter().map(address).collect(),
            cc: self.cc.iter().map(address).collect(),
            subject: self.subject.clone(),
            text: crate::editor::to_flowed(doc),
            html: Some(crate::editor::to_html(doc)),
            receipt: self.receipt,
            openpgp: self.protection.openpgp(),
            smime: self.protection.smime(),
            updated: now,
            ..base.clone()
        }
    }

    /// Ask for a read receipt, or stop asking.
    pub(in crate::ui) fn toggle_receipt(&mut self) {
        self.receipt = match self.receipt {
            ReceiptRequest::Unrequested => ReceiptRequest::Requested,
            ReceiptRequest::Requested => ReceiptRequest::Unrequested,
        };
        self.touch();
    }

    /// The recipients of `list`.
    pub(in crate::ui) fn list_mut(&mut self, list: List) -> &mut Vec<Person> {
        match list {
            List::To => &mut self.to,
            List::Cc => &mut self.cc,
        }
    }

    /// The text typed into `list`'s field.
    pub(in crate::ui) fn typed_mut(&mut self, list: List) -> &mut String {
        match list {
            List::To => &mut self.typed_to,
            List::Cc => &mut self.typed_cc,
        }
    }

    /// The key a paragraph renders under: its index, plus a generation once an IME wrote it.
    pub(in crate::ui) fn node_key(&self, index: usize) -> String {
        match self.fresh.get(&index) {
            Some(generation) => format!("n{index}g{generation}"),
            None => format!("n{index}"),
        }
    }
}

/// A person from an address. A bare address is named by its local part.
pub(in crate::ui) fn person(address: &Address) -> Person {
    let name = match address.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => address
            .email
            .split('@')
            .next()
            .unwrap_or(&address.email)
            .to_owned(),
    };
    Person {
        name,
        address: address.email.clone(),
    }
}

/// The address a person chip stands for.
pub(in crate::ui) fn address(person: &Person) -> Address {
    Address {
        name: Some(person.name.clone()).filter(|name| {
            let local = person.address.split('@').next().unwrap_or("");
            !name.is_empty() && name != local
        }),
        email: person.address.clone(),
    }
}
