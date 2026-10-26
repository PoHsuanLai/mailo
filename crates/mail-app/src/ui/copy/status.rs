//! Status lines: what the window says while something is so (fresh, syncing, offline).
//!
//! Case: Sentence case, as short as says it ("Updated 3 minutes ago").

/// "1 message", "132 messages".
pub fn messages(count: u64) -> String {
    if count == 1 {
        "1 message".to_owned()
    } else {
        format!("{count} messages")
    }
}

// How fresh the mail is, in the sync mark's tip's fewer words.
pub const JUST_NOW: &str = "just now";

/// "1 min ago", "3 min ago".
pub fn minutes_ago(minutes: i64) -> String {
    format!("{minutes} min ago")
}

/// A draft's row in ⌘K's Recent, under its name: "Draft · 14:05", "Draft · Tue".
pub fn draft_touched(when: &str) -> String {
    format!("Draft \u{b7} {when}")
}

/// The bar over a message held to leave at a time: "Leaves Tue 09:00".
pub fn leaves(when: &str) -> String {
    format!("Leaves {when}")
}

/// A draft's row named by who it goes to, when it has no subject: "To Ada Lovelace".
pub fn draft_to(who: &str) -> String {
    format!("To {who}")
}

/// "at 14:05", for an update an hour or more ago.
pub fn at_clock(clock: &str) -> String {
    format!("at {clock}")
}

/// The selection bar's count: "3 selected".
pub fn selected(count: usize) -> String {
    format!("{count} selected")
}

// Peek, while it draws an attachment or when it cannot.
/// A PDF's page, counted from 1.
pub fn page_of(page: u32, count: u32) -> String {
    format!("Page {page} of {count}")
}
pub const LOADING: &str = "Loading…";
pub const NOT_DOWNLOADED: &str = "Not downloaded yet";
pub const NO_PREVIEW: &str = "No preview for this file";
/// Why a part that should have drawn did not.
pub fn no_preview(why: &str) -> String {
    format!("No preview: {why}")
}
/// A forwarded message with no plain text to show.
pub const NO_TEXT: &str = "This message has no plain text";
/// Why anyview drew no card: what follows "No preview: ".
pub const TOOK_TOO_LONG: &str = "it took too long";
pub const TOO_LARGE_TO_PEEK: &str = "it’s too large";
/// A part the store could not read; the reason goes to the log.
pub const UNREADABLE: &str = "it couldn’t be read here";
pub fn no_attachment(index: usize) -> String {
    format!("there is no attachment {index}")
}

/// The trust popover's last line: what the receiving server checked about the sender, in the
/// checks' own words ("SPF pass · DKIM pass · DMARC pass — checked by mx.example.com").
pub fn sender_checks(said: &str) -> String {
    format!("Sender checks: {said}")
}

/// The quiet line over a message whose remote images are held back; its actions follow it.
pub const IMAGES_HIDDEN: &str = "Images hidden";

/// Whether an account's vacation reply is on, beside its address in Settings › Vacation Reply.
pub mod vacation {
    pub const OFF: &str = "Off";
    pub const ON: &str = "On";
    /// Its "Until" has passed: the server sends it no longer.
    pub const ENDED: &str = "Ended";
    /// The account's server runs no vacation reply.
    pub const UNAVAILABLE: &str = "Not available";

    /// On, starting at `when`.
    pub fn from(when: &str) -> String {
        format!("From {when}")
    }

    /// On, ending at `when`.
    pub fn until(when: &str) -> String {
        format!("Until {when}")
    }

    /// When a kept reply runs: from `from` until `until`, each `None` when it is unbounded.
    pub fn runs(from: Option<&str>, until: Option<&str>) -> String {
        match (from, until) {
            (None, None) => "from now until it is turned off".to_owned(),
            (Some(from), None) => format!("from {from} until it is turned off"),
            (None, Some(until)) => format!("from now until {until}"),
            (Some(from), Some(until)) => format!("from {from} until {until}"),
        }
    }
}

