//! A message's source: its stored raw bytes, every header included, as plain monospace text.
//!
//! The third way to show a message, beside Reader and Original. Nothing in it is parsed as
//! HTML or as MIME: the bytes are decoded as UTF-8 and put in a text node, which the document
//! never reads as markup. What cannot be shown as itself says so rather than being dropped: a
//! control character is drawn as its Unicode control picture, a bidi or other format control
//! as its code point (so a sender's right-to-left override cannot reorder what the source says),
//! and a byte that is not UTF-8 as U+FFFD, with a note counting them.
//!
//! The blob is read off the thread that draws, from the press that asks for it (F140). The
//! bytes are what the store holds, which is the message as it arrived except in two cases the
//! view names: a body not downloaded yet, and a large IMAP message rebuilt from its parts.

use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;
use mail_domain::{BlobId, MessageId};
use mail_store::{SqliteStore, Store};

use crate::view::Shell;

/// How one message is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Shown {
    /// As blocks, drawn by mailo.
    #[default]
    Reader,
    /// The sender's sanitized HTML, in its sealed frame.
    Original,
    /// The raw message.
    Source,
}

/// Which way each message of the open thread is shown. A message not in it is [`Shown::Reader`].
pub(super) type Showing = HashMap<MessageId, Shown>;

/// The source of each stored body asked for, by the blob it was read from: a body that arrives
/// or changes is a new blob and is read again.
pub(super) type Sources = HashMap<BlobId, Loaded>;

/// Where one message's source is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Loaded {
    Loading,
    Ready(Source),
    Failed(String),
}

/// A message's source, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Source {
    /// The bytes as text, lines ending in `\n`.
    pub text: String,
    /// What the reader should know about how `text` stands to the message: each a sentence.
    pub notes: Vec<String>,
}

/// The most of a message the view draws. A message with its attachments can be tens of
/// megabytes, which as one text node is more than a window should lay out; the headers, which
/// are what a source view is mostly opened for, are at the top.
pub(super) const SHOWN_LIMIT: usize = 256 * 1024;

/// How `message` is shown now.
pub(super) fn shown(showing: &Showing, message: MessageId) -> Shown {
    showing.get(&message).copied().unwrap_or_default()
}

/// Show `message` as its source, reading the blob `body` if it has not been read.
///
/// Called from a press. The read is spawned there, on a blocking thread, which is where a task
/// is polled (F140) and where a large blob does not stall the frame.
pub(super) fn open(
    message: MessageId,
    body: Option<BlobId>,
    mut showing: Signal<Showing>,
    sources: Signal<Sources>,
) {
    showing.write().insert(message, Shown::Source);
    if let Some(blob) = body {
        load(message, blob, sources);
    }
}

/// Read `blob`, `message`'s body, into `sources`, unless it is there or on its way.
fn load(message: MessageId, blob: BlobId, mut sources: Signal<Sources>) {
    if matches!(
        sources.peek().get(&blob),
        Some(Loaded::Loading | Loaded::Ready(_))
    ) {
        return;
    }
    sources.write().insert(blob, Loaded::Loading);
    let store = consume_context::<Arc<SqliteStore>>();
    spawn(async move {
        let read = tokio::task::spawn_blocking(move || read(&store, message, blob)).await;
        let loaded = match read {
            Ok(Ok(source)) => Loaded::Ready(source),
            Ok(Err(why)) => Loaded::Failed(why),
            Err(error) => Loaded::Failed(format!("reading it stopped before it finished: {error}")),
        };
        sources.write().insert(blob, loaded);
    });
}

