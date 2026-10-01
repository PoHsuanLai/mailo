# The move to quire v0.2.2 and the settled desktop design

mailo now draws with quire v0.2.2. Every surface is built from quire's components; mailo's own
CSS is layout only. This page says what changed, what a person sees differently, what is still
open, and how to pick the work up. It is written for whoever owns mailo next.

## Pinned to v0.2.2

`mail-app` names quire by tag v0.2.2 (`ds`, `ds-settings`, `ds-blitz`, and `ds-lint` and
`ds-harness` as dev-dependencies). `ds-shell` is gone: `SpaceEditor` and `SpaceDot` moved into `ds`,
and the stylesheet and the lint kits are `ds::stylesheet()` and `ds::kits()`. `Cargo.lock` holds one
`ds`, one `ds-blitz`, one `blitz-dom`, one `blitz-kit` and one `dioxus-core`.

### What v0.2.1 said (kept for the record)

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
| Spaces | dots, grain, per-Space motion, a hand-made editor | quire's `SpaceEditor` and `SpaceDot`; the card's accent is quire's own "Accent" segment in that editor |
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

### The v0.2.2 pass (branch `design-quire-0.2`)

The stand-ins F198 listed are quire's now, and gone from mailo:

- **Pick lists.** `ui/menu`'s `Picker` and `Checklist` are deleted. Move to, the Import
  destination, the rule editor's label and folder pickers and the Labels picker are quire's
  `PickList<String>` under the control that opened them; the caller holds the query and narrows
  with mailo's matcher. A row that toggles a label is `AfterPick::KeepOpen`; "Create" is a
  `PaletteRow`. The page's Properties is a `Menu` of toggles (`Floating { toggles }`), which stays
  open on a pick.
- **Menu hints.** The snooze times and the composer's markdown triggers are `MenuItem::with_hint`
  (`Right::Hint` in mailo's item). Snooze reads "Later today 7:38 AM", "This weekend Sat 9:00 AM".
- **Toast actions.** Leaving a mailing list offers "Archive All" on the toast
  (`use_toasts().push_action`); the `Alert` for it is gone.
- **Tooltip.** The time in a thread row is `Tooltip { hover_key }` fed by the row's own hover hooks
  on the Tip profile.
- **Rows.** A folder row's actions are a `PopUpButton` of `PopUpKind::Overflow` in
  `Accessory::Slot` (a right-click opens the same items); its rename is `Row { edit }`; deleting a
  folder that holds mail asks in the row's own line (`Row { confirm: RowConfirm }`). `SectionHeader`
  takes `actions`. `PinTiles` draws the accounts, each `PinItem` with its own provider mark.
  `TodayTab` carries the row's hover hooks, so Today tabs have their hover card again. `ThreadRow`
  carries `data-hc` on its own `common`.
- **Fields and banners.** The vacation reply is a multi-line `TextField`. The empty state's command
  is a `RunTone::Code` run. Remote images, a read receipt, what stopped Send (sign and encrypt) and a
  message's bad or partly believed signature are `InlineBanner`s; a good or unverifiable signature
  is a `Label { severity }`. An invitation's facts are a `FactList`, its answer an `InlineBanner`.
  `FieldRow` wraps several controls, so the one lint exception (`.c-props .ds-field-row-control`) is
  deleted and `style/exceptions.rs` is empty. The format bar uses `Icon::Bold` and its kin.
  Tests build a pointer with `EditPointer::new(..).over(..).clicking(Clicks(2))`.
- **Spaces.** `SpaceLook` has its `grain` again; mailo reads and writes the `grain` key in
  `spaces.json`, and an old file without one takes the preset's own (`default_look`). The Space editor
  shows quire's Grain slider and its "Space colour" / "Your accent" segment.

**The window.** The grid is quire's `SplitView`: the sidebar pane (232 wide, 180 to 320, folds away
past 90) and the card, and in the card the list pane (400, 280 to 640) and the reader. The list's
header is quire's 52 px `Toolbar`; its centre holds the place's name, its status and the Group,
Properties, Sync and Compose buttons, because a `ToolbarItem` cannot hand over its element for a menu
to hang from. The sidebar's `EdgePeek` is the pane's body while pinned. Hidden, the pane folds and the
`EdgePeek` moves into a zero-width host at the window's edge (`.edge-host`), because a pane clips what
floats over it. quire's `Sidebar` is not used: it holds one `List`, and mailo's sidebar is account
tiles, several lists, Today tabs and the Space foot.

