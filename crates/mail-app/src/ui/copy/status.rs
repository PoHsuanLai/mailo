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

/// What Apply Rules did.
pub fn rules_applied(acted: usize) -> String {
    match acted {
        0 => "No rule matched.".to_owned(),
        1 => "Rules acted on 1 message.".to_owned(),
        n => format!("Rules acted on {n} messages."),
    }
}

/// Why Apply Rules did nothing.
pub fn rules_not_applied(why: &str) -> String {
    format!("Rules not applied: {why}.")
}

/// Apply Rules with nothing open or picked.
pub const NOTHING_TO_APPLY_RULES_TO: &str = "Pick or open conversations to apply rules to them.";

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

// ⌘F's count, beside what was typed.
/// The match it is on, counted from 1: "3 of 12".
pub fn match_of(at: usize, total: usize) -> String {
    format!("{at} of {total}")
}
pub const NO_MATCHES: &str = "No matches";

// Peek, while it draws an attachment or when it cannot.
/// A PDF's page, counted from 1.
pub fn page_of(page: u32, count: u32) -> String {
    format!("Page {page} of {count}")
}
// What the Import sheet found at a path, before its count: "Thunderbird profile, 1 204 messages".
pub const SOURCE_EMLX: &str = "Apple Mail message";
pub const SOURCE_APPLE_MAIL: &str = "Apple Mail mailboxes";
pub const SOURCE_THUNDERBIRD: &str = "Thunderbird profile";

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

/// The quiet line under a message's header that carries a one-time code: the code, then Copy
/// Code.
pub fn one_time_code(code: &str) -> String {
    format!("Code {code}")
}

/// The quiet line over a message whose remote images are held back; its actions follow it.
pub const IMAGES_HIDDEN: &str = "Images hidden";
/// The message source sheet, for a message whose body is not here yet.
pub const SOURCE_NOT_HERE: &str = "This message isn’t downloaded yet.";

/// The message source sheet, when the message is longer than it shows: "Showing the first
/// 256.0 kB of 4.2 MB."
pub fn source_cut(shown: &str, whole: &str) -> String {
    format!("Showing the first {shown} of {whole}. Download the message for all of it.")
}

/// The line over a message that asks for a read receipt, under "Ask each time": who asked.
pub fn receipt_asked(sender: &str) -> String {
    format!("{sender} asked for a read receipt")
}

/// Beside [`receipt_asked`] when the receipt would go to an address other than the one the
/// message came back from: where it would go.
pub fn receipt_goes_elsewhere(to: &str) -> String {
    format!("It would go to {to}, not where the message came from")
}

/// Beside [`receipt_asked`] when the message names no address it came back from, so where the
/// receipt would go can be said but not checked.
pub fn receipt_goes_unchecked(to: &str) -> String {
    format!("It would go to {to}, which can’t be checked against where the message came from")
}

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

    /// What an account's tile says while the person has paused its syncing.
    pub const PAUSED: &str = "Syncing is paused.";
    /// A folder role left to the server: "Automatic (Sent Items)", or plain "Automatic" before
    /// the server has named one.
    pub fn automatic(observed: Option<&str>) -> String {
        match observed {
            Some(path) => format!("Automatic ({path})"),
            None => "Automatic".to_owned(),
        }
    }
    /// The help under Description: what it changes.
    pub const DESCRIPTION_HELP: &str = "Shown in the sidebar in place of the address.";
    /// The help under Sync while it is off.
    pub const PAUSED_HELP: &str = "mailo won\u{2019}t sync this account until you turn this on.";
    /// An amount of storage, in the largest unit it fills: "512 kB", "1.2 GB", "15 GB".
    ///
    /// The unit is chosen after rounding to a tenth, so an amount that rounds up to 1024 of one
    /// unit is said in the next, never as "1024 kB".
    pub fn amount(bytes: u64) -> String {
        const UNITS: [(&str, u64); 4] = [
            ("kB", 1 << 10),
            ("MB", 1 << 20),
            ("GB", 1 << 30),
            ("TB", 1 << 40),
        ];
        if bytes < UNITS[0].1 {
            return format!("{bytes} B");
        }
        // The tenths of a unit that `bytes` comes to, rounded to nearest.
        let tenths = |size: u64| (u128::from(bytes) * 10 + u128::from(size) / 2) / u128::from(size);
        let (unit, tenths) = UNITS
            .iter()
            .map(|&(unit, size)| (unit, tenths(size)))
            .find(|&(_, tenths)| tenths < 1024 * 10)
            // Past what the units name, the last says it.
            .unwrap_or_else(|| (UNITS[3].0, tenths(UNITS[3].1)));
        match tenths % 10 {
            0 => format!("{} {unit}", tenths / 10),
            tenth => format!("{}.{tenth} {unit}", tenths / 10),
        }
    }
    /// The Storage row: "1.2 GB of 15 GB used".
    pub fn storage_used(used: &str, limit: &str) -> String {
        format!("{used} of {limit} used")
    }
    /// The Storage row once little is left: mail that does not fit is refused by the server.
    pub fn storage_nearly_full(used: &str, limit: &str) -> String {
        format!("{used} of {limit} used, nearly full")
    }
}

/// What a signature's row in Settings says of it: which default it is.
pub mod signature {
    /// The default for new messages and for replies.
    pub const DEFAULT: &str = "New messages and replies";
    pub const NEW_MESSAGES: &str = "New messages";
    pub const REPLIES: &str = "Replies";
}

/// The composer's attachment strip: what the message comes to, as it would be sent.
pub fn message_size(size: &str) -> String {
    format!("About {size} in all")
}

/// Under Picture Size, while pictures are sent smaller.
pub const PICTURES_WITHOUT_LOCATION: &str = "Smaller pictures leave out where they were taken.";

/// The composer's warning, before Send, of a message larger than its server takes: its size, the
/// limit, and who set it.
pub fn too_large(size: &str, limit: &str, said: mail_core::compose::Said) -> String {
    use mail_core::compose::Said;
    match said {
        Said::Server => {
            format!("This message is {size}, and its mail server takes at most {limit}.")
        }
        Said::Graph => format!("This message is {size}, and Microsoft 365 sends at most {limit}."),
        Said::Usual => {
            format!("This message is {size}, and many mail servers refuse messages over {limit}.")
        }
    }
}

// Where a search applied to the list looks, said at the end of the search shown in its header.
/// A search of every account in view.
pub const SEARCHED_ALL_MAIL: &str = "in all mail";
/// A search of one account of several: "in ada@example.com".
pub fn searched_in(account: &str) -> String {
    format!("in {account}")
}
/// A bare pattern, matched against the rows the place had loaded: "on this page of Inbox".
pub fn searched_page_of(place: &str) -> String {
    format!("on this page of {place}")
}

/// A group's row in Settings › Contacts: how many members it has.
pub fn members(count: usize) -> String {
    match count {
        0 => "No members".to_owned(),
        1 => "1 member".to_owned(),
        many => format!("{many} members"),
    }
}
