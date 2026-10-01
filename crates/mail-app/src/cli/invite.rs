//! `mailo invite`: the words typed after it, as the command [`mail_core::invite`] runs.

use mail_core::invite::{InviteAction, InviteCommand};
use mail_domain::{Attendance, MessageId};
use std::path::PathBuf;

/// The words after `invite`.
pub fn parse(args: &[String]) -> Result<InviteCommand, String> {
    let usage = || super::usage();
    let raw = args
        .first()
        .ok_or_else(|| format!("invite needs a message id\n\n{}", usage()))?;
    let uuid = raw
        .parse()
        .map_err(|_| format!("{raw:?} is not a message id"))?;
    let message = MessageId::from_uuid(uuid);
    let rest = args.get(1..).unwrap_or_default();
    let action = match rest.first().map(String::as_str) {
        None => InviteAction::Show,
        Some("--ics") => match rest {
            [_, path] => InviteAction::Ics(PathBuf::from(path)),
            [_] => return Err(format!("--ics needs a file to write\n\n{}", usage())),
            [_, _, extra, ..] => return Err(format!("unexpected {extra:?}\n\n{}", usage())),
            [] => return Err(usage()),
        },
        Some(word) => {
            let attendance = match word {
                "accept" => Attendance::Accepted,
                "tentative" => Attendance::Tentative,
                "decline" => Attendance::Declined,
                other => return Err(format!("unknown answer {other:?}\n\n{}", usage())),
            };
            let comment = match &rest[1..] {
                [] => None,
                [flag, text] if flag == "--comment" => Some(text.clone()),
                [flag] if flag == "--comment" => {
                    return Err(format!("--comment needs the text to send\n\n{}", usage()));
                }
                [extra, ..] => return Err(format!("unexpected {extra:?}\n\n{}", usage())),
            };
            InviteAction::Answer {
                attendance,
                comment,
            }
        }
    };
    Ok(InviteCommand { message, action })
}