/// How many accounts a Space shows, beside its name in Settings › Spaces.
pub fn accounts_shown(count: usize) -> String {
    match count {
        0 => "No accounts".to_owned(),
        1 => "1 account".to_owned(),
        many => format!("{many} accounts"),
    }
}

/// Several of a Space's accounts are in trouble, as the All tile's mark says.
pub fn accounts_need_attention(count: usize) -> String {
    format!("{count} accounts need attention.")
}

/// Under the rules of an account whose server cannot run them: mailo runs them as it syncs.
pub const RULES_RUN_HERE: &str = "Runs only while mailo is open.";

/// What the reader says about a message's protection, a line a fact, for OpenPGP and S/MIME
/// (`ui::pgp::said`). No full stop: each is one of several lines.
pub mod protection {
    pub const NOT_ENCRYPTED: &str = "Not encrypted";
    pub const NOT_SIGNED: &str = "Not signed";
    pub const ONLY_PART_SIGNED: &str = "Only part of this message is signed";

    // OpenPGP.
    pub const DECRYPTED: &str = "Encrypted, and decrypted for reading";
    /// `ids` are the keys it is encrypted to, grouped and joined.
    pub fn to_keys_not_held(ids: &str) -> String {
        format!("Encrypted to keys you don’t have ({ids})")
    }
    pub fn passphrase_needed(key: &str) -> String {
        format!("Encrypted to your key {key}: passphrase needed")
    }
    pub fn unreadable(why: &str) -> String {
        format!("Encrypted, and couldn’t be read: {why}")
    }
    /// Who signed, when the key held names nobody.
    pub const A_KEY_YOU_HOLD: &str = "a key you hold";
    pub const VERIFIED_BY_YOU: &str = "verified by you";
    pub const NOT_VERIFIED_BY_YOU: &str = "not verified by you";
    pub fn good_signature(name: &str, key: &str, trust: &str) -> String {
        format!("Good signature by {name} · {key} · {trust}")
    }
    pub const BAD_SIGNATURE: &str =
        "Bad signature: the message changed after signing, or the signature is forged";
    pub fn signed_by_key_not_held(key: &str) -> String {
        format!("Signed by key {key}, which you don’t have")
    }

    // S/MIME.
    pub const SMIME_DECRYPTED: &str = "Encrypted with S/MIME, and decrypted for reading";
    /// Decrypted from encryption that cannot show it unchanged (CBC): why its remote images do
    /// not load.
    pub const SMIME_NOT_AUTHENTICATED: &str = "Encrypted with S/MIME and decrypted for reading, but the encryption can\u{2019}t show the message wasn\u{2019}t changed, so its remote images stay blocked";
    /// `to` are the certificates it is encrypted to, joined.
    pub fn smime_to_certs_not_held(to: &str) -> String {
        format!(
            "Encrypted with S/MIME to certificates you hold no key for ({to}), so it can’t be read here"
        )
    }
    pub fn smime_unreadable(why: &str) -> String {
        format!("Encrypted with S/MIME, and couldn’t be read: {why}")
    }
    /// Who signed, when the certificate names nobody.
    pub const A_CERTIFICATE_WITH_NO_NAME: &str = "a certificate with no name";
    pub fn smime_good(name: &str) -> String {
        format!("Good S/MIME signature by {name}, from a certificate you trust")
    }
    pub fn smime_doubtful(name: &str) -> String {
        format!(
            "S/MIME signature by {name}: the message is unchanged, but the certificate doesn’t vouch for the sender"
        )
    }
    pub const SMIME_UNKNOWN_SIGNER: &str = "Signed with S/MIME by a certificate that isn’t in the message and that you don’t hold, so it can’t be checked";

