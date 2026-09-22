//! The shell's stylesheet.
//!
//! Its own file because it is one long constant and nothing else: at a thousand lines `ui.rs`
//! was over the ~400 that `CONVENTIONS.md` §8 asks for, and fifty lines of CSS in the middle of
//! the component tree helped nobody find either.

pub(super) const STYLE: &str = r#"
:root { color-scheme: light dark; --edge: color-mix(in oklab, currentColor 15%, transparent); }
* { box-sizing: border-box; }
body { margin: 0; font: 14px/1.5 system-ui, sans-serif; }
/* Focusable for the keyboard, without a ring around the entire window. */
.app:focus, .app:focus-visible { outline: none; }
.app { display: grid; grid-template-columns: 180px minmax(340px, 32%) minmax(0, 1fr); height: 100vh; }
.places { display: flex; flex-direction: column; gap: 2px; padding: 12px; border-right: 1px solid var(--edge); }
.place { text-align: left; padding: 6px 10px; border: 0; border-radius: 6px; background: none; color: inherit; font: inherit; cursor: pointer; }
.place:hover { background: var(--edge); }
.place.on { background: var(--edge); font-weight: 600; }
/* Set apart from the places above it: those choose what to look at, this one writes. */
.place.compose { margin-top: 10px; font-weight: 600; border: 1px solid var(--edge); }
/* The label menu, under the row whose button opened it. */
.labels { grid-column: 1 / -1; display: flex; flex-wrap: wrap; gap: 6px; padding: 8px 4px 2px; }
.labels .label { padding: 3px 9px; border: 1px solid var(--edge); border-radius: 999px; background: none; color: inherit; font: inherit; font-size: 0.85em; cursor: pointer; }
.labels .label:hover { background: var(--edge); }
.labels .label.on { background: var(--edge); font-weight: 600; }
.list { overflow-y: auto; border-right: 1px solid var(--edge); }
.search { width: 100%; padding: 10px 12px; border: 0; border-bottom: 1px solid var(--edge); background: none; color: inherit; font: inherit; }
.row { display: grid; grid-template-columns: minmax(0, 7fr) minmax(0, 13fr) auto; gap: 10px; align-items: center; padding: 10px 12px; border-bottom: 1px solid var(--edge); cursor: pointer; position: relative; }
.row:hover { background: var(--edge); }
.row.unread .subject, .row.unread .who { font-weight: 650; }
.who, .subject { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.when { opacity: .6; font-variant-numeric: tabular-nums; }
.hover { display: none; position: absolute; right: 8px; gap: 4px; }
.row:hover .hover { display: flex; }
.hover button { font: inherit; font-size: 12px; padding: 2px 8px; border: 1px solid var(--edge); border-radius: 999px; background: Canvas; color: inherit; cursor: pointer; }
.reader { overflow-y: auto; padding: 20px 24px; }
.reader h1 { font-size: 20px; margin: 0 0 12px; }
article { border-top: 1px solid var(--edge); padding: 14px 0; }
article header { display: flex; gap: 6px; align-items: baseline; flex-wrap: wrap; margin-bottom: 8px; }
article time { margin-left: auto; opacity: .6; }
.text { white-space: pre-wrap; word-wrap: break-word; font: inherit; margin: 0; }
.html { width: 100%; min-height: 320px; border: 0; }
.attachments { list-style: none; margin: 0 0 10px; padding: 0; display: flex; flex-wrap: wrap; gap: 6px; }
.attachments li { display: flex; align-items: baseline; gap: 6px; padding: 4px 10px; border: 1px solid var(--edge); border-radius: 999px; font-size: 12px; }
.attachments .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 22em; }
.attachments .size { opacity: .6; font-variant-numeric: tabular-nums; }
.pending, .empty { opacity: .6; font-style: italic; }
/* The first-run message names a command, which has to survive its own line breaks. */
.empty { padding: 12px 12px 0; margin: 0; }
.command { margin: 6px 12px 0; padding: 6px 8px; border: 1px solid var(--edge); border-radius: 6px; font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
.composer { border-top: 2px solid var(--edge); margin-top: 16px; padding-top: 12px; display: flex; flex-direction: column; gap: 8px; }
.composer-head { display: flex; align-items: baseline; gap: 10px; }
.composer-head strong { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.composer label { display: grid; grid-template-columns: 70px 1fr; align-items: center; gap: 8px; font-size: 12px; opacity: .75; }
.composer input { font: inherit; padding: 6px 8px; border: 1px solid var(--edge); border-radius: 6px; background: Canvas; color: inherit; }
.composer-body { font: inherit; min-height: 180px; padding: 8px; border: 1px solid var(--edge); border-radius: 6px; background: Canvas; color: inherit; resize: vertical; }
.composer-actions { display: flex; align-items: center; gap: 8px; }
.composer-actions button { font: inherit; padding: 6px 14px; border: 1px solid var(--edge); border-radius: 6px; background: Canvas; color: inherit; cursor: pointer; }
.composer-actions .primary { font-weight: 600; }
.ghost.danger { border-color: color-mix(in oklab, #c0392b 60%, var(--edge)); color: #c0392b; font-weight: 600; }
.ghost { font: inherit; font-size: 12px; padding: 2px 8px; border: 1px solid var(--edge); border-radius: 999px; background: none; color: inherit; cursor: pointer; }
.notice { margin: 0; padding: 6px 8px; border-radius: 6px; background: var(--edge); font-size: 13px; }
.hint { font-size: 12px; opacity: .6; }
.place { display: flex; align-items: center; gap: 8px; }
.badge { margin-left: auto; font-size: 11px; font-variant-numeric: tabular-nums; opacity: .7; }
.place.on .badge { opacity: 1; }
.spacer { flex: 1; }
.sync { text-align: center; border: 1px solid var(--edge); }
.sync:disabled { opacity: .6; cursor: default; }
.sync-note { margin: 8px 2px 0; font-size: 11px; opacity: .7; white-space: pre-wrap; word-break: break-word; }
.sync-note.bad { opacity: .95; font-weight: 600; }
.more { display: block; width: calc(100% - 24px); margin: 10px 12px; padding: 8px; font: inherit; border: 1px solid var(--edge); border-radius: 6px; background: none; color: inherit; cursor: pointer; }
.images { font: inherit; font-size: 12px; padding: 4px 10px; border: 1px solid var(--edge); border-radius: 999px; background: none; color: inherit; cursor: pointer; margin-bottom: 8px; }
"#;
