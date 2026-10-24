# Security policy

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting on this repository:
<https://github.com/PoHsuanLai/mailo/security/advisories/new> (the Security tab, then "Report a
vulnerability"). Please do not open a public issue or pull request for a vulnerability.

Include the version, the platform, and what you did to reach the problem. A malformed message that
crashes or misrenders is welcome as a file.

## Supported versions

| Version | Supported |
| --- | --- |
| 0.1.x | yes |

Nothing older exists. Fixes land on the latest 0.1.x.

## Security model

mailo reads mail from servers it does not control, so anything in a message is untrusted. What the
code does about that, as of 0.1.0:

- **Remote content is blocked by default.** A message's remote images are not fetched until you
  allow them for that message or for a trusted sender (`RemoteImages::Blocked` is the default in
  `mail-mime`). When allowed, mailo fetches them itself, with no cookies and no `Referer`, a size
  cap and a timeout; documents do not reach the network on their own. Read receipts are never sent
  automatically.
- **HTML is sanitised.** Message HTML goes through an allow-list sanitiser (ammonia) in
  `mail-mime`. Links are limited to `http`, `https`, `mailto` and `cid` (a part of the same
  message), and CSS that could reach the
  network is scrubbed before a sender's styles are kept for the reader's frame.
- **mailo's own fetches are HTTPS-only, with certificates checked:** server discovery, WKD key
  lookup, BIMI logos, CardDAV, one-click unsubscribe and allowed remote images. A remote image
  named `http:` is asked for over `https:` at the same address. A shared resolver refuses
  loopback, private, link-local and `.local` addresses, so a message cannot make mailo reach the
  machine it runs on or the local network.
- **Untrusted input is bounded.** Server responses (IMAP literals, JMAP event streams, POP3
  multi-line answers) and fetched bodies are read up to a fixed size.
- **Secrets are kept in the OS keychain:** the login keychain on macOS, the Credential Manager on
  Windows, the Secret Service on Linux. Account passwords and OAuth tokens are filed there through porter, and
  OpenPGP and S/MIME private keys are kept there, never in SQLite.
- **Attachment names are not trusted.** A name chosen by the sender is reduced to one safe path
  component before anything is written, so a name like `../../x` cannot escape the folder.
- **Files are private.** mailo's data, config and cache folders are mode 0700 and its database,
  blobs and saved attachments 0600. On macOS a saved or opened attachment carries the quarantine
  mark. A program or script is never opened from mailo; it is shown in its folder.
- **Unauthenticated decryption is said so.** An S/MIME body decrypted from a mode without
  integrity (AES-CBC) keeps remote content blocked, and its status says it is not authenticated.

## Known gaps

- The macOS and Windows packages are unsigned.