    // The signer's certificate, in one line of parts joined by " · ".
    pub fn certificate_of(subject: &str) -> String {
        format!("Certificate of {subject}")
    }
    pub fn issued_by(issuer: &str) -> String {
        format!("issued by {issuer}")
    }
    pub fn valid(from: &str, to: &str) -> String {
        format!("valid {from} to {to}")
    }

    // One thing wrong with the certificate behind a signature that matches.
    pub const UNTRUSTED: &str = "Untrusted: no authority you trust issued its certificate, so anyone could have made it. \
                                 Trust it in Other Keys (Settings › Accounts) only after checking with its owner";
    pub fn expired(on: &str) -> String {
        format!(
            "Expired: a certificate in its chain stopped being valid on {on}, before the message was signed"
        )
    }
    pub fn not_yet_valid(until: &str) -> String {
        format!(
            "Not yet valid: a certificate in its chain wasn’t valid until {until}, after the message was signed"
        )
    }
    pub const NOT_FOR_EMAIL: &str = "Not for mail: its certificate wasn’t issued for signing mail";
    pub const NO_SENDER: &str = "No sender to check the certificate against";
    pub fn for_someone_else(from: &str) -> String {
        format!("Signed with a certificate for someone else’s address, not {from}")
    }

    // A signature that does not hold.
    pub const SMIME_ALTERED: &str = "Bad S/MIME signature: changed after signing";
    pub const SMIME_FORGED: &str = "Bad S/MIME signature: does not match its certificate";
    pub fn smime_weak(algorithm: &str) -> String {
        format!("S/MIME signature made with {algorithm}, which is too weak")
    }
    pub fn smime_unsupported(algorithm: &str) -> String {
        format!("S/MIME signature made with {algorithm}, which isn’t checked")
    }
    pub fn smime_malformed(why: &str) -> String {
        format!("S/MIME signature that can’t be read: {why}")
    }
}

/// An account's page in Settings: how the account has each of its rows
/// ([`super::fields::account`]).
pub mod account {
    /// The Status row while the account waits for the person: the same for a sign-in refused
    /// and for Mail no longer allowed, since the button beside it is the same.
    pub const WAITING: &str = "mailo can\u{2019}t fetch new mail until you sign in again.";

    // How a server's connection is protected.
    pub const TLS: &str = "TLS";
    pub const STARTTLS: &str = "STARTTLS";
    pub const NOT_ENCRYPTED: &str = "not encrypted";
    /// A server: "IMAP, imap.example.com:993, TLS".
    pub fn server(protocol: &str, host: &str, port: u16, security: &str) -> String {
        format!("{protocol}, {host}:{port}, {security}")
    }

    pub const LEFT_ON_SERVER: &str = "Left on the server";
    pub const DELETED_FROM_SERVER: &str = "Deleted from the server";
    pub const GRAPH: &str = "Microsoft Graph";
    pub fn jmap(session: &str) -> String {
        format!("JMAP, {session}")
    }
    pub const KEPT_HERE: &str = "Kept on this computer";
    pub const JMAP_SAME_SERVER: &str = "JMAP, on the same server";
    pub const DOES_NOT_SEND: &str = "Does not send";

    // Who an account signs in with.
    pub const GOOGLE: &str = "Google";
    pub const MICROSOFT: &str = "Microsoft";
    /// An issuer mailo does not sign in to; no account here holds one.
    pub const ANOTHER_ISSUER: &str = "another issuer";
    pub fn in_the_browser(issuer: &str) -> String {
        format!("With {issuer}, in the browser")
    }
    pub fn with_a_token(username: &str) -> String {
        format!("With a token, as {username}")
    }
    pub fn with_a_password(username: &str) -> String {
        format!("With a password, as {username}")
    }
}

/// A group's row in Settings › Contacts: how many members it has.
pub fn members(count: usize) -> String {
    match count {
        0 => "No members".to_owned(),
        1 => "1 member".to_owned(),
        many => format!("{many} members"),
    }
}
