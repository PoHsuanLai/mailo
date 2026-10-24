# mailo-proto

Mail protocols as sans-I/O state machines: IMAP, POP3, SMTP submission, JMAP and Sieve
(ManageSieve). A machine is fed the bytes that arrived and answers with what it needs next:
bytes to write, more to read, a TLS upgrade or a wait. The caller owns the socket, the runtime
and the clock, so the same machine runs under tokio, a blocking loop, or a test that replays a
recorded transcript.

```rust
use mail_domain::SaslMech;
use mail_proto::machine::{IoNeed, IoReady, Machine, Progress};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use porter_core::{Credential, SecretText};

let mut session = ImapSession::new(
    ImapAuth {
        username: "ada@example.test".to_owned(),
        credential: Credential::Password(SecretText::new("s3cr3t".to_owned())),
        sasl: vec![SaslMech::Plain],
    },
    vec![ImapCommand::Capability],
)
.expect("a credential with no line breaks in it");

// The session waits for the server's greeting before it says anything.
let Progress::Need(needs) = session.start() else { panic!("it needs bytes") };
assert!(matches!(needs.as_slice(), [IoNeed::Read]));

// Greeted, it asks to write its first command.
let Progress::Need(needs) = session.feed(IoReady::Bytes(b"* OK IMAP4rev1 ready\r\n".to_vec()))
else {
    panic!("it has a command to send")
};
assert!(needs.iter().any(|need| matches!(need, IoNeed::Write(_))));
```

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
