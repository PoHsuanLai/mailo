//! Lane 16: finding in a conversation. ⌘F with a conversation open brings up a find bar over it,
//! not the mail's search: what is typed is found in its messages as they are drawn, a plain-text
//! message and a sender's HTML alike, counted ("1 of 4") and marked in their frames, the current
//! match apart. Enter and ⌘G step forward, Shift+Enter and ⇧⌘G back, wrapping; Escape closes the
//! bar and takes every mark out, leaving each message's words as they were. ⌘K is still the
//! search panel's.

use ds_harness::Harness;

use super::drive::{Drive, Key, PRIMARY};
use super::window::{Window, deliver};

const SUBJECT: &str = "The quarterly report";
/// The bar's field and its count.
const FIELD: &str = ".find-bar input";
const COUNT: &str = ".find-bar .find-count";
/// The search panel's field.
const SEARCH: &str = ".ds-search-card input";

/// A conversation of two: a plain-text message far taller than the window, its second match at
/// its foot, and a reply in HTML whose word is cut by a `<b>` and which hides a preheader, as
/// bulk mail does.
fn letters(store: &mail_core::SqliteStore) {
    let filler = (1..=60)
        .map(|n| format!("Line {n} of the long letter."))
        .collect::<Vec<_>>()
        .join("\r\n");
    let plain = format!(
        "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: {SUBJECT}\r\n\
         Date: {}\r\nMessage-ID: <report-1@example.test>\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n\
         The quarterly Report is attached.\r\n{filler}\r\nThe report covers March.\r\n",
        super::seed::hours_ago(2)
    );
    deliver(store, "report-1", &plain);
    let html = format!(
        "From: Grace <grace@example.test>\r\nTo: me@example.test\r\nSubject: Re: {SUBJECT}\r\n\
         Date: {}\r\nMessage-ID: <report-2@example.test>\r\n\
         In-Reply-To: <report-1@example.test>\r\nReferences: <report-1@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <html><body><p style=\"display:none\">report preheader</p>\
         <p>Thanks for the <b>rep</b>ort, read it twice.</p><p>One more r\u{e9}port note.</p>\
         </body></html>\r\n",
        super::seed::hours_ago(1)
    );
    deliver(store, "report-2", &html);
}

/// The frames of the open conversation, oldest first.
fn frames(harness: &Harness) -> usize {
    harness.count(".reader-body iframe.html")
}

/// What `look` reads in the `n`th frame's document (1-based), or `None` while it is not there.
fn in_frame<T>(
    harness: &Harness,
    n: usize,
    look: impl Fn(&blitz_dom::BaseDocument) -> Option<T>,
) -> Option<T> {
    let selector = format!(".reader-body article.frame:nth-of-type({n}) iframe.html");
    harness.with_doc(|doc| {
        // A selector of this file's that does not parse finds nothing, as a missing frame does.
        let iframe = doc.query_selector(&selector).ok()??;
        let sub = doc.get_node(iframe)?.subdoc()?.inner();
        look(&sub)
    })
}

/// The `n`th frame's marks: every one, and the current one's words.
fn marks(harness: &Harness, n: usize) -> (usize, Vec<String>) {
    let count = in_frame(harness, n, |doc| {
        // A selector of this file's that does not parse counts nothing.
        Some(doc.query_selector_all("mark[data-mailo-find]").ok()?.len())
    });
    let current = in_frame(harness, n, |doc| {
        let found = doc
            .query_selector_all("mark[data-mailo-find=\"current\"]")
            // The same.
            .ok()?;
        Some(
            found
                .iter()
                .filter_map(|id| doc.get_node(*id).map(blitz_dom::Node::text_content))
                .collect(),
        )
    });
    // A frame not drawn yet holds no marks.
    (count.unwrap_or_default(), current.unwrap_or_default())
}

/// The `n`th frame's words.
fn words(harness: &Harness, n: usize) -> String {
    // A frame not drawn yet has no words.
    in_frame(harness, n, |doc| Some(doc.root_element().text_content())).unwrap_or_default()
}

/// How far the boxes around the first frame are scrolled, all told: the reader moves to bring a
/// match far down an earlier message into view.
fn scrolled(harness: &Harness) -> f64 {
    let selector = ".reader-body article.frame:nth-of-type(1) iframe.html";
    harness
        .with_doc(|doc| {
            // A selector of this file's that does not parse finds nothing, as a missing frame does.
            let mut at = doc.query_selector(selector).ok()?;
            let mut total = 0.0;
            while let Some(id) = at {
                let node = doc.get_node(id)?;
                total += node.scroll_offset().y;
                at = node.layout_parent.get();
            }
            Some(total + doc.viewport_scroll().y)
        })
        // No frame drawn: nothing scrolled for it.
        .unwrap_or_default()
}

