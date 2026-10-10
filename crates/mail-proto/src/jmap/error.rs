//! A method call the server answered with `error` (RFC 8620 §3.6.2).

use crate::{ProtoError, Refusal};
use mail_domain::{Retry, Retryable};
use std::time::Duration;

/// Why one method call in a request did not run.
///
/// A request succeeds or fails as a whole only at the HTTP level; inside it each call answers
/// for itself, and one refused call leaves the others' results standing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MethodError {
    /// `cannotCalculateChanges`: the state we hold is older than the server remembers. Not a
    /// failure — the caller starts again from a full listing.
    #[error("cannotCalculateChanges")]
    CannotCalculateChanges,
    /// Any other error type the server named, with its description where it gave one.
    #[error("{}", refusal_text(.kind, .description.as_deref()))]
    Refused {
        kind: String,
        description: Option<String>,
    },
    /// No answer with the call id we sent, or an answer of the wrong shape.
    #[error("no usable answer: {0}")]
    Missing(String),
}

fn refusal_text(kind: &str, description: Option<&str>) -> String {
    match description {
        Some(d) => format!("{kind}: {d}"),
        None => kind.to_owned(),
    }
}

impl MethodError {
    /// The error named by an `["error", {"type": …}, id]` response's arguments.
    pub(super) fn from_args(args: &serde_json::Value) -> MethodError {
        let kind = args
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("serverFail");
        if kind == "cannotCalculateChanges" {
            return MethodError::CannotCalculateChanges;
        }
        MethodError::Refused {
            kind: kind.to_owned(),
            description: args
                .get("description")
                .and_then(|d| d.as_str())
                .map(str::to_owned),
        }
    }
}

impl Retryable for MethodError {
    /// As the [`ProtoError`] it would become.
    fn retry(&self) -> Retry {
        ProtoError::from(self.clone()).retry()
    }
}

impl From<MethodError> for ProtoError {
    /// Classified by the error's type, which the RFC fixes, and never by its description, which
    /// it does not.
    fn from(error: MethodError) -> ProtoError {
        let text = error.to_string();
        match error {
            MethodError::Missing(why) => ProtoError::Malformed(format!("JMAP: {why}")),
            // Only reachable when a caller did not handle it itself; the state is gone, and
            // asking again with it will fail the same way.
            MethodError::CannotCalculateChanges => ProtoError::Refused {
                kind: Refusal::Permanent,
                text,
            },
            MethodError::Refused { kind, .. } => match kind.as_str() {
                // "The server is temporarily unavailable", and an internal error that may not
                // recur. Retried, like an SMTP 4xx.
                "serverUnavailable" | "serverFail" | "serverPartialFail" => ProtoError::Refused {
                    kind: Refusal::Transient,
                    text,
                },
                // RFC 8620 §5.3 names no rate-limit error for methods, but RFC 8621 does for
                // submission (`rateLimit`), and backing off is the whole remedy.
                "rateLimit" => ProtoError::Throttled {
                    reason: text,
                    retry_after: Some(Duration::from_secs(60 * 15)),
                },
                "unknownCapability" | "unknownMethod" | "accountNotSupportedByMethod" => {
                    ProtoError::Unsupported(text)
                }
                _ => ProtoError::Refused {
                    kind: Refusal::Permanent,
                    text,
                },
            },
        }
    }
}
