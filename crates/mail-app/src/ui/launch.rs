use super::app::App;
use super::frame::frame_statements;
use crate::appearance::WindowDirs;
use crate::space::Spaces;
use crate::view::Appearance;
use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;

/// What to put on the page if the interface never mounts.
///
/// It normally does. It does *not* when the process is started with `GDK_BACKEND=x11` on a
/// Wayland session: the window opens, every script runs, `window.onload` fires and the
/// interpreter is ready, and no edits ever arrive, so the mount point stays empty for good —
/// see FINDINGS F107, and F106 for the wrong conclusion that cost.
///
/// A blank window that says nothing is the worst version of that, whatever the cause. This turns
/// it into a window that says the interface did not start, names the one thing known to do it,
/// and lists what works in a terminal meanwhile — which is most of the application.
const NOTHING_MOUNTED: &str = "The interface did not start.\n\n\
     If this process was launched with GDK_BACKEND=x11 on a Wayland session, that is the cause: \
     run it without that variable.\n\n\
     Everything else works from a terminal: `mailo list`, `mailo show <thread>`, \
     `mailo search <words>`, `mailo reply <message>`, `mailo send <draft>`, `mailo sync`.";

/// Keep the app's root focused, so the keyboard has somewhere to land.
///
/// A keydown targets the focused element and bubbles *up*. `body` is that element until
/// something focusable is clicked, and `body` is the root div's parent — so a Rust `onkeydown`
/// on the div is never reached. `tabindex` alone does not fix it; `autofocus` does not either,
/// being a form-control attribute WebKit ignores on a div; and focusing it from Rust needs an
/// async task, which is the one thing that does not work here — a future spawned from a
/// component body is never polled, so neither `spawn` nor `document::eval` nor `use_future` ever
/// runs. Injected into the page head instead, where it needs nothing from Dioxus at all.
pub(super) const KEEP_FOCUS: &str = r#"<script>
document.addEventListener("DOMContentLoaded", () => {
  const hold = () => {
    const app = document.querySelector(".app");
    if (!app || document.activeElement === app) { return; }
    // Only when focus is nowhere in particular. Taking it from a text box would make typing
    // impossible, which is a far worse bug than the one this exists to fix.
    const here = document.activeElement;
    if (here && here !== document.body && here !== document.documentElement) { return; }
    app.focus();
  };
  // On a timer, not once: the first attempt runs before anything is mounted, and a later render
  // can drop focus back to `body` without firing any event that says so.
  setInterval(hold, 250);
  // If nothing has mounted by now, nothing is going to. Say so rather than showing a blank
  // rectangle: the text is set through `textContent`, so it cannot become markup.
  setTimeout(() => {
    const main = document.getElementById("main");
    if (main && main.children.length === 0) {
      const note = document.createElement("pre");
      note.id = "nothing-mounted";
      note.style.cssText = "margin:0;padding:24px;font:14px/1.6 system-ui,sans-serif;white-space:pre-wrap";
      note.textContent = window.__mailo_nothing_mounted || "The interface did not start.";
      main.appendChild(note);
    }
  }, 4000);
});
</script>"#;

/// The statements that stamp `look` onto `<html>`.
///
/// System deletes `data-theme` rather than setting it: the attribute's absence is what
/// lets `prefers-color-scheme` decide, including after a click that leaves Light or Dark.
/// [`appearance_head`] wraps this in `<script>` and a click evals it as it stands, so the
/// first frame and a later change cannot say different things. Values go through
/// `serde_json::to_string`: an attribute written into a script is an injection site even
/// when it can only be one of six words.
pub(super) fn appearance_script(look: Appearance, space: &crate::space::Space) -> String {
    let quote = |word: &str| {
        serde_json::to_string(word).expect("a &str always serializes") // `&str` serialization cannot fail
    };
    let accent = quote(look.accent.slug());
    let motion = quote(look.motion.slug());
    let theme = match look.theme.attribute() {
        Some(theme) => {
            let theme = quote(theme);
            format!("document.documentElement.dataset.theme = {theme};")
        }
        None => "delete document.documentElement.dataset.theme;".to_owned(),
    };
    format!(
        "document.documentElement.dataset.accent = {accent};\n\
         document.documentElement.dataset.motion = {motion};\n\
         {theme}\n{}",
        frame_statements(look, space)
    )
}

