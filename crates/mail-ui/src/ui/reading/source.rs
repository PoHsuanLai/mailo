//! A message as it came: its header lines (Show All Headers, ⇧⌘H) or its whole source (View
//! Source, ⌥⌘R), as plain monospaced text in a sheet over the window, to read, select and copy.
//!
//! What is shown is the stored bytes (`mail_core::export::raw_head`: the first of them, off the
//! thread that draws, never the whole blob), never a rendering, so a
//! message that draws wrongly can be looked at whole. A message laid out as text is costly past
//! a size, and one with large attachments is mostly base64 nobody reads, so the sheet shows the
//! first [`SHOWN`] bytes, cut at a line's end, and says how much there was; Download in the ⋯
//! writes all of it.

use crate::ui::common::in_card;
use crate::ui::copy;
use crate::ui::press::{SheetClose, on_primary};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;
use mail_client::Client;
use mail_client::export::RawHead;
use mail_client::prelude::CoreError;
use mail_domain::{MessageId, ThreadId, human_size};

/// Which of a message's raw text a sheet shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raw {
    /// The header lines: everything before the first empty line.
    Headers,
    /// The whole message, headers and body.
    Source,
}

/// A sheet asked for: which message, and which of its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawAsked {
    pub message: MessageId,
    pub raw: Raw,
}

/// The most of a message the sheet lays out, in bytes.
const SHOWN: usize = 128 * 1024;

/// What the sheet shows of a message's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Shown {
    pub text: String,
    /// How many bytes of [`Self::text`]'s part there were, when only the first of them are
    /// shown.
    pub cut: Option<usize>,
}

/// The header lines of `raw`: up to its first empty line, or all of it when it has none.
fn header_block(raw: &[u8]) -> &[u8] {
    let ends = [&b"\r\n\r\n"[..], &b"\n\n"[..]];
    ends.iter()
        .filter_map(|end| {
            raw.windows(end.len())
                .position(|at| at == *end)
                .map(|at| at + end.len() / 2)
        })
        .min()
        .map_or(raw, |at| &raw[..at])
}

/// What `raw` shows as `what`, the first `most` bytes of it, cut back to the end of a line where
/// there is one. Bytes that are not UTF-8 (an 8-bit header) show as the replacement character.
#[cfg(test)]
pub(super) fn shown(raw: &[u8], what: Raw, most: usize) -> Shown {
    let part = match what {
        Raw::Headers => header_block(raw),
        Raw::Source => raw,
    };
    cut_at(part, part.len(), most)
}

/// What a message's first bytes show as `what`: the same as [`shown`] of the whole message, from
/// only the head of it that was read (`head.bytes`, at most `most` + 1 of them).
pub(super) fn shown_head(head: &RawHead, what: Raw, most: usize) -> Shown {
    let whole = usize::try_from(head.whole).unwrap_or(usize::MAX);
    let part = match what {
        Raw::Headers => header_block(&head.bytes),
        Raw::Source => &head.bytes[..],
    };
    // A part that runs to the end of what was read goes on as far as the message does.
    let length = if part.len() == head.bytes.len() {
        whole
    } else {
        part.len()
    };
    cut_at(part, length, most)
}

/// `part`, `length` bytes long in all of which `part` holds the first ones: whole when it fits in
/// `most`, else its first `most` bytes cut back to the end of a line.
fn cut_at(part: &[u8], length: usize, most: usize) -> Shown {
    if length <= most {
        return Shown {
            text: String::from_utf8_lossy(part).into_owned(),
            cut: None,
        };
    }
    let head = &part[..most.min(part.len())];
    let line = head
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(head, |at| &head[..=at]);
    Shown {
        text: String::from_utf8_lossy(line).into_owned(),
        cut: Some(length),
    }
}

/// The message the ⋯ and the keys mean among `messages`: the one a list of single messages
/// opened (`focus`), else the one the conversation opens on, the newest.
fn meant(messages: &[MessageId], focus: Option<MessageId>) -> Option<MessageId> {
    focus
        .filter(|focus| messages.contains(focus))
        .or_else(|| messages.last().copied())
}

/// Ask for `raw` of `thread`'s message the keys mean ([`meant`]).
pub(in crate::ui) fn ask(shell: Signal<Shell>, thread: ThreadId, raw: Raw) {
    let client = crate::ui::live::client();
    let focus = shell.peek().focus;
    let newest = client
        .stored()
        .thread(thread)
        // A conversation gone since the key was pressed has nothing to show.
        .ok()
        .and_then(|loaded| meant(&loaded.messages, focus));
    if let Some(message) = newest {
        ask_message(shell, message, raw);
    }
}

