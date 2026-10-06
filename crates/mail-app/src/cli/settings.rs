//! The command line's half of `crate::settings`: `mailo notify [on|off]`, and
//! `mailo --write-schema <dir>`, which writes the schema the desktop's Settings app reads.

use crate::settings::{self, NewMail};
use ds_settings::ConfigRoot;

/// `mailo notify [on|off]`: set new-mail notifications, then say which they are. With no config
/// directory there is nowhere they are kept, and it says so rather than guess.
pub fn notify(
    root: Option<&ConfigRoot>,
    set: Option<mail_core::notify::Setting>,
) -> Result<String, String> {
    let Some(root) = root else {
        return Err("no config directory to keep the setting in: set HOME".to_owned());
    };
    let now = match set {
        Some(setting) => {
            settings::change(root, |settings| {
                settings.notifications.new_mail = NewMail::from(setting);
            })?
            .notifications
            .new_mail
        }
        None => settings::load(root).notifications.new_mail,
    };
    Ok(match now {
        NewMail::On => "notifications are on\n".to_owned(),
        NewMail::Off => {
            "notifications are off; `mailo notify on` to have `watch` raise them again\n".to_owned()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::notify;
    use ds_settings::ConfigRoot;
    use mail_core::notify::Setting;

    #[test]
    fn notify_writes_settings_toml_and_says_what_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = ConfigRoot::Scratch(dir.path().to_path_buf());
        // (what is set, what it says)
        let cases = [
            (None, "notifications are on"),
            (Some(Setting::Off), "notifications are off"),
            (None, "notifications are off"),
            (Some(Setting::On), "notifications are on"),
        ];
        for (set, says) in cases {
            let said = notify(Some(&root), set).unwrap();
            assert!(said.starts_with(says), "{set:?}: {said}");
        }
        assert!(dir.path().join("mailo").join("settings.toml").exists());
    }

    #[test]
    fn with_no_config_directory_it_says_so() {
        let said = notify(None, None).unwrap_err();
        assert!(said.contains("no config directory"), "{said}");
    }
}
