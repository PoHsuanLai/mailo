# mailo-domain

The vocabulary of a mail client, as plain Rust data and pure functions: accounts and identities,
messages, threads and their summaries, mailboxes and labels, drafts, filters and views, the
operations a client applies locally and replays on a server, and the threading that groups
messages into conversations.

No I/O, no async, no clock: every function that needs the time takes it. That makes the types
safe to persist, send between threads and test without a runtime.

```rust
use mail_domain::{normalize_id, subject_or_none};

// Message-IDs compare without their angle brackets; an unusable one is empty.
assert_eq!(normalize_id(" <a1@example.test> "), "a1@example.test");
assert_eq!(normalize_id("<not an id>"), "");

// A subject with nothing in it is shown as such, not as a blank line.
assert_ne!(subject_or_none(""), "");
```

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