/// `message`'s source from the store: its body `blob`, and what is to be said about it.
fn read(store: &SqliteStore, message: MessageId, blob: BlobId) -> Result<Source, String> {
    let message = store.message(message).map_err(|e| e.to_string())?;
    let bytes = store
        .blobs()
        .get(&store.connection(), blob)
        .map_err(|e| format!("the stored message could not be read: {e}"))?;
    let mut notes = Vec::new();
    if crate::compose::rebuilt(&message, &bytes) {
        notes.push(
            "This message is large, so it was downloaded in parts. What is shown is rebuilt \
             from them: every header and text part as sent, and each attachment left on the \
             server as its headers with an empty body."
                .to_owned(),
        );
    }
    let (text, unreadable) = as_text(&bytes, SHOWN_LIMIT);
    if bytes.len() > SHOWN_LIMIT {
        notes.push(format!(
            "Showing the first {} of {}.",
            crate::attach::human_size(text.len() as u64),
            crate::attach::human_size(bytes.len() as u64)
        ));
    }
    if unreadable > 0 {
        notes.push(format!(
            "{unreadable} byte sequence(s) are not UTF-8 and are shown as \u{FFFD}."
        ));
    }
    Ok(Source { text, notes })
}

/// The first `limit` bytes of `raw` (cut at a line end where there is one) as text to draw, and
/// how many byte sequences in them were not UTF-8.
///
/// CRLF and LF end a line. Every other control character is drawn as its control picture
/// (U+2400–U+2421), so a bare CR or an escape is seen and never acted on; a C1 control or a
/// format character that reorders or hides text (bidi overrides and isolates, zero-width marks)
/// is drawn as `<U+XXXX>`.
pub(super) fn as_text(raw: &[u8], limit: usize) -> (String, usize) {
    let shown = if raw.len() > limit {
        let head = &raw[..limit];
        match head.iter().rposition(|byte| *byte == b'\n') {
            Some(end) => &head[..=end],
            None => head,
        }
    } else {
        raw
    };
    let mut text = String::with_capacity(shown.len());
    let mut unreadable = 0;
    let mut pending_cr = false;
    for chunk in shown.utf8_chunks() {
        for c in chunk.valid().chars() {
            if pending_cr {
                pending_cr = false;
                if c == '\n' {
                    text.push('\n');
                    continue;
                }
                text.push('\u{240D}');
            }
            match c {
                '\r' => pending_cr = true,
                '\n' | '\t' => text.push(c),
                '\u{0}'..='\u{1F}' => text.push(char::from_u32(0x2400 + c as u32).unwrap_or('?')),
                '\u{7F}' => text.push('\u{2421}'),
                c if hides_or_reorders(c) => text.push_str(&format!("<U+{:04X}>", c as u32)),
                c => text.push(c),
            }
        }
        if !chunk.invalid().is_empty() {
            if pending_cr {
                pending_cr = false;
                text.push('\u{240D}');
            }
            unreadable += 1;
            text.push('\u{FFFD}');
        }
    }
    if pending_cr {
        text.push('\u{240D}');
    }
    (text, unreadable)
}

