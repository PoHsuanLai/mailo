//! Does a dioxus desktop window keep running after its first render?
//!
//! The smallest reproduction of the question F140 turned into. No `mail-app` code takes part:
//! one signal, one timer, one button. Run it and watch the terminal.
//!
//! ```text
//! cargo run -p mail-app --example rerender
//! ```
//!
//! Expected: twenty `TICK` lines, half a second apart, and the window counting up with them.
//!
//! Observed from a shell in this project's development session — a Wayland window that is
//! created but that nobody brings to the front — exactly one `TICK`, then silence, under both
//! `dioxus::launch` and `LaunchBuilder::desktop()`. Whether that is dioxus's run loop stopping or
//! a compositor throttling a surface that is never presented is the open question, and it cannot
//! be settled from a terminal: it needs somebody to look at the window.
//!
//! It matters because the same silence in `mail-app` means the five-minute poll loop never runs
//! a pass and the composer's autosave never fires.
use dioxus::prelude::*;

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    let mut n = use_signal(|| 0);
    use_future(move || async move {
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            eprintln!("TICK");
            n += 1;
        }
    });
    rsx! {
        div { id: "out", "count {n}" }
        button {
            id: "bump",
            onclick: move |_| {
                // Prints whether or not the screen changes, which is the whole distinction: an
                // event that reaches the handler and a window that then redraws are two things.
                eprintln!("CLICK");
                n += 1;
            },
            "bump"
        }
    }
}
