# mailo's design language

This file states what mailo looks like and why. It builds on quire, the design system mailo draws with (`quire/design/00-PRINCIPLES.md`, `34-MODERN-LOOK.md`, `09-ARC-HEURISTICS.md`). quire decides the tokens: type, colour, radii, control sizes, motion. This file decides how mailo puts those to use.

When a screen and this file disagree, the screen is wrong. When this file and quire disagree on a value, quire wins.

## 0. Start from the person, then fold the spec in

Email's standards (IMAP, SMTP, MIME, JMAP, Graph, OpenPGP, DMARC, MDN, Sieve) have hundreds of concepts. A person doing their mail has about eight jobs. mailo is designed from the jobs. Each concept from the standards is then given the smallest form a person can meet it in, which is usually none.

### 0.1 The jobs

| Job | What the person sees |
| --- | --- |
| See what is new and what needs me | Inbox: conversations, newest first, unread marked |
| Read a conversation | The reader: the messages, latest open, older ones folded |
| Answer it or pass it on | Reply in place; Forward |
| Write something new | The composer: To, Subject, the message |
| Deal with it | Done (Archive), Not Now (Snooze), Not Wanted (Delete, Junk), Keep Handy (Star, Pin) |
| Find something | Search: one field, results as a list |
| Trust it's working | Nothing, until it isn't, then one line that says what to do |
| Set it up once | Add an account by signing in; everything else has a working default |

A screen shows what its job needs. Anything else is one step away (⋯, the right-click menu, a key) or happens automatically.

### 0.2 How each email concept reaches the person

Every concept falls into one of four tiers. When a concept isn't in this table, it starts at **Never**.

- **Never**: the person never meets it. mailo handles it.
- **On trouble**: shown only when it goes wrong, as one plain sentence and at most one action.
- **One step**: in ⋯, the right-click menu, Send ▾ or Settings, never drawn on the screen at rest.
- **Shown**: on the screen at rest.

