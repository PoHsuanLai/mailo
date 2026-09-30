# The move to quire v0.2.1 and the settled desktop design

mailo now draws with quire v0.2.1. Every surface is built from quire's components; mailo's own
CSS is layout only. This page says what changed, what a person sees differently, what is still
open, and how to pick the work up. It is written for whoever owns mailo next.

## Pinned to v0.2.1

mailo names quire by tag v0.2.1. v0.2.0 could not be fetched by tag (its manifest named `blitz-kit`
by a relative path); v0.2.1 names it by `git` and `rev` (public repo
github.com/PoHsuanLai/blitz-kit), so a plain clone builds with no local `[patch]` and no sibling
checkouts. `Cargo.lock` holds one `blitz-dom`, one `blitz-kit` and one `dioxus-core`.

## What changed, by surface

One commit carries the version move (`mailo draws with quire v0.2.0 ...`): quire 0.2 renamed,
merged or deleted most of the names `mail-app` imported (design/30 Part 4), so no smaller step
compiles. What follows it is small.

| Surface | Before | Now |
| --- | --- | --- |
| Window root | `Ds` with grain, the Postmark look, mailo's own motion levels | `Ds` in the Window material on the Space colour; Inter only; the person's `style.css` drawn live after quire's and mailo's sheets; mailo's sheet in the `app` layer through `AppStyle` |
| Sidebar | hand-built items, account tiles, favicons, section headers, an edge strip | `EdgePeek` over a source-list `List` of `Row`s; accounts are `PinTile`s; Today is `TodayTabs`; headers are `SectionHeader`; folders are `Row`s with an outline, a count `Badge` and a row action; the command pill is `CommandPill` |
| Spaces | dots, grain, per-Space motion, a hand-made editor | quire's `SpaceEditor` and `SpaceDot` (ds-shell); the card's accent is quire's own "Accent" segment in that editor |
| List | mailo's rows, strip, toast and row menus | `List` of `ThreadRow`s, `HoverStrip` actions, `Roster` enter and leave, `SectionHeader` bands, `TextField` Search, `EmptyState` |
| Reader | hand-built avatar, tabs, attachment list, find pill | `Avatar`, `Label`, a `SegmentedControl` for Reader or Original, attachments as `Row`s, find as `TextField` Search, tools as toolbar `Button`s; the floating reader is quire's `Peek` |
| Composer | bespoke property rows, selection bubble, chip flash and shake | `FieldRow`s, `Chip`s, `TextField`, `EditSurface`, `PopUpButton`, `SegmentedControl`; quire's `SendPill` for the countdown |
| Command palette, pickers | a hand-rolled palette and rich menus | `CommandPalette` over the window (Ctrl T; Move to, Import into, the rule editor's pickers), `Menu` for short command lists, a `Popover` of `Checkbox`es for toggled sets (Labels, Properties) |
| Hover cards, toasts | mailo's own | `HoverCard` parts and `LinkPill`; `use_toasts` with Undo |
| Sheets, alerts | a dialog box and a scrim per sheet | `Sheet` hung from the window's top with Cancel (Escape) and the default button rightmost (Return); `Alert` for folder delete and leaving a list |

mailo's own CSS went from 1128 lines to 547, in the `app` layer, and passes quire's Strict lint
(`our_stylesheet_lints_clean`) with one exception that has its reason in `style/exceptions.rs`.
The markup lint (no raw controls, no unstyled class, no literal colour in a `style`) runs over the
frame and over every sheet and card (`style::tests::markup_offences`).

### Deleted mailo-local UI

`ui/field.rs` (quire's `TextField`), the toast component, menu rows and their keys, sidebar
items, account and Today tiles, the edge strip and peek scrim, the selection-bubble menu, the
hand-drawn progress bar, the `.fmenu`, `.files-wrap`, `.book-wrap`, `.rules-wrap` and `.keys-wrap`
dialog wrappers, `ul.attachments`, `.reader-av`, and every keyframe quire no longer has
(`calm.rs`, the seal pop, slide, gulp, landing and star pop). Grain and per-Space motion are gone
from the model; an old `spaces.json` that still carries them loads and drops them on the next
write.

## User-visible changes

- No grain anywhere. Inter everywhere, whatever `appearance.typeface` says (quire's editorial
  faces are not mail's voice). The accent is one of the Mac's eight, Blue by default; mailo reads it from
  quire's `appearance.toml` and never writes it (design/22-SETTINGS.md section 9.5): choose it in
  the desktop's Settings app or the control centre. Each Space still picks whether its card keeps
  that accent or borrows the Space's hue (quire's editor, "Accent" segment, saved in the Space's
  look; `CardAccent::Chosen` by default). quire's labels are "The accent" and "A hint of the
  Space"; "Your accent" and "Space colour" are a quire request.
  The old Postmark accent and `appearance.json` accents are not read.
- Motion is macOS's: menus open at once, rows and toasts slide and fade, springs only on contact.
  There is no Calm or Extra and no per-Space motion; the desktop's reduced-motion preference is
  the only switch. Switching Space cross-fades the colour; the sidebar contents no longer slide.
- The sidebar is hidden and peeked by `EdgePeek`: point at the left edge to float it, click to pin.
- The list header, row strip and menus are quire's: Group is a check-marked menu; Properties and
  Labels are popovers of checkboxes (Labels has a filter and a "Create" row); Move to and the
  Import and rule pickers are the command palette's panel. Arrow keys in the list open the row
  they land on. A row that leaves fades out and the rows below close the gap.
- Snooze times show the time in the item's name; the Mac's menu has no second line.
- Sheets hang from the window's top edge with Cancel on the left and the default on the right;
  Escape cancels, Return is the default. Deleting a folder that holds mail and leaving a mailing
  list ask with an `Alert`.
- The composer: an empty To marks the field invalid with words (no shake); the chip flash is
  gone; Send fades the page out; a format bar over a selection replaces the bubble; the `/` and
  `@` menus show no help or markdown hints; the vacation reply's body is one line until quire has
  a multi-line field.
- Today tabs lost their hover previews; the folder menu's help lines are gone.
- The reader's empty state is quire's `EmptyState`; the first-run "No account yet" says the same
  words in its body line.
- Keys are unchanged (see "Open" below).

## Tests

`cargo test --workspace`: 2213 passed, 0 failed, 58 ignored before (master `da70e60`); 2197
passed, 0 failed, 58 ignored after. `mail-app`'s unit tests went from 653 to 637: the tests of
mailo's own toast, row motion (gulp, landing, star pop, count bump), selection bubble, chip flash
and sidebar slide went with what they tested; the peek panel and the
typeface pin have new ones, and the Space editor's test asserts it offers no global accent picker. The window's tests drive the real headless window through
`ds_harness` (`Harness::new`, `Driver::send(Input::..)`, `Query`); `tests/support/drive.rs` names
the dozen calls they make (`click`, `key`, `chord`, ...) once, each one `Input` sent. Fake data
only, scratch directories, no network, no real config: `tests/screens.rs` (ignored; set
`MAILO_SHOTS` to a directory) paints the main window, menu, composer, command palette and two
sheets, light and dark, from a seeded scratch store.

## Open: what mailo still needs from quire

Each line is a component and why. They are also in `FINDINGS.md` (F160).

- **blitz-kit as a git dependency** in quire's manifest (see the top of this page).
- **Filterable pick list.** A `Menu` or `Popover` with a `TextField` Search and `Row`s that can
  stay open on a pick. mailo composes `CommandPalette` (closes on pick) and a `Popover` of
  `Checkbox`es (`ui/menu`: `Picker`, `Checklist`).
- **`Menu` that stays open on a pick** (toggle items), and **a second line or trailing hint** on
  `MenuItem` (Snooze times, folder help, the composer's markdown triggers).
- **`PinItem` per-tile mark.** `PinTiles` takes one provider mark for all tiles, so accounts are
  `PinTile`s in mailo's grid.
- **`TodayTabs` per-tab `Common`/hooks** for a hover card; **`RowAction` with `Common`/mounted**
  so a row's menu hangs from its button; **`Row` inline-edit slot** (rename in place);
  **`SectionHeader` with more than one action**; **`ThreadRow` `Common` data** on the inner row.
- **Tooltip keyed by a hook** (a thread row's time); **toast action other than Undo**.
- **Inline banner** in a pane's flow (remote images, read receipt, an answered invitation), **a
  status tone for `Label`** (signature results), **a read-only label and value list** (invitation
  facts), **an overflow for a row's actions**, **a row-scoped inline confirmation**.
- **`FieldRow` whose control cell wraps** several controls (the one lint exception).
- **Multi-line `TextField`** and **`EmptyState` with a code run** in its description.
- **Bold, Italic, Underline, Strike, Code glyphs** (the format bar uses letters) and **a public
  `Clicks`/`EditPointer` constructor** for consumers' tests.
- **`SpaceEditor` and `SpaceDot` in `ds`, not `ds-shell`:** mailo now ships the whole shell
  sheet for two components.

## Open: design work not done

- The window is still a CSS grid (232 px sidebar, list, reader). quire's `SplitView`, `Sidebar`
  and `Toolbar` (design/30 2.7) would give resizable panes, collapse by spring and the 52 px
  toolbar; the list header is `Label`s and `Button`s today.
- No menu bar. The Mac's App, File, Edit and View menus wait on quire's `MenuBar` transport
  (design/30 Deferred 2); the format bar stays until a Format menu exists.
- Keys are the prototype's (Ctrl T, Ctrl S, Ctrl F, Ctrl P). design/27 6.2 reserves the standard
  map (Cmd K, Ctrl Cmd S and so on); moving them is a decision, and `view::shortcut` and
  `ui/app.rs`'s `on_key` are the two places.
- Two rows can read as selected at once (the place and the open Today tab).
- The sweeps that need a real Mac or a real compositor (first-click rules, the traffic lights,
  blur behind the sidebar) are unchecked here.

## Continuing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # check the exit code, not a piped tail
./scripts/check-boundary.sh
cargo deny check licenses

MAILO_SHOTS=$PWD/shots cargo test -p mail-app --test screens -- --ignored   # the pictures
```

A new rule in quire's lint reaches mailo at the next tag bump: bump, run the two style tests
(`cargo test -p mail-app --lib ui::style`), fix the named selectors. CONVENTIONS.md section 10
still binds; the harness section now says `ds_harness`.