/// A script that stamps the appearance onto `<html>` before the stylesheet is first applied.
///
/// It goes in the custom head, which is inserted before `</head>` and therefore runs ahead
/// of the interpreter module, so the first frame is already in the right palette.
fn appearance_head(look: Appearance, space: &crate::space::Space) -> String {
    format!("<script>\n{}\n</script>", appearance_script(look, space))
}

/// Extra markup for the head, from `$MAILO_PROBE`, in debug builds only.
///
/// How this application is verified the way its user runs it. The window has no scripting seam
/// and the desktop has no reliable one — `xdotool` needs `GDK_BACKEND=x11`, which breaks the
/// WebView's own edit delivery, and that mistake cost three rounds and a retracted finding
/// (FINDINGS F106/F107). A page that can dispatch its own events and report to a loopback
/// listener needs neither, and works with the screen locked, which is where the technique earned
/// its place: it is the difference between "I could not look at it" and knowing.
///
/// **Never compiled into a release build.** A mail window that runs script from an environment
/// variable turns control of the environment into the ability to read every message in the
/// store and send it somewhere, which is a real step up from what setting a variable otherwise
/// buys. `cfg(debug_assertions)` is the whole guard: `scripts/live-window.sh` uses the debug
/// binary, and nothing a user installs has this in it at all.
#[cfg(debug_assertions)]
fn probe() -> String {
    std::env::var("MAILO_PROBE").unwrap_or_default()
}

#[cfg(not(debug_assertions))]
fn probe() -> &'static str {
    ""
}

/// Launch the shell, already wearing `look` and `spaces`.
pub fn run(store: Arc<SqliteStore>, look: Appearance, spaces: Spaces, dirs: Option<WindowDirs>) {
    let space = spaces.current_space();
    let icons = crate::appearance::cache_dir()
        .map(|dir| crate::provider::icon::Loaded::read(&dir.join("providers")))
        .unwrap_or_default();
    let mut launch = dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(
                    dioxus::desktop::WindowBuilder::new()
                        .with_title("mailo")
                        .with_inner_size(dioxus::desktop::LogicalSize::new(1200.0, 800.0)),
                )
                .with_menu(None)
                .with_custom_head(format!(
                    "{}<script>window.__mailo_nothing_mounted = {};</script>{KEEP_FOCUS}{}",
                    appearance_head(look, &space),
                    serde_json::to_string(NOTHING_MOUNTED)
                        .unwrap_or_else(|_| "\"The interface did not start.\"".to_owned()),
                    probe()
                )),
        )
        .with_context(store)
        .with_context(look)
        .with_context(spaces)
        .with_context(icons);
    if let Some(dirs) = dirs {
        launch = launch.with_context(dirs);
    }
    launch.launch(ShellRoot);
}

/// Holds the icon cache in a signal so a refresh can replace it.
///
/// The files were read once, before the first frame. The signal is what a later
/// refresh writes; the chips subscribe to it.
#[component]
fn ShellRoot() -> Element {
    let loaded = try_consume_context::<crate::provider::icon::Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    rsx! { App {} }
}

#[cfg(test)]
mod tests {
    use super::Appearance;
    use super::appearance_head;
    use super::appearance_script;
    use crate::palette::{self, Dot};
    use crate::space::Space;
    use crate::view::{Accent, Motion, Theme};

    fn starts_with_look(head: &str, lines: &str) {
        let expect = format!("<script>\n{lines}");
        assert!(
            head.starts_with(&expect),
            "the appearance lines moved:\n{head}"
        );
    }