/// Ask for `raw` of `message`, as its own menu does.
pub(in crate::ui) fn ask_message(mut shell: Signal<Shell>, message: MessageId, raw: Raw) {
    shell.write().raw = Some(RawAsked { message, raw });
}

/// Close the sheet.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().raw = None;
    crate::ui::host::Host::focus_app();
}

/// The sheet, over the window while it is asked for.
#[component]
pub(in crate::ui) fn RawSheet(shell: Signal<Shell>) -> Element {
    let Some(asked) = shell.read().raw else {
        return rsx! {};
    };
    let title = match asked.raw {
        Raw::Headers => copy::titles::ALL_HEADERS,
        Raw::Source => copy::titles::MESSAGE_SOURCE,
    };
    rsx! {
        Sheet {
            label: title,
            attach: Attach::Window,
            common: in_card(),
            onclose: move |()| close(shell),
            // Read once for each message and part asked for, not on every draw of the window.
            RawBody { key: "{asked.message}-{asked.raw:?}", asked, title, shell }
        }
    }
}

/// What was read for the sheet: the text, and a line about it (how much is left out, or why
/// there is nothing).
type Read = (Option<String>, Option<String>);

/// What the sheet shows of what the client read for `asked`: the text and the line about it.
fn read_of(head: Result<Option<RawHead>, CoreError>, raw: Raw) -> Read {
    match head {
        Ok(Some(head)) => {
            let shown = shown_head(&head, raw, SHOWN);
            let note = shown.cut.map(|whole| {
                copy::status::source_cut(
                    &human_size(shown.text.len() as u64),
                    &human_size(whole as u64),
                )
            });
            (Some(shown.text), note)
        }
        Ok(None) => (None, Some(copy::status::SOURCE_NOT_HERE.to_owned())),
        Err(error) => (None, Some(String::from(error))),
    }
}

/// Read `asked` from the client: the first [`SHOWN`] bytes and one more, to know there are more,
/// and never the whole of a large message.
fn read(client: &Client, asked: RawAsked) -> Read {
    read_of(
        client.export().raw_head(asked.message, SHOWN + 1),
        asked.raw,
    )
}

