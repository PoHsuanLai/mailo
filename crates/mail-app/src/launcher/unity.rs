//! The `com.canonical.Unity.LauncherEntry` count: what the message says, apart from sending it.
//!
//! The protocol is one signal. An application emits `Update` on the session bus, from any object
//! path, with two arguments: its desktop entry as a URI (`application://<desktop file id>`) and a
//! dictionary of properties, of which the count is `count` (an `int64`) and whether a dock draws
//! it is `count-visible` (a boolean). A dock matches the URI against the entries it shows, and
//! forgets the count when the sender leaves the bus, so a closed mailo leaves no stale number.
//!
//! [`update`] builds the message and is pure; [`Bus`] is the seam it goes out through — the
//! session bus in the window (`launcher/session.rs`), a recorder in the tests.

use super::{Badge, Unread};

/// The interface the signal is emitted on.
pub const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

/// The signal's name.
pub const MEMBER: &str = "Update";

/// The object path it is emitted from. Docks read the interface and the URI, not the path; this
/// is libunity's shape, under mailo's own name.
pub const PATH: &str = "/com/canonical/unity/launcherentry/mailo";

/// The desktop entry mailo installs (`packaging/mailo.desktop`), outside a Flatpak.
pub const DESKTOP_ID: &str = "mailo";

/// One `Update`: the arguments of the signal, before any bus has touched them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    /// `application://<desktop file id>.desktop`.
    pub app_uri: String,
    /// The `count` property.
    pub count: i64,
    /// The `count-visible` property: false hides the badge, which is how zero is said.
    pub count_visible: bool,
}

/// The desktop file id mailo's entry is installed under.
///
/// Outside a sandbox, `mailo`. Inside a Flatpak, the entry is exported renamed to the app id
/// (`io.github.PoHsuanLai.mailo.desktop`, `packaging/flatpak`), and the sandbox names that id in
/// `FLATPAK_ID`, which is what `flatpak_id` is. An empty one is no id.
pub fn desktop_id(flatpak_id: Option<&str>) -> &str {
    match flatpak_id.map(str::trim) {
        Some(id) if !id.is_empty() => id,
        _ => DESKTOP_ID,
    }
}

/// The message that shows `unread` on the launcher entry `desktop_id`.
///
/// Zero hides the badge rather than drawing a 0: an empty inbox is not news. A count beyond
/// `int64` (it cannot happen; SQLite's own counts are `int64`) is shown as the largest one.
pub fn update(desktop_id: &str, unread: Unread) -> Update {
    Update {
        app_uri: format!("application://{desktop_id}.desktop"),
        count: i64::try_from(unread.0).unwrap_or(i64::MAX),
        count_visible: unread.0 > 0,
    }
}

/// Where an [`Update`] goes: the session bus, or a recorder.
///
/// Called on the thread that draws, so the real one only queues the message.
pub trait Bus: Send + Sync {
    fn emit(&self, update: Update);
}

/// The launcher entry as a [`Badge`]: each count becomes an [`Update`] for mailo's entry.
pub struct Unity<B> {
    desktop_id: String,
    bus: B,
}

impl<B> std::fmt::Debug for Unity<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Unity")
            .field("desktop_id", &self.desktop_id)
            .finish_non_exhaustive()
    }
}

impl<B: Bus> Unity<B> {
    pub fn new(desktop_id: String, bus: B) -> Self {
        Unity { desktop_id, bus }
    }
}

impl<B: Bus> Badge for Unity<B> {
    fn show(&self, unread: Unread) {
        self.bus.emit(update(&self.desktop_id, unread));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn a_count_is_an_update_for_the_entry_and_zero_hides_it() {
        const CASES: &[(&str, &str, u64, &str, i64, bool)] = &[
            (
                "nothing unread",
                "mailo",
                0,
                "application://mailo.desktop",
                0,
                false,
            ),
            ("one", "mailo", 1, "application://mailo.desktop", 1, true),
            (
                "many",
                "mailo",
                1234,
                "application://mailo.desktop",
                1234,
                true,
            ),
            (
                "inside the Flatpak",
                "io.github.PoHsuanLai.mailo",
                3,
                "application://io.github.PoHsuanLai.mailo.desktop",
                3,
                true,
            ),
            (
                "beyond int64",
                "mailo",
                u64::MAX,
                "application://mailo.desktop",
                i64::MAX,
                true,
            ),
        ];
        for &(name, id, unread, uri, count, visible) in CASES {
            assert_eq!(
                update(id, Unread(unread)),
                Update {
                    app_uri: uri.to_owned(),
                    count,
                    count_visible: visible,
                },
                "{name}"
            );
        }
    }

    #[test]
    fn the_entry_is_mailo_s_own_or_the_flatpak_s() {
        const CASES: &[(&str, Option<&str>, &str)] = &[
            ("not sandboxed", None, "mailo"),
            (
                "in the Flatpak",
                Some("io.github.PoHsuanLai.mailo"),
                "io.github.PoHsuanLai.mailo",
            ),
            ("an empty FLATPAK_ID", Some(""), "mailo"),
            ("a blank one", Some("  "), "mailo"),
        ];
        for &(name, flatpak, expect) in CASES {
            assert_eq!(desktop_id(flatpak), expect, "{name}");
        }
    }

    /// The installed entry and the Flatpak manifest are what a dock matches the URI against, so
    /// the ids above are read from them rather than trusted.
    #[test]
    fn the_ids_are_the_ones_packaging_installs() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging");
        assert!(
            root.join(format!("{DESKTOP_ID}.desktop")).is_file(),
            "packaging/{DESKTOP_ID}.desktop"
        );
        let manifest = std::fs::read_to_string(root.join("flatpak/io.github.PoHsuanLai.mailo.yml"))
            .expect("the Flatpak manifest");
        assert!(
            manifest
                .lines()
                .any(|line| line.trim() == "app-id: io.github.PoHsuanLai.mailo"),
            "the manifest's app id moved"
        );
        assert!(
            manifest.contains("/app/share/applications/${FLATPAK_ID}.desktop"),
            "the Flatpak no longer exports the entry under its app id"
        );
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Update>>);

    impl Bus for &Recorder {
        fn emit(&self, update: Update) {
            self.0.lock().unwrap().push(update);
        }
    }

    #[test]
    fn each_count_shown_goes_out_as_one_update() {
        let bus = Recorder::default();
        let unity = Unity::new("mailo".to_owned(), &bus);
        unity.show(Unread(5));
        unity.show(Unread(0));
        assert_eq!(
            *bus.0.lock().unwrap(),
            [update("mailo", Unread(5)), update("mailo", Unread(0))]
        );
    }
}
