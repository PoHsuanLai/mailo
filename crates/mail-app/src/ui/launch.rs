use super::app::App;
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
    // The composer's editor and its fields own the keyboard. Its / and @ menus open beside the
    // caret without taking focus, so they must not be focused here either.
    const here = document.activeElement;
    if (here && here.closest && here.closest(".c-body, .c-props .inp, .c-scroll .inp, .bubble .inp")) { return; }
    // An open menu owns the keyboard. Focusing `.app` here would take it back on the next
    // tick, and the field would lose whatever was just typed.
    const menuField = document.querySelector(".cmdk .inp, .fmenu .inp");
    if (menuField) {
      if (document.activeElement === menuField) { return; }
      menuField.focus();
      return;
    }
    const menu = document.querySelector(".fmenu");
    if (menu) {
      if (document.activeElement === menu) { return; }
      menu.focus();
      return;
    }
    const app = document.querySelector(".app");
    if (!app || document.activeElement === app) { return; }
    // Only when focus is nowhere in particular. Taking it from a text box would make typing
    // impossible, which is a far worse bug than the one this exists to fix.
    if (here && here !== document.body && here !== document.documentElement) { return; }
    // With a draft open, nowhere in particular means the draft.
    const page = document.querySelector(".cpage .c-body, .inline-reply .c-body");
    (page || app).focus();
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

/// The faces the stylesheet names, as `@font-face` rules with the fonts inside them.
///
/// quire's (`ds::font_face_css`, the `webview-fonts` feature): the faces ship in the binary
/// as `data:` URIs, so the window never phones a font host. In the head, once, rather than in
/// the page, where every render would diff a few hundred kilobytes of text it never changes.
fn fonts_head() -> String {
    format!("<style>{}</style>", ds::font_face_css())
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

/// Launch the shell, already wearing `look` and `spaces`, open where `start` says.
pub fn run(
    store: Arc<SqliteStore>,
    look: Appearance,
    spaces: Spaces,
    dirs: Option<WindowDirs>,
    start: super::Start,
) {
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
                    "{}<script>window.__mailo_nothing_mounted = {};</script>{KEEP_FOCUS}{}{}",
                    fonts_head(),
                    serde_json::to_string(NOTHING_MOUNTED)
                        .unwrap_or_else(|_| "\"The interface did not start.\"".to_owned()),
                    super::compose::GLUE,
                    probe()
                )),
        )
        .with_context(store)
        .with_context(look)
        .with_context(spaces)
        .with_context(icons)
        .with_context(start);
    if let Some(dirs) = dirs {
        launch = launch.with_context(dirs);
    }
    launch.launch(ShellRoot);
}

/// Holds the icon cache in a signal so a refresh can replace it, and the window's settings.
///
/// The files were read once, before the first frame. The signal is what a later
/// refresh writes; the chips subscribe to it.
///
/// The settings are quire's: `appearance.toml` and the desktop's preferences, both watched, as
/// one signal `App`'s root reads (`ds_settings::use_environment`). `main` imported
/// `appearance.json` into the TOML file before the window opened. Only the launched window
/// watches; a test renders `App` without this and never touches the real config directory.
#[component]
fn ShellRoot() -> Element {
    let loaded = try_consume_context::<crate::provider::icon::Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    let environment = ds_settings::use_environment(ds_settings::AppName::MAILO);
    use_context_provider(|| environment);
    rsx! { App {} }
}

#[cfg(test)]
mod tests {
    use super::fonts_head;

    #[test]
    fn the_head_carries_every_face_as_an_embedded_woff2() {
        // "wOF2" is the WOFF2 signature; in base64 it begins "d09GMg". A face missing from the
        // head is a silent fallback to the system face that only a screenshot would show.
        let head = fonts_head();
        let faces = head.matches("@font-face").count();
        assert!(faces > 0, "{head:.200}");
        assert_eq!(
            head.matches("url(data:font/woff2;base64,d09GMg").count(),
            faces,
            "a face that is not an embedded woff2"
        );
        // Each `--font-*` stack leads with a face the head declares.
        for family in [ds::Family::Display, ds::Family::Ui, ds::Family::Data] {
            let first = family.stack().split(',').next().unwrap_or("");
            let declared = format!("font-family: {first};");
            assert!(
                head.contains(&declared),
                "{family:?} leads with {first}, not in the head"
            );
        }
    }
}