    #[test]
    fn the_default_appearance_is_stamped_exactly() {
        // The delete is deliberate. System used to omit `dataset.theme` entirely, which is
        // right on first paint and wrong after a click that leaves Light or Dark: the head
        // and the click share [`appearance_script`], so both remove the attribute.
        let head = appearance_head(Appearance::default(), &Space::default());
        starts_with_look(
            &head,
            "document.documentElement.dataset.accent = \"postmark\";\n\
             document.documentElement.dataset.motion = \"standard\";\n\
             delete document.documentElement.dataset.theme;",
        );
    }

    #[test]
    fn a_dark_pine_window_is_stamped_exactly() {
        let head = appearance_head(
            Appearance {
                theme: Theme::Dark,
                accent: Accent::Pine,
                motion: Motion::Extra,
                ..Appearance::default()
            },
            &Space::default(),
        );
        starts_with_look(
            &head,
            "document.documentElement.dataset.accent = \"pine\";\n\
             document.documentElement.dataset.motion = \"extra\";\n\
             document.documentElement.dataset.theme = \"dark\";",
        );
    }

    #[test]
    fn system_writes_no_theme_dataset() {
        // Absence is the property. System deletes the attribute so a previous Light or Dark
        // cannot linger, and it must not assign `dataset.theme` or the stylesheet's
        // `prefers-color-scheme` guard is unreachable.
        let head = appearance_head(
            Appearance {
                theme: Theme::System,
                accent: Accent::Vermilion,
                motion: Motion::default(),
                ..Appearance::default()
            },
            &Space::default(),
        );
        assert!(
            head.contains("delete document.documentElement.dataset.theme"),
            "a system theme left a previous data-theme in place: {head}"
        );
        assert!(
            !head.contains("dataset.theme ="),
            "a system theme set data-theme, so the desktop can no longer decide: {head}"
        );
    }

    #[test]
    fn appearance_script_is_exact() {
        let cases = [
            (
                Appearance {
                    theme: Theme::Dark,
                    accent: Accent::Pine,
                    motion: Motion::Calm,
                    ..Appearance::default()
                },
                "document.documentElement.dataset.accent = \"pine\";\n\
                 document.documentElement.dataset.motion = \"calm\";\n\
                 document.documentElement.dataset.theme = \"dark\";",
            ),
            (
                Appearance {
                    theme: Theme::System,
                    accent: Accent::Graphite,
                    motion: Motion::Standard,
                    ..Appearance::default()
                },
                "document.documentElement.dataset.accent = \"graphite\";\n\
                 document.documentElement.dataset.motion = \"standard\";\n\
                 delete document.documentElement.dataset.theme;",
            ),
        ];
        for (look, lines) in cases {
            let script = appearance_script(look, &Space::default());
            assert!(
                script.starts_with(lines),
                "{look:?} did not start with the appearance lines:\n{script}"
            );
        }
    }

    #[test]
    fn system_deletes_the_theme_and_does_not_set_it() {
        let script = appearance_script(
            Appearance {
                theme: Theme::System,
                accent: Accent::Graphite,
                motion: Motion::default(),
                ..Appearance::default()
            },
            &Space::default(),
        );
        assert!(
            script.contains("delete document.documentElement.dataset.theme"),
            "{script}"
        );
        assert!(
            !script.contains("dataset.theme ="),
            "system set data-theme: {script}"
        );
    }

    #[test]
    fn the_head_script_sets_the_quoted_frame_gradient() {
        // Quoting is the whole assertion. A gradient pasted raw is not a JS string: the
        // commas and parentheses are parsed as arguments, and the token is never set.
        let space = Space {
            dots: vec![
                Dot {
                    hue: 268.0,
                    chroma: 0.72,
                },
                Dot {
                    hue: 318.0,
                    chroma: 0.55,
                },
            ],
            ..Space::default()
        };
        let look = Appearance {
            theme: Theme::Light,
            ..Appearance::default()
        };
        let gradient = palette::gradient(&palette::derive(&space.dots, false));
        let quoted = serde_json::to_string(&gradient).expect("a string serializes");
        let script = appearance_script(look, &space);
        let needle = format!("document.documentElement.style.setProperty(\"--f-grad\", {quoted})");
        assert!(
            script.contains(&needle),
            "the frame gradient was not the quoted palette value:\n{script}"
        );
    }
}
