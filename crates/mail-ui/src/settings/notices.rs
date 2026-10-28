//! The notifications the desktop shows, as the settings say as each is said.
//!
//! A window and a watch run for days, so what they say is not what the settings said when they
//! started: Notifications, Notify for, Notify in, Sound and the muted accounts are each read
//! again where they are used, and a switch in Settings is followed by the next pass, not by the
//! next start.

use super::{MailSettings, NewMail, load};
use ds_settings::ConfigRoot;
use mail_client::notify::{Interest, Notification, Notifier, Sound, Whom};

/// How a notification reaches the desktop, with the sound the settings choose.
pub type Shows = Box<dyn Fn(&Notification, Sound) + Send + Sync>;

/// A [`Notifier`] that reads the person's settings each time it is asked.
pub struct Notices {
    shows: Shows,
    root: Option<ConfigRoot>,
}

impl Notices {
    /// Notifications that `shows` puts on the desktop while the settings under `root` have them
    /// on; with no `root` (no config directory) the settings are the defaults.
    pub fn new(shows: Shows, root: Option<ConfigRoot>) -> Self {
        Notices { shows, root }
    }

    fn settings(&self) -> MailSettings {
        self.root.as_ref().map(load).unwrap_or_default()
    }

    /// Whose new mail is said, as the settings say now.
    pub fn whom(&self) -> Whom {
        self.settings().notifications.notify_for.into()
    }
}

impl std::fmt::Debug for Notices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notices(..)")
    }
}

impl Notifier for Notices {
    fn interest(&self) -> Interest {
        self.settings().notifications.interest()
    }

    fn show(&self, notification: &Notification) {
        let set = self.settings().notifications;
        if set.new_mail == NewMail::On {
            (self.shows)(notification, set.interest().sound);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{NotifyFor, NotifyPlaces, change};
    use mail_client::notify::Places;
    use std::sync::{Arc, Mutex};

    fn note() -> Notification {
        Notification {
            account: mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(1)),
            summary: "Ada".to_owned(),
            body: "lunch".to_owned(),
            opens: mail_core::notify::Opens::Inbox(mail_domain::id::account_id_from_uuid(
                uuid::Uuid::from_u128(1),
            )),
            reply: false,
        }
    }

    #[test]
    fn what_is_said_follows_the_settings_changed_between_two_passes() {
        let dir = tempfile::tempdir().unwrap();
        let root = ConfigRoot::Scratch(dir.path().to_path_buf());
        let said: Arc<Mutex<Vec<Sound>>> = Arc::default();
        let seen = said.clone();
        let notices = Notices::new(
            Box::new(move |_, sound| seen.lock().unwrap().push(sound)),
            Some(root.clone()),
        );

        // As they ship: the Inbox, everyone, with the system's sound.
        assert_eq!(notices.interest().places, Places::Inbox);
        assert_eq!(notices.whom(), Whom::Everyone);
        notices.show(&note());
        assert_eq!(*said.lock().unwrap(), [Sound::Default]);

        change(&root, |set| {
            set.notifications.places = NotifyPlaces::Followed;
            set.notifications.notify_for = NotifyFor::Vips;
            set.notifications.muted_accounts = vec!["me@example.test".to_owned()];
            set.notifications.sound = crate::settings::NotifySound::Silent;
        })
        .unwrap();
        assert_eq!(notices.interest().places, Places::Followed);
        assert_eq!(notices.whom(), Whom::Vips);
        assert!(!notices.interest().hears("me@example.test"));
        notices.show(&note());
        assert_eq!(*said.lock().unwrap(), [Sound::Default, Sound::Silent]);

        change(&root, |set| set.notifications.new_mail = NewMail::Off).unwrap();
        notices.show(&note());
        assert_eq!(
            said.lock().unwrap().len(),
            2,
            "switched off, nothing is said"
        );
    }
}
