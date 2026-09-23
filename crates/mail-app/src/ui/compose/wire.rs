//! The glue between the page's `contenteditable` and the editor core.
//!
//! [`GLUE`] is the only script: it forwards what the browser is about to do, and puts the caret
//! where Rust says. It never builds markup and never edits text. Everything else is here, in
//! Rust: [`hear`] turns what it forwarded into `editor::` calls on the page's session.
//!
//! The channel is a hidden `textarea` (`.c-wire`). The script writes one JSON message into it
//! and fires `input`, which Dioxus delivers to Rust synchronously. Back the other way, the body
//! carries `data-seq` (the last message Rust has applied) and `data-caret`; a mutation observer
//! applies the caret once the edits that go with it are in the page.

use serde::Deserialize;

use super::float;
use super::page::Page;
use crate::editor::{Caret, InputEvent, Pos, Range};

/// The head script. Kept under a hundred lines by a test, so it cannot become an editor.
pub(in crate::ui) const GLUE: &str = r#"<script>
// The composer's glue: forwards what the browser is about to do, puts the caret where Rust says.
(() => {
  const seg = window.Intl && Intl.Segmenter ? new Intl.Segmenter() : null;
  const parts = (s) => (seg ? [...seg.segment(s)].map((p) => p.segment) : [...s]);
  const host = (n) => { const e = n && (n.nodeType === 1 ? n : n.parentElement); return e && e.closest(".c-body"); };
  const inert = (el) => el.getAttribute("contenteditable") === "false";
  // A DOM point as [paragraph, grapheme offset], through the data-n on every paragraph node.
  const pos = (b, node, off) => {
    const el = (node.nodeType === 1 ? node : node.parentElement).closest("[data-n]");
    if (el && b.contains(el)) {
      if (inert(el)) return [+el.dataset.n, 0];
      const r = document.createRange(); r.setStart(el, 0); r.setEnd(node, off);
      return [+el.dataset.n, parts(r.toString()).length];
    }
    const kid = node.childNodes[off];
    const at = kid && kid.nodeType === 1 && (kid.matches("[data-n]") ? kid : kid.querySelector("[data-n]"));
    if (at) return [+at.dataset.n, 0];
    const all = b.querySelectorAll("[data-n]"), last = all[all.length - 1];
    return last ? [+last.dataset.n, inert(last) ? 1 : parts(last.textContent).length] : [0, 0];
  };
  const span = (b, r) => [...pos(b, r.startContainer, r.startOffset), ...pos(b, r.endContainer, r.endOffset)];
  const current = (b) => { const s = getSelection(); return s.rangeCount && b.contains(s.anchorNode) ? span(b, s.getRangeAt(0)) : null; };
  // Behind: Rust has not drawn what was last sent, so the page's positions are old ones.
  const stale = (b) => (b.dataset.seq || "0") !== String(b.__k || 0);
  const post = (b, msg) => { const w = b.parentElement.querySelector(".c-wire"); if (w) { w.value = JSON.stringify(msg); w.dispatchEvent(new Event("input", { bubbles: true })); } };
  const send = (b, t, data, ranges, extra) => {
    const old = stale(b); b.__k = (b.__k || 0) + 1;
    post(b, Object.assign({ k: b.__k, t, data: data ?? null, ranges: old ? [] : ranges, sel: old ? null : current(b) }, extra));
  };
  // [paragraph, grapheme offset] back to a DOM point.
  const point = (b, n, off) => {
    const el = b.querySelector('[data-n="' + n + '"]');
    if (!el) return null;
    if (inert(el)) { const i = [...el.parentNode.childNodes].indexOf(el); return [el.parentNode, off > 0 ? i + 1 : i]; }
    const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    let left = off, t, last = null;
    while ((t = walk.nextNode())) {
      const p = parts(t.data);
      if (left <= p.length) return [t, p.slice(0, left).join("").length];
      left -= p.length; last = t;
    }
    return last ? [last, last.data.length] : [el, 0];
  };
  const place = (b) => {
    const c = (b.dataset.caret || "").split(":").map(Number), now = current(b);
    if (b.__composing || document.activeElement !== b || c.length !== 4 || (now && now.join(":") === c.join(":"))) return;
    const a = point(b, c[0], c[1]), z = point(b, c[2], c[3]);
    if (a && z) getSelection().setBaseAndExtent(a[0], a[1], z[0], z[1]);
  };
  // The / and @ menus and the bubble sit at the caret, which only the page can measure: below it
  // unless that runs off the window, and the bubble always above.
  const float = () => document.querySelectorAll(".c-edit [data-anchor]").forEach((f) => {
    const s = getSelection(); if (!s.rangeCount || !f.offsetParent) return;
    const g = s.getRangeAt(0), r = g.getClientRects()[0] || g.getBoundingClientRect(), box = f.offsetParent.getBoundingClientRect();
    const up = f.dataset.anchor === "above" || r.bottom + f.offsetHeight + 12 > innerHeight;
    f.style.left = Math.max(0, r.left - box.left) + "px";
    f.style.top = Math.max(0, up ? r.top - box.top - f.offsetHeight - 8 : r.bottom - box.top + 6) + "px";
  });
  new MutationObserver(() => { document.querySelectorAll(".c-body").forEach(place); float(); })
    .observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-seq", "data-caret"] });
  const on = (type, fn) => document.addEventListener(type, (e) => { const b = host(e.target); if (b) fn(e, b); }, true);
  on("beforeinput", (e, b) => {
    // During a composition the browser writes the paragraph and Rust waits for compositionend.
    if (e.isComposing || e.inputType === "insertCompositionText") return;
    e.preventDefault();
    const ranges = stale(b) || !e.getTargetRanges ? [] : e.getTargetRanges().map((r) => span(b, r));
    send(b, e.inputType, e.data ?? (e.dataTransfer ? e.dataTransfer.getData("text/plain") : null), ranges);
  });
  on("compositionstart", (e, b) => { send(b, "compositionstart", null, []); b.__composing = true; });
  on("compositionend", (e, b) => { b.__composing = false; send(b, "compositionend", e.data, []); });
  on("paste", (e, b) => {
    e.preventDefault(); const d = e.clipboardData, sel = current(b);
    send(b, "insertFromPaste", d.getData("text/plain"), sel ? [sel] : [], { html: d.getData("text/html") || null });
  });
  document.addEventListener("selectionchange", () => {
    const s = getSelection(), b = s.rangeCount && host(s.anchorNode), r = b && !b.__composing && !stale(b) && current(b);
    if (r && r.join(":") !== b.dataset.caret) post(b, { t: "select", ranges: [r] });
    float();
  });
  window.mailoCaret = { get: (b) => current(b),
    set: (b, n, off) => { const a = point(b, n, off); if (a) getSelection().setBaseAndExtent(a[0], a[1], a[0], a[1]); } };
})();
</script>"#;

