# mailo-mime

Reading and writing email messages: MIME parsing into plain values, message building, HTML
sanitizing for display and read receipts (MDN). Signing and sealing are
[`mailo-crypto`](https://crates.io/crates/mailo-crypto)'s, mailbox files
[`mailo-archive`](https://crates.io/crates/mailo-archive)'s. Bytes in, values out: nothing here opens a socket or
reads a file.

The sanitizer is built for showing untrusted mail. Its output carries no script, no event
handler, no `javascript:` URL, and, with remote images blocked, nothing that fetches from the
network.

```rust
use mail_mime::{SanitizePolicy, parse, sanitize};

let raw = b"From: Ada <ada@example.test>\r\n\
Subject: Minutes\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p onclick=\"steal()\">See <img src=\"https://tracker.test/p.gif\"> the notes.</p>\r\n";

let message = parse(raw).expect("a message");
assert_eq!(message.subject, "Minutes");

let html = message.html.expect("an HTML body");
let safe = sanitize(&html, SanitizePolicy::CURRENT);
assert!(!safe.as_str().contains("onclick"));
assert!(!safe.as_str().contains("tracker.test"));
```

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
