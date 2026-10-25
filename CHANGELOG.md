# Changelog

All notable changes to mailo are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) from the first release.

## [0.1.0] - Unreleased

First public release.

### Accounts and protocols

- IMAP, POP3 (leave-on-server is a choice), JMAP, Microsoft Graph, and local-only accounts.
- Sending over SMTP, JMAP `EmailSubmission` or Graph `sendMail`.
- OAuth sign-in for Google and Microsoft accounts, with a client id of your own, since none
  ships (`MAILO_OAUTH_CLIENT_ID`, porter's `clients.toml`, or mailo's `oauth.json`); password
  sign-in with SASL for the rest, and server discovery from an address.
- Account secrets and signing keys are kept in the OS keychain.
- Profiles, each with its own store, accounts, keys and agent, and Spaces over them in the sidebar.
- Offline use through an outbox that replays queued actions.
- `mailo watch` for continuous fetching without a window.
- Push for Microsoft 365 accounts that read through Graph, through a relay you deploy on a
  Cloudflare account of your own (`relay/`): set its address as the account's Push relay in
  Settings, and new mail arrives at once. Without one, or with it unreachable, the account polls
  as before.

### Reading

- HTML mail is sanitised; remote images are blocked until allowed per message or per trusted
  sender; read receipts are only sent on request.
- Threaded conversations, folders and labels, stars, archive, pin, and undo for most actions.
- Reader that fits a wide message to the pane, folds older messages in a thread, and shows
  attachments as tiles with previews through anyview.
- BIMI sender logos, `List-Unsubscribe` (one-click and mailto), and Authentication-Results.
- Printing to PDF through the system print dialog.

### Security

- Remote fetches are HTTPS-only (an `http:` image is asked for over `https:`) and refuse
  loopback, private, link-local and `.local` addresses.
- Server responses and fetched bodies are read up to a fixed size.
- Data folders are 0700 and the database, blobs and saved attachments 0600; on macOS attachments
  carry the quarantine mark. A program or script is never opened from mailo; it is shown in its
  folder.
- An S/MIME body decrypted without integrity (AES-CBC) keeps remote content blocked and says it is
  not authenticated.

### Writing and scheduling

- Composer with signatures, templates, spell check, attachments, reply and forward.
- Several signatures to an address, written and chosen in Settings. A reply is signed with the
  default for replies under the quoted original, as Apple Mail does, unless
  `compose.signature_above_quote` is on; `mailo reply`, which used to sign above the quote, now
  signs under it too.
- `mailto:` links open a composer (to, cc, bcc, subject, body).
- Send later (`mailo send --at`) and unsend before the time, snooze, and follow-up reminders.

### Search and rules

- Full-text search over the local store, with server-side search where the protocol has it.
- Rules run in mailo; where the server has ManageSieve, rules and an away reply can be pushed to it.
- Tidy Up for the inbox.

### Encryption

- OpenPGP: key generation, import and export, sign, encrypt, verify, Autocrypt, WKD lookup.
- S/MIME: PKCS#12 import, sign, encrypt, decrypt, verify, and a trust list. AES-GCM and AES-CBC
  content encryption are read; triple DES is refused for mail dated after 2023.

### Contacts and calendar

- Local address book with vCard import and export, and CardDAV sync.
- Calendar invitations are shown in the reader, answered on request with an iTIP reply, and saved
  as `.ics`. There is no CalDAV sync.

### Import and export

- Import from mbox, Maildir and `.eml`; export of messages.

### Desktop integration

- macOS: `mailo.app` in a `.dmg`, registration for `mailto:` with an offer to become the default
  mail app, links handed to a running window, Dock reopen, notification centre.
- Linux: `.deb` and `.rpm`, a desktop entry, freedesktop notifications whose click opens the
  conversation with an activation token, hand-off to a running window over D-Bus, a systemd user
  unit for `mailo watch`, and a Flatpak manifest.
- Windows: an MSI registering `mailo` for `mailto:`, and toast notifications.
- A command line over the same store as the window: `mailo account`, `search`, `send`, `snooze`,
  `rules`, `pgp`, `smime`, `contacts`, `import`, `export` and more.

### Libraries

- `mail-domain`, `mail-mime`, `mail-proto` and `mail-pim` are sans-I/O library crates, enforced by
  `scripts/check-boundary.sh`. They are to be published as `mailo-domain`, `mailo-mime`,
  `mailo-proto` and `mailo-pim`.

### Known limitations

- The macOS and Windows packages are unsigned.
- Windows has no hand-off to a running window, and toasts carry Windows PowerShell's name.
- On macOS, a banner raised by `mailo watch` in a terminal opens nothing when clicked.
