//! What a macOS banner carries so that a click on it can be understood later: by the window that
//! is running when it is clicked, which need not be the process that posted it (`mailo watch`
//! posts banners too), and with nothing kept in memory between the post and the click.
//!
//! What the banner opens and whether it offers Reply ride along in its `userInfo` as text
//! ([`info`]); a click is the key of what was pressed (the same keys the other platforms' servers
//! report, [`super::act_for`]) with that text handed back ([`launch`]). Both are pure, so what a
//! click does is tested without a notification centre.

use super::desktop::Launch;
use super::{DEFAULT_ACTION, Notification, Opens, act_for};
use mail_domain::ThreadId;
use porter_core::AccountId;
use std::collections::HashMap;
use std::fmt::Debug;

/// A banner to post, as the app's notification centre is handed it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Banner {
    pub title: String,
    pub body: String,
    /// A system sound's name; `None` for none.
    pub sound: Option<String>,
    /// The title of the banner's main button, if it has one.
    pub main_button: Option<String>,
    /// Further buttons, `(key, title)`, offered with the main one.
    pub more_buttons: Vec<(String, String)>,
    /// Text that comes back, as it was, with the click ([`info`]).
    pub info: Vec<(String, String)>,
}

/// Where a banner is posted. The notification centre is the app's to reach (it takes platform
/// calls this crate does not make), so the app hands in what posts, and the one that hears the
/// clicks calls [`super::desktop::heard`].
pub trait Posts: Debug + Send + Sync {
    /// Post `banner` and do not wait for it. Nothing to say when there is no centre to post to.
    fn post(&self, banner: &Banner);
}

/// What the person pressed on a banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pressed {
    /// The banner itself.
    Content,
    /// Its main button.
    Main,
    /// One of the further buttons, by its key.
    More(String),
    /// Anything else (an inline reply, or nothing the banner offered).
    Other,
}

/// A click on a banner: what was pressed, and the text the banner was posted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Click {
    pub pressed: Pressed,
    pub info: HashMap<String, String>,
}

/// The `userInfo` key of what the banner opens: `thread:<id>` or `inbox:<account>`.
const OPENS: &str = "mailo.opens";
/// The `userInfo` key present, as `1`, when the banner offers Reply.
const REPLY: &str = "mailo.reply";

/// What a click on `n`'s banner needs, as the text pairs its `userInfo` holds.
pub fn info(n: &Notification) -> Vec<(String, String)> {
    let opens = match &n.opens {
        Opens::Thread(thread) => format!("thread:{thread}"),
        Opens::Inbox(account) => format!("inbox:{}", account.as_str()),
    };
    let mut info = vec![(OPENS.to_owned(), opens)];
    if n.reply {
        info.push((REPLY.to_owned(), "1".to_owned()));
    }
    info
}

/// What `info` says the banner opens, or `None` for a banner that is not ours or is garbled.
fn opens(info: &HashMap<String, String>) -> Option<Opens> {
    let text = info.get(OPENS)?;
    if let Some(thread) = text.strip_prefix("thread:") {
        // A garbled id is a banner that is not ours: it opens nothing.
        let uuid = uuid::Uuid::parse_str(thread).ok()?;
        return Some(Opens::Thread(ThreadId::from_uuid(uuid)));
    }
    let account = text.strip_prefix("inbox:")?;
    // As above, for a garbled account.
    Some(Opens::Inbox(AccountId::parse(account).ok()?))
}

/// The program a click starts: `action` is the key of what was pressed (`None` for anything the
/// banner did not offer), `info` what the banner was posted with. Nothing for a banner that
/// carries no `info` of ours, or a button it never offered.
pub fn launch(action: Option<&str>, info: &HashMap<String, String>) -> Option<Launch> {
    let opens = opens(info)?;
    let reply = info.get(REPLY).is_some_and(|value| value == "1");
    let act = act_for(action?, &opens, reply)?;
    Some(Launch { opens, act })
}

/// The key a press of the banner itself, or of its main button (Open), reports.
pub const CONTENT: &str = DEFAULT_ACTION;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::{ARCHIVE_ACTION, Act, REPLY_ACTION};

    fn note(opens: Opens, reply: bool) -> Notification {
        Notification {
            account: mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc)),
            summary: "Ada".to_owned(),
            body: "lunch".to_owned(),
            opens,
            reply,
        }
    }

    fn thread() -> Opens {
        Opens::Thread(ThreadId::from_uuid(uuid::Uuid::from_u128(1)))
    }

    fn inbox() -> Opens {
        Opens::Inbox(mail_domain::id::account_id_from_uuid(
            uuid::Uuid::from_u128(0xacc),
        ))
    }

    fn carried(n: &Notification) -> HashMap<String, String> {
        info(n).into_iter().collect()
    }

    #[test]
    fn a_press_is_understood_from_what_the_banner_carried_and_nothing_else() {
        let reply = carried(&note(thread(), true));
        let plain = carried(&note(thread(), false));
        let summary = carried(&note(inbox(), false));
        let cases = [
            (
                "the banner",
                Some(CONTENT),
                &plain,
                Some((thread(), Act::Open)),
            ),
            (
                "archive",
                Some(ARCHIVE_ACTION),
                &plain,
                Some((thread(), Act::Archive)),
            ),
            (
                "reply offered",
                Some(REPLY_ACTION),
                &reply,
                Some((thread(), Act::Reply)),
            ),
            ("reply not offered", Some(REPLY_ACTION), &plain, None),
            (
                "a summary opens its inbox",
                Some(CONTENT),
                &summary,
                Some((inbox(), Act::Open)),
            ),
            (
                "a summary has no archive",
                Some(ARCHIVE_ACTION),
                &summary,
                None,
            ),
            ("a button never offered", Some("delete"), &plain, None),
            ("an inline reply or nothing", None, &plain, None),
            (
                "a banner that is not ours",
                Some(CONTENT),
                &HashMap::new(),
                None,
            ),
            (
                "a banner with a garbled id",
                Some(CONTENT),
                &HashMap::from([(OPENS.to_owned(), "thread:nope".to_owned())]),
                None,
            ),
        ];
        for (name, action, info, expect) in cases {
            let expect = expect.map(|(opens, act)| Launch { opens, act });
            assert_eq!(launch(action, info), expect, "{name}");
        }
    }
}