/// One message from the glue, as it arrives.
#[derive(Debug, Deserialize)]
struct Raw {
    #[serde(default)]
    k: Option<u64>,
    t: String,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    ranges: Vec<[usize; 4]>,
    #[serde(default)]
    sel: Option<[usize; 4]>,
    #[serde(default)]
    html: Option<String>,
}

/// A message from the glue, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Heard {
    /// The selection moved, by a click or an arrow key. Not numbered: it changes no text.
    Select(Range),
    /// A `beforeinput` or a composition event, numbered by the glue.
    Input {
        seq: u64,
        event: InputEvent,
        /// The selection when it happened, or `None` when the page was behind.
        selection: Option<Range>,
    },
}

fn range([start_node, start_offset, end_node, end_offset]: [usize; 4]) -> Range {
    Range {
        start: Pos::new(start_node, start_offset),
        end: Pos::new(end_node, end_offset),
    }
    .ordered()
}

/// Parse what the glue wrote. `None` for anything that is not one of its messages.
pub(in crate::ui) fn parse(raw: &str) -> Option<Heard> {
    let raw: Raw = serde_json::from_str(raw).ok()?;
    if raw.t == "select" {
        return raw.ranges.first().copied().map(range).map(Heard::Select);
    }
    let mut event = InputEvent::new(
        raw.t,
        raw.data,
        raw.ranges.into_iter().map(range).collect(),
        false,
    );
    event.html = raw.html.filter(|html| !html.is_empty());
    Some(Heard::Input {
        seq: raw.k?,
        event,
        selection: raw.sel.map(range),
    })
}

/// Apply one message to the page. `at_ms` groups typing into undo steps.
pub(in crate::ui) fn hear(page: &mut Page, heard: Heard, at_ms: u64) {
    match heard {
        Heard::Select(range) => select(page, range),
        Heard::Input {
            seq,
            event,
            selection,
        } => {
            page.wire.seq = seq;
            input(page, event, selection, at_ms);
        }
    }
}

fn select(page: &mut Page, range: Range) {
    if page.wire.composing.is_some() {
        return;
    }
    if range.is_collapsed() {
        page.selection = None;
    } else {
        page.selection = Some(range);
    }
    if page.session.caret.pos != range.end {
        page.session.caret = Caret::at(range.end.node, range.end.offset);
    }
    float::after_move(page);
}

/// The IME rule, and the choice of which range an event acts on.
fn input(page: &mut Page, mut event: InputEvent, selection: Option<Range>, at_ms: u64) {
    let caret = page.session.caret.pos;
    let here = Range {
        start: caret,
        end: caret,
    };
    match event.input_type.as_str() {
        "compositionstart" => {
            page.wire.composing = Some(selection.unwrap_or(here));
            return;
        }
        "compositionend" => {
            let start = page.wire.composing.take().unwrap_or(here);
            event.ranges = vec![start];
            apply(page, &event, at_ms);
            // The IME wrote into this paragraph's DOM. A new key builds it again from the doc.
            let generation = page.fresh.entry(start.start.node).or_insert(0);
            *generation += 1;
            return;
        }
        _ if page.wire.composing.is_some() => return,
        _ => {}
    }
    if let Some(selection) = selection {
        // A collapsed Backspace or Delete is Rust's to resolve (an armed object, a heading that
        // gives up its kind first), not the browser's guess at one cluster.
        let resolves_itself = matches!(
            event.input_type.as_str(),
            "deleteContentBackward" | "deleteContentForward" | "deleteWordBackward"
        ) && selection.is_collapsed();
        if event.ranges.is_empty() || resolves_itself {
            event.ranges = vec![selection];
        }
    }
    apply(page, &event, at_ms);
}

fn apply(page: &mut Page, event: &InputEvent, at_ms: u64) {
    let before = page.session.doc.clone();
    let keeps_selection = event.input_type.starts_with("format");
    if let Err(why) = page.session.handle(event, at_ms) {
        page.notice = Some(format!("That edit did not apply: {why}"));
        return;
    }
    if !keeps_selection {
        page.selection = None;
    }
    if page.session.doc != before {
        page.touch();
    }
    float::after_input(page, event);
}

/// `data-caret`: the selection, else the caret, as `node:offset:node:offset`.
pub(in crate::ui) fn caret_attr(page: &Page) -> String {
    let range = page.selection.unwrap_or(Range {
        start: page.session.caret.pos,
        end: page.session.caret.pos,
    });
    format!(
        "{}:{}:{}:{}",
        range.start.node, range.start.offset, range.end.node, range.end.offset
    )
}