#[component]
fn RawBody(asked: RawAsked, title: &'static str, shell: Signal<Shell>) -> Element {
    // Read off the thread that draws, as the sheet opens; the sheet says so until it is in.
    let reading = use_resource(move || {
        let client = crate::ui::live::client();
        async move {
            tokio::task::spawn_blocking(move || read(&client, asked))
                .await
                .unwrap_or_else(|_| (None, Some(copy::errors::NOT_READ_HERE.to_owned())))
        }
    });
    let read_so_far: Option<Read> = (*reading.read()).clone();
    let Some((text, note)) = read_so_far else {
        return rsx! {
            div { class: "raw-sheet",
                Label { text: title, style: LabelStyle::Title }
                Label {
                    text: copy::status::LOADING,
                    role: LabelRole::Tertiary,
                    style: LabelStyle::Footnote,
                }
                div { class: "sheet-actions",
                    SheetClose { label: copy::controls::DONE, on_close: move |()| close(shell) }
                }
            }
        };
    };
    rsx! {
        div { class: "raw-sheet",
            Label { text: title, style: LabelStyle::Title }
            if let Some(note) = note {
                Label { text: note, role: LabelRole::Tertiary, style: LabelStyle::Footnote }
            }
            if let Some(text) = text.clone() {
                div { class: "raw-text", "data-raw": "{asked.raw:?}", "{text}" }
            }
            div { class: "sheet-actions",
                if let Some(text) = text {
                    Button {
                        label: copy::controls::COPY_ALL,
                        onclick: on_primary(move || crate::ui::host::Host::copy(&text)),
                    }
                }
                SheetClose { label: copy::controls::DONE, on_close: move |()| close(shell) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &[u8] = b"From: ada@example.test\r\nSubject: Hi\r\n\r\nBody line\r\n\r\nMore\r\n";

    #[test]
    fn the_headers_are_the_lines_before_the_first_empty_one() {
        // (case, bytes, header lines)
        let cases: [(&str, &[u8], &[u8]); 3] = [
            ("crlf", RAW, b"From: ada@example.test\r\nSubject: Hi\r\n"),
            ("lf", b"A: 1\nB: 2\n\nbody\n", b"A: 1\nB: 2\n"),
            ("no body", b"A: 1\r\n", b"A: 1\r\n"),
        ];
        for (case, raw, want) in cases {
            assert_eq!(header_block(raw), want, "{case}");
        }
    }

    #[test]
    fn a_long_message_is_cut_at_a_line_and_says_how_long_it_was() {
        let whole = shown(RAW, Raw::Source, 1024);
        assert_eq!(whole.text.as_bytes(), RAW);
        assert_eq!(whole.cut, None);
        let headers = shown(RAW, Raw::Headers, 1024);
        assert_eq!(headers.text, "From: ada@example.test\r\nSubject: Hi\r\n");
        // 30 bytes reach into "Subject: Hi": the cut goes back to the end of the From line.
        let cut = shown(RAW, Raw::Source, 30);
        assert_eq!(cut.text, "From: ada@example.test\r\n");
        assert_eq!(cut.cut, Some(RAW.len()));
        // A line longer than the room is cut where the room ends.
        let long = shown(b"abcdefgh", Raw::Source, 3);
        assert_eq!((long.text.as_str(), long.cut), ("abc", Some(8)));
        // An 8-bit header shows, its byte replaced.
        let eight = shown(b"Subject: caf\xe9\r\n\r\n", Raw::Headers, 1024);
        assert_eq!(eight.text, "Subject: caf\u{fffd}\r\n");
    }
}

#[cfg(test)]
mod head_tests {
    use super::*;

    /// Only the head of a large message is read, and the sheet shows what it would of the whole:
    /// the same text, and a cut that names the message's real size.
    #[test]
    fn a_head_shows_what_the_whole_would_and_names_the_real_size() {
        let mut whole = b"From: ada@example.test\r\nSubject: Hi\r\n\r\n".to_vec();
        whole.resize(10_000, b'a');
        let most = 100;
        let head = RawHead {
            bytes: whole[..most + 1].to_vec(),
            whole: whole.len() as u64,
        };
        for what in [Raw::Source, Raw::Headers] {
            assert_eq!(
                shown_head(&head, what, most).text,
                shown(&whole, what, most).text,
                "{what:?}"
            );
        }
        assert_eq!(shown_head(&head, Raw::Source, most).cut, Some(10_000));
        // The headers end inside the head: they were read whole, so nothing of them is cut.
        let headers = shown_head(&head, Raw::Headers, most);
        assert_eq!(headers.text, "From: ada@example.test\r\nSubject: Hi\r\n");
        assert_eq!(headers.cut, None);
        // A message that fits is shown whole.
        let small = RawHead {
            bytes: b"A: 1\r\n\r\nbody".to_vec(),
            whole: 12,
        };
        let all = shown_head(&small, Raw::Source, most);
        assert_eq!((all.text.as_str(), all.cut), ("A: 1\r\n\r\nbody", None));
        // Headers that run past the head are cut, and say how long the message is.
        let long = RawHead {
            bytes: vec![b'h'; most + 1],
            whole: 5_000,
        };
        assert_eq!(shown_head(&long, Raw::Headers, most).cut, Some(5_000));
    }

    #[test]
    fn what_is_read_is_the_text_and_a_line_about_it() {
        let (text, note) = read_of(Ok(None), Raw::Source);
        assert_eq!(text, None);
        assert_eq!(note.as_deref(), Some(copy::status::SOURCE_NOT_HERE));
        let head = RawHead {
            bytes: b"A: 1\r\n".to_vec(),
            whole: 6,
        };
        assert_eq!(
            read_of(Ok(Some(head)), Raw::Source),
            (Some("A: 1\r\n".to_owned()), None)
        );
    }
}

#[cfg(test)]
mod meant_tests {
    use super::meant;
    use mail_domain::MessageId;

    #[test]
    fn the_message_a_key_means_is_the_one_opened_alone_else_the_newest() {
        let [a, b, c] = [(); 3].map(|()| MessageId::generate());
        assert_eq!(meant(&[a, b, c], None), Some(c));
        assert_eq!(meant(&[a, b, c], Some(a)), Some(a));
        assert_eq!(
            meant(&[a, b], Some(c)),
            Some(b),
            "one of another conversation"
        );
        assert_eq!(meant(&[], Some(a)), None);
    }
}
