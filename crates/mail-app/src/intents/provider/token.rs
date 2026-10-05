//! Undo tokens: what mailo hands the router so that an action can be taken back later, and reads
//! again when the router brings one back.
//!
//! A token names the thing to do, not a row anywhere: `stack-7` is the seventh entry of the undo
//! stack, `discard-<id>` removes a draft mailo made, `unsend-<id>.<id>` takes the messages an
//! action queued back to drafts. A token mailo did not mint, or one from a mailo that has since stopped (the stack is
//! in memory), reads as none and is `Gone`.

use mail_core::undo::UndoHandle;
use mail_domain::DraftId;
use std::fmt;

/// What an undo token stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    /// One gesture on the undo stack: every conversation it touched.
    Stack(UndoHandle),
    /// A draft an action made, to be discarded.
    Discard(DraftId),
    /// The messages an action queued (one for a send, one a conversation for a forward), to be
    /// taken back to drafts.
    Unsend(Vec<DraftId>),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Stack(UndoHandle(n)) => write!(f, "stack-{n}"),
            Token::Discard(draft) => write!(f, "discard-{draft}"),
            Token::Unsend(drafts) => {
                f.write_str("unsend-")?;
                for (n, draft) in drafts.iter().enumerate() {
                    if n > 0 {
                        f.write_str(".")?;
                    }
                    write!(f, "{draft}")?;
                }
                Ok(())
            }
        }
    }
}

impl Token {
    /// The token written as `text`, or `None` when it is not one of ours.
    pub(super) fn parse(text: &str) -> Option<Token> {
        let draft = |rest: &str| rest.parse().ok().map(DraftId::from_uuid);
        match text.split_once('-')? {
            ("stack", n) => n.parse().ok().map(|n| Token::Stack(UndoHandle(n))),
            ("discard", rest) => draft(rest).map(Token::Discard),
            ("unsend", rest) => rest
                .split('.')
                .map(draft)
                .collect::<Option<Vec<_>>>()
                .map(Token::Unsend),
            _ => None,
        }
    }
}
