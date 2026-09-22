use super::app::App;
use crate::view::Appearance;
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

/// A script that stamps the appearance onto `<html>` before the stylesheet is first applied.
///
/// It goes in the custom head, which is inserted before `</head>` and therefore runs ahead
/// of the interpreter module, so the first frame is already in the right palette. Values go
/// through `serde_json::to_string`, the same as `NOTHING_MOUNTED`: an attribute written into
/// a `<script>` is an injection site even when it can only be one of six words.
fn appearance_head(look: Appearance) -> String {
    let quote = |word: &str| {
        serde_json::to_string(word).expect("a &str always serializes") // `&str` serialization cannot fail
    };
    let accent = quote(look.accent.slug());
    let body = match look.theme.attribute() {
        Some(theme) => {
            let theme = quote(theme);
            format!(
                "document.documentElement.dataset.accent = {accent};\n\
                 document.documentElement.dataset.theme = {theme};"
            )
        }
        None => format!("document.documentElement.dataset.accent = {accent};"),
    };
    format!("<script>\n{body}\n</script>")
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

/// Launch the shell.
pub fn run(store: Arc<SqliteStore>) {
    dioxus::LaunchBuilder::desktop()
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
                    appearance_head(Appearance::default()),
                    serde_json::to_string(NOTHING_MOUNTED)
                        .unwrap_or_else(|_| "\"The interface did not start.\"".to_owned()),
                    probe()
                )),
        )
        .with_context(store)
        .launch(App);
}

#[cfg(test)]
mod tests {
    use super::Appearance;
    use super::appearance_head;
    use crate::view::{Accent, Theme};

    #[test]
    fn the_default_appearance_is_stamped_exactly() {
        assert_eq!(
            appearance_head(Appearance::default()),
            "<script>\ndocument.documentElement.dataset.accent = \"postmark\";\n</script>"
        );
    }

    #[test]
    fn a_dark_pine_window_is_stamped_exactly() {
        assert_eq!(
            appearance_head(Appearance {
                theme: Theme::Dark,
                accent: Accent::Pine,
            }),
            "<script>\ndocument.documentElement.dataset.accent = \"pine\";\n\
             document.documentElement.dataset.theme = \"dark\";\n</script>"
        );
    }

    #[test]
    fn system_writes_no_theme_dataset() {
        // Absence is the property: an exact string can grow a comment that names the attribute
        // and still set it. `Theme::System` must not mention `dataset.theme` at all, or the
        // stylesheet's `prefers-color-scheme` guard is unreachable.
        let head = appearance_head(Appearance {
            theme: Theme::System,
            accent: Accent::Vermilion,
        });
        assert!(
            !head.contains("dataset.theme"),
            "a system theme set data-theme, so the desktop can no longer decide: {head}"
        );
    }
}
