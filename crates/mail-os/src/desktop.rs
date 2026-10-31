//! The desktop's notification service as mail-core is handed it. Everywhere but macOS it is the
//! core's own; on macOS the banners are posted, and their clicks heard, by `mail-appkit`, which
//! the core does not reach up to, so this is where the two are joined.

use mail_client::notify::desktop::Desktop;

/// The notification service for a process with no run loop to hear a click on: `mailo watch`.
pub fn connect() -> Desktop {
    #[cfg(target_os = "macos")]
    {
        Desktop::connect(std::sync::Arc::new(macos::Centre))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Desktop::connect()
    }
}

/// As [`connect`], for the window: the process that runs the application's main run loop. On
/// macOS that is what lets a click on a banner be heard, by the one delegate this installs (the
/// first time; later calls leave it be); everywhere else it is the same as `connect`.
pub fn connect_in_window() -> Desktop {
    #[cfg(target_os = "macos")]
    {
        // False when a delegate is installed already, or there is no centre: either way there is
        // nothing more to do.
        let _ = mail_appkit::banner::hear_banners(macos::heard);
    }
    connect()
}

#[cfg(target_os = "macos")]
mod macos {
    use mail_appkit::banner as appkit;
    use mail_client::notify::banner::{Banner, Click, Posts, Pressed};

    /// The notification centre, reached through `mail-appkit`.
    #[derive(Debug)]
    pub struct Centre;

    impl Posts for Centre {
        fn post(&self, banner: &Banner) {
            // False is no centre to post to (no bundle identifier), and nothing to say about that.
            appkit::show(&appkit::Banner {
                title: banner.title.clone(),
                body: banner.body.clone(),
                sound: banner.sound.clone(),
                main_button: banner.main_button.clone(),
                more_buttons: banner.more_buttons.clone(),
                info: banner.info.clone(),
            });
        }
    }

    /// A click from the delegate, handed to the core to start what it asks for.
    pub fn heard(click: appkit::Activation) {
        let pressed = match click.pressed {
            appkit::Pressed::Content => Pressed::Content,
            appkit::Pressed::Main => Pressed::Main,
            appkit::Pressed::More(key) => Pressed::More(key),
            appkit::Pressed::Other => Pressed::Other,
        };
        mail_client::notify::desktop::heard(Click {
            pressed,
            info: click.info,
        });
    }
}
