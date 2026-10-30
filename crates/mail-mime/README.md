# mailo-mime

Reading and writing email messages: MIME parsing into plain values, message building, authentication results, unsubscribe headers
and read receipts (MDN). Signing and sealing are
[`mailo-crypto`](https://crates.io/crates/mailo-crypto)'s, mailbox files
[`mailo-archive`](https://crates.io/crates/mailo-archive)'s. Bytes in, values out: nothing here opens a socket or
reads a file.

```rust
use mail_mime::parse;

let raw = b"From: Ada <ada@example.test>\r\n\
Subject: Minutes\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>See the notes.</p>\r\n";

let message = parse(raw).expect("a message");
assert_eq!(message.subject, "Minutes");
assert!(message.html.is_some());
```

What a parsed message shows (sanitizing, blocks, the printed page) is
[`mailo-reader`](https://crates.io/crates/mailo-reader)'s.

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