/// A character that changes how the text around it is drawn, or draws as nothing: C1 controls,
/// and the bidi and zero-width format characters (Unicode's `Cf` that matter for spoofing).
fn hides_or_reorders(c: char) -> bool {
    matches!(
        c,
        '\u{80}'..='\u{9F}'
            | '\u{AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// One message's source, drawn when it is the way that message is shown.
///
/// A sibling after the message's body, never its parent, for the reason the reader gives: a new
/// parent reloads the Original frame. Forward as attachment is offered here, beside the bytes it
/// would send; a refusal is said in the reader's notice, `said`.
#[component]
pub(super) fn SourceView(
    message: MessageId,
    body: Option<BlobId>,
    showing: Signal<Showing>,
    sources: Signal<Sources>,
    shell: Signal<Shell>,
    revision: Option<Signal<u64>>,
    said: Signal<Option<String>>,
) -> Element {
    if shown(&showing.read(), message) != Shown::Source {
        return rsx! {};
    }
    let Some(blob) = body else {
        return rsx! {
            div { class: "source",
                p { class: "source-note",
                    "Only this message's headers have been downloaded, and they are stored as \
                     fields, not as the bytes they came in. Its source is here once its body is."
                }
            }
        };
    };
    let loaded = sources.read().get(&blob).cloned();
    rsx! {
        div { class: "source",
            div { class: "source-tools",
                ds::Button {
                    variant: ds::ButtonVariant::Mini,
                    label: "Forward as attachment",
                    aria_label: "Forward as attachment".to_owned(),
                    onclick: super::super::press::on_primary(move || {
                        forward_attached(message, shell, revision, said)
                    }),
                }
            }
            match loaded {
                None => rsx! {
                    ds::Button {
                        variant: ds::ButtonVariant::Mini,
                        label: "Load the source",
                        aria_label: "Load the source".to_owned(),
                        onclick: super::super::press::on_primary(move || load(message, blob, sources)),
                    }
                },
                Some(Loaded::Loading) => rsx! { p { class: "source-note", "Reading the message…" } },
                Some(Loaded::Failed(why)) => rsx! { p { class: "source-note", "{why}" } },
                Some(Loaded::Ready(source)) => rsx! {
                    for note in source.notes {
                        p { class: "source-note", "{note}" }
                    }
                    pre { class: "source-text mono", "{source.text}" }
                },
            }
        }
    }
}

/// Open a composer on `message` forwarded as an attachment, or say in `said` why it cannot be.
///
/// The draft is made on a blocking thread: it reads the whole message to check it is the one
/// that was sent.
fn forward_attached(
    message: MessageId,
    mut shell: Signal<Shell>,
    revision: Option<Signal<u64>>,
    mut said: Signal<Option<String>>,
) {
    let store = consume_context::<Arc<SqliteStore>>();
    spawn(async move {
        let made = tokio::task::spawn_blocking(move || {
            crate::compose::draft_forward_attached(&store, message, &[], "", chrono::Utc::now())
        })
        .await;
        match made {
            Ok(Ok(draft)) => {
                shell.write().compose(&draft);
                if let Some(mut revision) = revision {
                    revision += 1;
                }
            }
            Ok(Err(why)) => said.set(Some(format!("Not forwarded as an attachment: {why}."))),
            Err(error) => said.set(Some(format!(
                "Forwarding stopped before it finished: {error}"
            ))),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::as_text;

    #[test]
    fn the_bytes_are_drawn_as_themselves_and_what_cannot_be_is_named() {
        const CASES: &[(&str, &[u8], &str, usize)] = &[
            ("CRLF ends a line", b"A: 1\r\nB: 2\r\n", "A: 1\nB: 2\n", 0),
            ("so does a bare LF", b"A: 1\nB: 2", "A: 1\nB: 2", 0),
            ("a bare CR is seen", b"A\rB\r", "A\u{240D}B\u{240D}", 0),
            ("an escape is seen", b"\x1b[31mred", "\u{241B}[31mred", 0),
            ("a tab stays", b"\tx", "\tx", 0),
            (
                "a bidi override cannot reorder",
                "a\u{202E}cba".as_bytes(),
                "a<U+202E>cba",
                0,
            ),
            ("markup is text", b"<b>hi</b>", "<b>hi</b>", 0),
            ("UTF-8 is itself", "caf\u{e9}".as_bytes(), "caf\u{e9}", 0),
            ("Latin-1 is not UTF-8", b"caf\xe9!", "caf\u{FFFD}!", 1),
            ("a CR before a bad byte", b"a\r\xff", "a\u{240D}\u{FFFD}", 1),
        ];
        for (name, raw, text, bad) in CASES {
            assert_eq!(as_text(raw, 1024), ((*text).to_owned(), *bad), "{name}");
        }
    }

    #[test]
    fn a_long_message_is_cut_at_a_line_end_within_the_limit() {
        let raw = b"Subject: one\r\nSubject: two\r\nSubject: three\r\n";
        let (text, _) = as_text(raw, 30);
        assert_eq!(text, "Subject: one\nSubject: two\n");
    }
}
