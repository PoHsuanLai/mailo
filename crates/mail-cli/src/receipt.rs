//! What the read-receipt commands say: the request under a shown message, and the answer given.

use mail_core::receipt::{ReceiptState, Settled};
use mail_domain::{MessageId, ReceiptAnswer};
use mail_mime::ReturnPath;

/// The lines `mailo show` prints under a message, or nothing.
pub fn describe(state: &ReceiptState, message: MessageId) -> String {
    match state {
        ReceiptState::NotAsked | ReceiptState::Unknown => String::new(),
        ReceiptState::Answered(ReceiptAnswer::Sent) => "    read receipt sent\n".to_owned(),
        ReceiptState::Answered(ReceiptAnswer::Declined) => "    read receipt declined\n".to_owned(),
        ReceiptState::Pending(ask) => {
            let to: Vec<&str> = ask.to.iter().map(|a| a.email.as_str()).collect();
            let mut out = format!(
                "    asks for a read receipt, to {}\n    \
                 send one with: mailo receipt {message}   or decline: mailo receipt {message} --decline\n",
                to.join(", ")
            );
            if let ReturnPath::Differs { return_path } = &ask.return_path {
                out.push_str(&format!(
                    "    careful: the receipt would go to another domain than the message came \
                     from ({return_path})\n"
                ));
            }
            out
        }
    }
}

/// What `mailo receipt` did.
pub fn answered(settled: &Settled) -> String {
    match settled {
        Settled::Sent { to } => format!(
            "queued a read receipt to {}\n\ndeliver it with: mailo sync\n",
            to.join(", ")
        ),
        Settled::Declined { to } => {
            format!("declined: no receipt will go to {}\n", to.join(", "))
        }
    }
}
