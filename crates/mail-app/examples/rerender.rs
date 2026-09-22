//! Which wakes reach a dioxus desktop window, and which do not — the minimal case behind F140.
//!
//! ```text
//! cargo run -p mail-app --example rerender
//! ```
//!
//! Expected: twenty ticks from each of three drivers, half a second apart, and a window counting
//! up with them.
//!
//! Observed, from a shell in this project's development session: **one** wake in total, across
//! all three, and then silence. A tokio timer, a wake sent from an ordinary OS thread through a
//! channel, and the page itself calling back through `dioxus::document::eval` all stop the same
//! way, which rules out the time driver and rules out the webview. Adding `with_focused` and
//! `with_always_on_top` changes nothing, and so does `dioxus::launch` in place of
//! `LaunchBuilder::desktop()`.
//!
//! In `mail-app` the same instrumentation shows the shape of it: `App` runs twice, a click on
//! Sync reaches its handler and writes its signal — and no render follows the write. Events
//! still arrive; renders stop. No `mail-app` code takes part in the reproduction below.
use dioxus::prelude::*;

fn main() {
    // Asked to be presented: if the stall is a compositor throttling a surface nobody ever
    // brings to the front, a window that is focused and on top does not stall.
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::WindowBuilder::new()
                    .with_title("rerender")
                    .with_always_on_top(true)
                    .with_focused(true),
            ),
        )
        .launch(app);
}

fn app() -> Element {
    let mut timer = use_signal(|| 0);
    let mut channel = use_signal(|| 0);
    eprintln!("RENDER timer={} channel={}", timer.peek(), channel.peek());

    // A tokio timer.
    use_future(move || async move {
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            eprintln!("TICK");
            timer += 1;
        }
    });

    // A wake from an ordinary OS thread, through a channel. Same shape, different driver.
    use_future(move || async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u32>();
        std::thread::spawn(move || {
            for n in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if tx.send(n).is_err() {
                    return;
                }
            }
        });
        while let Some(n) = rx.recv().await {
            eprintln!("RECV {n}");
            channel += 1;
        }
    });

    // A third driver: the page itself, calling back into Rust on an interval. Webview IPC is
    // the one kind of event this window is known to receive — a click reached its handler.
    let mut from_page = use_signal(|| 0);
    use_future(move || async move {
        let mut beat =
            dioxus::document::eval(r#"setInterval(() => dioxus.send(Date.now()), 500);"#);
        loop {
            match beat.recv::<f64>().await {
                Ok(_) => {
                    eprintln!("BEAT");
                    from_page += 1;
                }
                Err(e) => {
                    eprintln!("BEAT ended: {e:?}");
                    return;
                }
            }
        }
    });

    rsx! { div { "timer {timer} channel {channel} page {from_page}" } }
}
