# mailo-pim

The personal information that travels beside mail: vCards (RFC 6350, and 2.1 and 3.0 as found in
the wild), iCalendar invitations (RFC 5545 and iTIP's replies), and the WebDAV requests and
replies CardDAV is spoken in. Text in, values out, and text back; sending a request is the
caller's business.

```rust
let cards = mail_pim::vcard::parse(
    "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Ada Lovelace\r\nEMAIL:ada@example.test\r\nEND:VCARD\r\n",
);
assert_eq!(cards.len(), 1);
assert_eq!(cards[0].formatted_name.as_deref(), Some("Ada Lovelace"));
assert_eq!(cards[0].emails[0].address, "ada@example.test");
```

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