/// The bar's count, as drawn.
fn count(harness: &Harness) -> Option<String> {
    harness.text_of(COUNT).map(|text| text.trim().to_owned())
}

#[test]
fn command_f_finds_in_the_open_conversation_and_command_k_still_searches() {
    let mut window = Window::open(letters);
    window.open_subject(SUBJECT);
    window.until("both messages are drawn", |h| {
        frames(h) == 2 && words(h, 1).contains("covers") && words(h, 2).contains("twice")
    });
    let before = (words(&window.harness, 1), words(&window.harness, 2));

    // ⌘F: the bar, with the keyboard; what is typed is counted in both messages, the hidden
    // preheader left out, a word cut by `<b>` and an accented one found.
    window.press(&[PRIMARY], Key::Char('f'), 1);
    window.until("⌘F puts the keyboard in the find bar", |h| {
        h.is_focused(FIELD)
    });
    assert_eq!(
        window.harness.count(SEARCH),
        0,
        "⌘F brought up the search panel"
    );
    window.type_text("report");
    window.until("four matches, on the first", |h| {
        count(h).as_deref() == Some("1 of 4")
    });
    // The HTML's first match is two marks, either side of the `</b>`.
    window.until("each frame marks its own", |h| {
        marks(h, 1) == (2, vec!["Report".to_owned()]) && marks(h, 2).0 == 3
    });

    // Enter and Shift+Enter in the field, ⌘G and ⇧⌘G anywhere, wrapping either way. The second
    // match is at the foot of the long letter, which the reader scrolls to.
    assert!(
        scrolled(&window.harness) < 1.0,
        "the reader moved for the first match"
    );
    window.press(&[], Key::Enter, 1);
    window.until("the second match, far down", |h| {
        count(h).as_deref() == Some("2 of 4") && marks(h, 1).1 == ["report"] && scrolled(h) > 200.0
    });
    let steps: [(&[Key], Key, &str); 4] = [
        (&[], Key::Enter, "3 of 4"),
        (&[Key::Shift], Key::Enter, "2 of 4"),
        (&[PRIMARY, Key::Shift], Key::Char('g'), "1 of 4"),
        (&[PRIMARY, Key::Shift], Key::Char('g'), "4 of 4"),
    ];
    for (held, key, want) in steps {
        window.press(held, key, 1);
        window.until(want, |h| count(h).as_deref() == Some(want));
    }
    window.until("the last match is the current one, in the HTML", |h| {
        marks(h, 1).1.is_empty() && marks(h, 2).1 == ["r\u{e9}port"]
    });
    window.press(&[PRIMARY], Key::Char('g'), 1);
    window.until("⌘G wraps to the first", |h| {
        count(h).as_deref() == Some("1 of 4") && marks(h, 1).1 == ["Report"]
    });

    // Across the `<b>`: one match, two marks.
    window.press(&[PRIMARY], Key::Char('f'), 1);
    window.type_text("report,");
    window.until("one match, cut by the bold", |h| {
        count(h).as_deref() == Some("1 of 1")
    });
    window.until("its two pieces are current", |h| {
        marks(h, 2).1 == ["rep", "ort,"] && marks(h, 1).0 == 0
    });
    window.type_text("x");
    window.until("nothing found", |h| {
        count(h).as_deref() == Some(mail_app::ui::copy::status::NO_MATCHES)
    });

    // Escape: the bar goes, the marks with it, and the words are as they were.
    window.press(&[], Key::Escape, 1);
    window.until("Escape closes the bar", |h| {
        h.count(".find-bar") == 0 && h.is_focused(".app")
    });
    window.until("no mark is left", |h| {
        marks(h, 1).0 == 0 && marks(h, 2).0 == 0
    });
    assert_eq!(
        (words(&window.harness, 1), words(&window.harness, 2)),
        before,
        "the messages' words changed"
    );
    assert!(
        window
            .harness
            .text_of(".reader")
            .is_some_and(|text| text.contains(SUBJECT)),
        "Escape closed the conversation too"
    );

    // ⌘K is the search panel's.
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("⌘K puts the keyboard in the search panel", |h| {
        h.is_focused(SEARCH)
    });
    assert_eq!(window.harness.count(".find-bar"), 0);
}