| Concept (standard) | Tier | Its form |
| --- | --- | --- |
| Protocols: IMAP, POP3, JMAP, Graph, SMTP | Never | Named only on the account's details page in Settings |
| Server settings, ports, auth methods | Never | Found by sign-in and autodiscovery; details page when that fails |
| Sync, IDLE, UIDs, polling | On trouble | The sidebar foot's mark, only while syncing is failing |
| Sign-in expired (OAuth refresh, password) | On trouble | The sign-in prompt on launch; one Sign In button in the pane |
| Server unreachable | On trouble | The pane's title and one line; it retries by itself |
| Threads (References, In-Reply-To) | Shown | Conversations, always; no setting |
| \Seen | Shown | One unread mark (a dot) |
| \Flagged | Shown | Star |
| Pin (mailo's own) | Shown | Beside Star: the same slot on a row, adjacent in menus |
| Special-use folders (Sent, Drafts, Trash, Junk, Archive) | Shown | Archive shown as Done; Sent, Drafts, Junk and Trash one step away in ⌘K and the list title's menu (§0.6); emptying in ⋯ |
| Other folders, Gmail labels | One step | One collapsed Folders list; a POP3 account has none |
| Snooze, Remind (mailo's own) | One step | Snooze in the row and reader menus; Remind only under Send ▾ |
| Cc, Bcc | One step | Shown in the composer once added (key or the To row's end) |
| From identities | One step | A From row only with more than one address |
| Scheduled send | One step | Send ▾ › Send Later; a Scheduled place only while one is pending |
| Read receipts (MDN) | One step | Request: Send ▾. An incoming request is answered by a setting, not a banner |
| OpenPGP, S/MIME | Never → On trouble | Signed and encrypted automatically when keys exist. One lock glyph says so; a popover explains. Choosing the scheme is ⋯ › Security |
| SPF, DKIM, DMARC | On trouble | Nothing when it passes; one line "This may not be from {sender}" when it fails |
| Remote content, tracking pixels | On trouble | Blocked by default; one quiet Load Images line per message that has any |
| MIME structure, charsets, HTML vs plain | Never | ⋯ › Show Plain Text, ⋯ › Download |
| Attachments | Shown | Tiles at the message's end |
| List-Unsubscribe | One step | ⋯ › Unsubscribe |
| Rules (Sieve, local) | One step | Settings › Rules |
| Drafts saving | Never | Closing keeps the draft; Delete Draft is in ⋯, with undo |
| Offline copies | One step | A switch on the account's details page |
| Spaces | Never with one | Drawn only once a second Space exists |
| Saved views | One step | Search › Save as View; listed under Places only once saved |

### 0.3 The screen at rest

A new person with one Google account sees only this:

- **Sidebar:** Inbox, Waiting, Snoozed, Done, Kept (and Scheduled while a send waits); Favourites once something is pinned; Labels and Folders collapsed; nothing else (§0.6).
- **List:** the place's title; New Message, Search, ⋯; the rows.
- **Reader:** Reply, Archive, Delete, ⋯; the subject; the conversation.
- **Composer:** To, Subject, the message; Attach, ⋯, Send ▾.

Each added element must name a job from 0.1 and a tier from 0.2. If it can't, it is removed.

### 0.4 Decisions

The owner's decisions (2026-10-14):

- The reader toolbar is Reply, Archive, Delete and ⋯.
- Pin and Star both stay; they share a row's mark slot and sit next to each other in menus.
- Remind lives only under Send ▾.
- The composer's subject is 20/700, as the reader's is.

### 0.5 What we take from Arc

We take Arc's ideas, not its look. There is no gradient, no frosted glass and no colour across the window frame. Nothing changes by time alone: mailo never ages out, expires or moves anything on its own, mail or shortcuts. Time acts only where the person set it, as in Snooze, a reminder or Send Later. Clearing up happens only when the person asks: a **Tidy Up…** tool shows exactly which old read Inbox conversations it would archive, and runs only on their say-so, with undo.

Each idea **replaces** something mailo already has. None adds a surface, and the window ends with fewer elements than it had.

| Arc idea | What it replaces in mailo | Net effect |
| --- | --- | --- |
| **Where you were is one keystroke away.** Arc keeps the tabs you opened under a divider and archives them by itself. mailo moves nothing by time, so a list like that would only grow; instead ⌘K, with nothing typed, lists Recent: the conversations opened in this Space and the drafts put aside, newest first. | The foot menu's Today list, a Today section in the sidebar, the History and Pinned places, and the account tile row. | Nothing at rest in the sidebar; History stays one step away in ⌘K. |
| **Favourites.** Pinned people or conversations as a short row of tiles, badged only when unread. | The account tile with its "9" badge and "+". Add Account moves to the Space's menu. | The tile row is shown only once something is pinned. Pin and Star stay side by side, per §0.4. |
| **⌘K goes anywhere.** One field: empty, it shows Recent (where you were), places and verbs with their keys; typed, it searches mail, and results fill the list. | The list toolbar's separate Search field and mode, the Connections sheet's ⌘K entry, and "Go to" commands repeated elsewhere. | One field for search and navigation. The toolbar's Search icon opens ⌘K. |
| **Peek is the one preview.** Attachments, inline images, a forwarded message and a link's destination card all open in one overlay; Esc closes it, ⌘↩ opens the item in its app. | The attachment viewer pane, the link hover card and the trust pill's details. | One preview grammar instead of three. |
| **One window to write in.** New Message, a `mailto:` link, a forward or a draft opens the composer in the reader column; replies stay inline under the thread. | The small composer window and its "Open in Main Window", and the Focus button. | A second window to find and manage cost more than it gave; Focus goes. |
| **Spaces are contexts.** A Space holds any accounts (Work: work Gmail plus Exchange). With one Space there is no Space UI at all. | One Space per account, the "Space 1" head and the lone dot. | Space UI only once a second Space exists. |
| **Chrome recedes.** At rest the reader shows Reply, Archive, Delete and ⋯ (§0.4). The sidebar has no tint gradient. A selection is accent while its list has the keyboard and grey otherwise, as on the Mac. | The reader's other toolbar icons and the sidebar's faint gradient. | Fewer marks. |

Not taken:
- auto-tidy, gradients, glass, a coloured frame;
- Boosts, Easels, playful copy, bouncy motion;
- an icon-rail sidebar;
- Spaces always shown;
- close buttons on hover.

### 0.6 Folding mail's complexity

Mail's standards name where a message is stored: Inbox, Archive, Sent, Drafts, Junk, Trash, and any number of folders and labels. The person thinks about what a conversation needs from them. The sidebar shows the second and keeps the first one step away.

| Sidebar | What it holds | Stored as |
| --- | --- | --- |
| **Inbox** | What still needs a look | the Inbox |
| **Waiting** | Sent mail the person asked to be reminded about | a reminder on the conversation |
| **Snoozed** | Conversations put off until a time the person chose | a snooze on the conversation |
| **Done** | Everything dealt with | Archive (or All Mail on Gmail) |
| **Kept** | Starred and pinned conversations, side by side | the server's flag, mailo's pin |
| **Scheduled** | Messages waiting for their send time; the row is drawn only while one waits | a send time on the draft |

Labels and Folders follow, collapsed. Sent, Drafts, Junk and Trash are not in the sidebar: they are reached through ⌘K and the list title's menu, and a conversation's menu says where it is ("Move to Inbox" in Done, "Not Spam" in Junk).

Rules for the fold:

- One action, one word: Archive moves to Done, Delete moves to Trash, and the word on every menu is the one for the side the conversation is offered, never the side it is on.
- Nothing moves by time alone. Where the person was is ⌘K's Recent, not a sidebar list that grows. **Tidy Up…** (in the sidebar's foot menu) is the only bulk clean-up: it lists the read Inbox conversations older than an age the person picks (starred, pinned, snoozed and waiting ones stay), archives exactly those only when asked, and can be undone.
- Server work is never a step the person takes. Rules are pushed to the server when saved, and only a failure is shown.
- A protocol word (IMAP, JMAP, Graph, POP) is shown only where the person sets up a server by hand.

## 1. The stance

mailo is a quiet Mac mail app. The mail is the content; everything else recedes.

A screen earns each element on it. Before adding one, ask what the person loses without it. If the answer is "nothing most of the time", it does not go on the screen. It goes into a menu, onto a key, or appears only in the state that needs it.

**Simple beats complete.** A screen that shows every option is easy to build and hard to use. mailo shows the common path and puts the rest one step away.

## 2. Subtraction rules

These decide most open questions. Apply them in order.

1. **Show a default by showing nothing.** An option set to its default is not drawn. The composer has no "Sends: Right away", "Protection: None" or "Remind me: Don't remind me" rows. The row appears only once the option is changed, and stays only while it differs from the default.
2. **Say a thing once.** If the place, the row or the context already says it, a label does not repeat it. For example:
   - A provider or account mark on a row only appears when more than one account is in view.
   - A section header does not restate the place it heads.
   - A tooltip does not restate a visible label.
3. **One home per action.**
   - Every action has one primary place: a toolbar button, a menu item or a key.
   - It is reachable from the right-click menu and the ⋯ menu as well, but it is not drawn twice on the same screen.
4. **A toolbar holds at most three actions plus ⋯.** Pick the three this pane is for. The rest go in ⋯ and the right-click menu, and keep their keys.
5. **Status is transient.** A save, a send or a sync is said once, then the screen returns to rest. Persistent words on screen ("Draft · saving…", "Downloading your mail…") are not allowed. Lasting state lives in one place, such as the sidebar foot's sync mark, as an icon with a tip.
6. **No decoration that needs a second look.** Remove any element whose only purpose is visual: a badge on a badge, a ring around an avatar, a coloured chip that states nothing new.
7. **Destructive actions never sit on hover.** They live in the right-click menu, the ⋯ menu or on a key, and come with undo. (Existing rule; see the no-destructive-hover memory.)

## 3. Layout

- **One leading edge per pane.** Every line of text in a pane starts on the same x, the pane's content inset (`--s-18`). Title, metadata and body all align on it. Nothing is indented to "look nested" unless it is nested.
- **Label columns align.** When a form has labels, each label starts on the pane's leading edge, in one column as wide as the widest label, and every value starts at the same x past it. (A right-aligned column indents short labels such as "To" off the edge the subject and body share.) Labels have no colons. ("From" not "From:".) Where a field explains itself through its placeholder, it has no label at all.
- **One ground per pane.** A pane is one surface. No grey band behind a header inside content: the reader's message header and the composer's subject sit on the same ground as the text below them. A hairline (`--hair`) may separate regions; a change of fill does not.
- **Separators only between regions,** never between items that spacing already separates. A list of rows needs no rule between them.
- **Bars are one height.** The toolbar of the list, the reader and the composer share one height and one baseline, so the three panes' titles and buttons line up across the window.
- **Rows have one pitch.** A list row is the height of what it shows. A row with no snippet is shorter; it does not leave a gap where the snippet would be.

## 4. Type

These are quire's styles. mailo uses only these:

| Role | Style |
| --- | --- |
| Pane title | Title, 16/700 |
| Reader subject | 20/700, display face |
| Composer subject | 20/700, the same as the reader: what you write looks like what you read |
| Names | Headline |
| Message text, read or written | 14, line height 1.65 |
| Metadata (dates, addresses, counts) | Footnote, `--ink-faint` |

There are no raw sizes. There is no third weight between regular and bold.

A screen uses at most three text styles above the body. If it needs a fourth, it has too much on it.

## 5. Colour

These are quire's colours. mailo adds nothing:

- Chrome is greyscale.
- The accent marks the one primary action on a screen and the selection, nothing else.
- Hue states a fact: unread, flagged, destructive, which account. It never decorates.
- Ink ladder: content `--ink`, labels `--ink-soft`, metadata `--ink-faint`.
- State styling (hover, pressed, disabled) is quire's. mailo does not invent opacities.

## 6. Controls

- **Buttons:**
  - Toolbar buttons are Large, icon-only, with a tip.
  - Content buttons are Regular; dense rows use Small.
  - There is one filled (primary) button per screen; everything else is plain.
  (Existing rule; see button-size-rule.)
- **Icons** match the text beside them: 14 in rows, 16 in content, toolbar size in toolbars.
- **Menus:**
  - Items are verbs in Title Case.
  - A menu has at most about seven items before it needs a submenu or a rule.
  - Submenus group related options; they are not a dumping ground.
- **Every icon-only control has a tip**, saying its name and its key.
- **Fields:**
  - Plain fields are for the composer.
  - Bezeled fields are for settings, sheets and popovers.
  - One row height per kind.

## 7. Copy

- Title Case for controls, menu items, tips and window titles. Sentence case for sentences.
- Short: a header is one or two words. A button is a verb or verb + noun.
- Plain words, no jargon: "Scheduled", not "Waiting to be sent". Protocol names (IMAP, JMAP, Graph) appear only in Settings › Accounts › details.
- All text lives in `crates/mail-app/src/ui/copy/`, by kind.

## 8. Motion

quire's motion, nothing else:

- Every animation names a state change.
- Springs only where a hand touched.
- No motion on first show.

## 9. How a screen is reviewed

A design review of any screen asks, in order:

1. What can be removed? (§2: defaults, repeats, decoration, persistent status.)
2. Does every line start on the pane's edge, and does every label column align? (§3)
3. Is there one ground, and are bars at one height across the panes? (§3)
4. Three text styles or fewer, from §4's table? (§4)
5. Does hue appear only as a claim? (§5)
6. At most three toolbar actions plus ⋯, one primary, tips on every icon? (§6)
7. Is the copy short, in the right case, and in `ui/copy`? (§7)

Each finding names the rule it breaks, the file and line, and the change. Prefer removing to restyling.
