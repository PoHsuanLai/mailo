//! The words typed after `watch`, `notify` and `offline`, as the commands they run. What the
//! commands do is `mail-core`'s (`notifications.rs`, `offline_sync.rs`, `sync_path.rs`).

use mail_core::notify;
use mail_core::offline::Keep;

#[test]
fn the_command_line_can_silence_a_watch_and_change_the_setting() {
    use mail_app::cli::{Command, WatchNotify, parse};
    let args = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(
        parse(&args("watch")),
        Ok(Command::Watch {
            notify: WatchNotify::AsSet
        })
    );
    assert_eq!(
        parse(&args("watch --no-notify")),
        Ok(Command::Watch {
            notify: WatchNotify::Never
        })
    );
    assert_eq!(
        parse(&args("notify off")),
        Ok(Command::Notify {
            set: Some(notify::Setting::Off)
        })
    );
    assert_eq!(
        parse(&args("notify on")),
        Ok(Command::Notify {
            set: Some(notify::Setting::On)
        })
    );
    assert_eq!(parse(&args("notify")), Ok(Command::Notify { set: None }));
    assert!(parse(&args("notify loudly")).is_err());
}

#[test]
fn the_command_line_sets_one_account_and_says_where_each_stands() {
    use mail_app::cli::{Command, parse};
    let args = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(
        parse(&args("offline me@example.test on")),
        Ok(Command::Offline {
            address: Some("me@example.test".to_owned()),
            set: Some(Keep::Everything),
        })
    );
    assert_eq!(
        parse(&args("offline me@example.test off")),
        Ok(Command::Offline {
            address: Some("me@example.test".to_owned()),
            set: Some(Keep::Bodies),
        })
    );
    assert_eq!(
        parse(&args("offline")),
        Ok(Command::Offline {
            address: None,
            set: None
        })
    );
    assert!(parse(&args("offline me@example.test always")).is_err());
}
