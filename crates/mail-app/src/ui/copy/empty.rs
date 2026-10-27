//! Empty states: what a list or a pane says when it has nothing to show.
//!
//! Case: Sentence case.

/// A conversation or a draft with no subject, where its subject would be.
pub use mail_domain::NO_SUBJECT;
/// A draft with nobody to go to yet, where its recipients would be.
pub const NO_RECIPIENT: &str = "(no recipient)";
/// Settings › General › Trusted Senders, before anyone is trusted.
pub const NO_TRUSTED_SENDERS: &str = "No trusted senders yet";
/// Settings › General › VIPs, with nobody marked.
pub const NO_VIPS: &str = "No VIPs yet";
/// Settings › Accounts › Other Keys, with nothing on it.
pub const NO_OTHER_KEYS: &str = "No other keys or certificates yet";
/// Settings › Accounts with no account.
pub const NO_ACCOUNTS: &str = "No accounts yet";
/// Settings › Rules with no account to make a rule on.
pub const ADD_AN_ACCOUNT_FIRST: &str = "Add an account first.";

/// The list with nothing left in it once the quick filter's toggles narrowed it.
pub const NOTHING_MATCHES_FILTER: &str = "Nothing here matches the filter";

/// The reader column with no conversation open.
pub const NO_MESSAGE_SELECTED: &str = "No message selected";

/// A contact card with no name in it.
pub const NO_NAME: &str = "(no name)";
