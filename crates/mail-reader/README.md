# mailo-reader

How a received message becomes what is read, with no I/O: untrusted HTML sanitized for a
sandboxed frame and lowered to blocks, plain text and `format=flowed` read as the sender
meant, a reply's quoted tail folded, the script a message is written in, and the printed page.
Nothing here opens a file, reads a clock or draws a window.

The sanitizer is built for showing untrusted mail. Its output carries no script, no event
handler, no `javascript:` URL, and, with remote images blocked, nothing that fetches from the
network.

```rust
use mail_mime::parse;
use mail_reader::{SanitizePolicy, sanitize};

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