**Sheets.** Every sheet hangs from the card's top edge, not the window's (`.in-card`, a class of
mailo's on the sheet that sets `top`); the frame's 8 px inset no longer shows above them. The Space
editor draws one frame: quire's editor card is flattened inside the sheet (`.ed-look`).

**One selected row.** The sidebar selects a place or the Today tab the open thread came from, never
both (`Shell::place_selected`, `Shell::selected_tab`).

**Empty groups have no heading.** No pins, no "Pinned"; nothing in Today, no "Today".

**Copy.** A pass over every user-visible string: empty states, hints, sheet help, banners and errors
say less (FINDINGS F198 lists the notable changes).

### Keys: the standard Mac map

Command is the main modifier. A session that maps Command to Control (Toshy) delivers Ctrl, a Mac
delivers Meta, so ⌘ is exactly one of them and ⌃⌘ is both (`ui/chord.rs`).

| Before | Now | Does |
| --- | --- | --- |
| Ctrl T | ⌘K | the command menu (⌘K in its field closes it; in the composer with a selection it is the link) |
| Ctrl S | ⌃⌘S | hide or show the sidebar |
| Ctrl 1 to 9 | ⌘1 to ⌘9 | switch to Space n (design/27 says ⌃1 to 9; ⌘ is the choice made here) |
| Ctrl F | ⌘F | find in the open message |
| Ctrl P | ⌘P | print the open message |
| Ctrl Z | ⌘Z | undo |
| Ctrl Enter | ⌘Return | send (the composer already took either) |
| c | ⌘N (and c) | a new message, from anywhere, typing or not |
| Delete | ⌘⌫ (and Delete) | trash |

With ⌘ held, no bare letter is a shortcut any more: ⌘C used to compose. ⇧⌘Z (redo) is not undo.

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
- The list header, row strip and menus are quire's: Group is a check-marked menu; Properties is a
  menu of toggles that stays open; Labels, Move to and the Import and rule pickers are a filterable
  pick list under their button (Labels has a "Create" row). Arrow keys in the list open the row
  they land on. A row that leaves fades out and the rows below close the gap.
- Snooze and the composer's `/` menu show their time or trigger at the end of the row.
- Sheets hang from the card's top edge with Cancel on the left and the default on the right;
  Escape cancels, Return is the default. Deleting a folder that holds mail asks in the folder's
  own row; leaving a mailing list offers Archive All on the toast.
- The composer: an empty To marks the field invalid with words (no shake); the chip flash is
  gone; Send fades the page out; a format bar over a selection replaces the bubble.
- Today tabs have their hover card again. The empty Today and Pinned groups show no heading.
- The reader's empty state is quire's `EmptyState` ("No message selected"); with no account the
  list says "No account" and, under it, the command to run.
- Keys are the Mac's standard map (see "Keys" above).

## Tests

`cargo test --workspace --no-fail-fast`: 2202 passed, 0 failed, 58 ignored (the user's count before
this pass was 2196; v0.2.1 left 2197). `mail-app`'s unit tests are 642. The window's tests drive the
real headless window through `ds_harness` (`Harness::new`, `Driver::send(Input::..)`, `Query`);
`tests/support/drive.rs` names the dozen calls they make once, each one `Input` sent. The unit tests
that drive a `VirtualDom` get no mounted element, so a quire `PopUpButton` (which opens against its
own) cannot open there: they ask for the same menu by a right-click (`right_click` in
`ui/fixtures`). Fake data only, scratch directories, no network, no real config.

No golden or contrast check in mailo is pinned to the frame's stops: the only pins were the
`SpaceLook` literals in tests, which now carry `grain`. `tests/screens.rs` (ignored; set
`MAILO_SHOTS` to a directory) paints every view, light and dark, from a seeded scratch store:
the list, the empty list, the reader, the centre peek, the reader with its banners, the label
picker, the snooze menu, a toast with its action, the composer, the command pill's menu and the
Add Account and Space editor sheets.

## Open: what mailo still needs from quire

The v0.2.1 list is done (F198 has each line closed). What is left, from this pass:

- **`Sidebar` with several sections and a foot.** It takes one `List`; mailo's sidebar is account
  tiles, three lists, Today tabs and a foot, on the frame's colour rather than paper.
- **`SplitView` pane that can host an `EdgePeek`.** A folded pane clips the sidebar that floats out
  of it; mailo keeps the `EdgePeek` outside the pane while hidden.
- **`ToolbarItem` that hands over its element (or rect) when picked**, so a menu can hang from a
  toolbar button. The list header's menus are mailo's own buttons in the toolbar's centre.
- **`Sheet` that attaches to a pane**, not only to the window (`Attach::Within`). mailo moves the
  sheet's top to the card's edge by its own class; it is still centred across the window.
- **`SpaceEditor` without its own card** (a `framed: false`), for use inside a `Sheet`. mailo
  flattens it with a class of its own.
- **An accessible name for an image-only `SegmentedControl` segment.** The format bar's segments have
  an icon and an empty label.
- **`ds::prelude` names** for `InlineBanner`, `FactList` and `Fact`, `HeaderAction`, `RunTone` and
  `Common`: the catalogue lists them, the prelude does not export them.
- **A test seam for `PopUpButton`.** It draws no menu until its element has mounted, which a
  renderer-less document never reports; `Floating` falls back to the window's corner.
- **`⌃` in the faces.** U+2303 draws as a caret in the command menu's key column.

## Open: design work not done

- No menu bar. The Mac's App, File, Edit and View menus wait on quire's `MenuBar` transport
  (design/30 Deferred 2); the format bar stays until a Format menu exists.
- The command pill, the list search and any "one input" work are deferred by decision: untouched.
- No spoof banner in the reader: the sender card's flag is quire's `HoverCardPart::Flag`, and the
  reader shows nothing for a borrowed name.
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
