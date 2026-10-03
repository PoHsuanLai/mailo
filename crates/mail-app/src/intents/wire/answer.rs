//! What mailo answers: an outcome, a refusal, the things a search found, a preview.

use super::call::EntityId;
use super::label::Labelled;
use serde::Serialize;

/// A thing and the words that name it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntityRef {
    /// The thing.
    pub id: EntityId,
    /// Its title.
    pub title: Labelled<String>,
    /// Its subtitle.
    pub subtitle: Labelled<String>,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    /// The thing.
    pub entity: EntityRef,
    /// Why it was found, in mailo's words.
    pub why: Option<String>,
}

/// A value an action returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Output {
    /// Words.
    Text(String),
    /// Several things.
    Entities(Vec<EntityId>),
}

/// Whether an outcome can be taken back, and with what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Undoable {
    /// No.
    No,
    /// Yes, with this token.
    Yes(String),
}

/// What to do next: mailo asks for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Follow {
    /// Nothing.
    Nothing,
}

/// One message in a thread preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snip {
    /// Who wrote it.
    pub from: Labelled<String>,
    /// The first words.
    pub snippet: Labelled<String>,
    /// When, in seconds since the Unix epoch.
    pub at: i64,
}

/// What the host draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Preview {
    /// Nothing to show.
    None,
    /// A conversation, at most five messages.
    Thread {
        /// The subject.
        subject: Labelled<String>,
        /// The newest messages.
        messages: Vec<Snip>,
    },
    /// A message that would be sent.
    Message {
        /// The recipients.
        to: Vec<Labelled<String>>,
        /// The subject.
        subject: Labelled<String>,
        /// The body.
        body: Labelled<String>,
    },
}

/// What a finished action returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// A value, labelled.
    pub value: Option<Labelled<Output>>,
    /// "Archived", in mailo's words.
    pub said: Option<String>,
    /// What to show.
    pub show: Preview,
    /// Whether it can be taken back.
    pub undo: Undoable,
    /// What to do next.
    pub follow: Follow,
}

/// Why mailo did not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum AppRefusal {
    /// A parameter is missing; these are the options.
    NeedsParam {
        /// Which.
        param: String,
        /// What mailo suggests.
        options: Vec<EntityRef>,
    },
    /// The thing is gone.
    NotFound(EntityId),
    /// The thing changed since it was named.
    Stale(EntityId),
    /// Mailo is busy.
    Busy,
    /// Mailo cannot do that.
    Unsupported,
    /// It failed, for this reason.
    Failed(String),
}

/// Why an undo did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoFault {
    /// The thing is gone, or the token was never mailo's, or it was used already.
    Gone,
    /// It changed since.
    Conflict,
}

/// What `Context` answers: where a person is in the app. A process with no window is nowhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Context {
    /// The app.
    pub app: String,
    /// The window's title.
    pub window: Labelled<String>,
    /// Where in the app.
    pub here: Here,
    /// What is selected.
    pub selection: Selection,
    /// What is on screen.
    pub visible: Visible,
    /// The field with focus.
    pub text_target: TextTarget,
    /// Whether the window may be described.
    pub privacy: Privacy,
}

/// Where in an app the person is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Here {
    /// Nowhere in particular.
    Nowhere,
}

/// What is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selection {
    /// Nothing.
    Nothing,
}

/// What is on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Visible {
    /// The kind of the listed things, if one.
    pub kind: Option<String>,
    /// Those in view.
    pub items: Vec<EntityRef>,
    /// How many there are in all.
    pub total: u32,
}

/// The focused field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TextTarget {
    /// None.
    None,
}

/// Whether a window may be described to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Privacy {
    /// Ordinary.
    Normal,
    /// Described as the app alone.
    Private,
}

/// The text of a provider's answer: `Ok` and `Err` as the router reads them (`{"Ok":..}`,
/// `{"Err":..}`). Nothing here can fail to serialize; an empty text, which the router reads as an
/// app that did not answer, is what the impossible would give.
pub fn answer_of<T: Serialize, E: Serialize>(answer: &Result<T, E>) -> String {
    serde_json::to_string(answer).unwrap_or_default()
}
