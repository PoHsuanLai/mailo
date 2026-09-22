//! Why the window stops after its first render — the whole of F140, with no `mail-app` code.
//!
//! ```text
//! cargo run -p mail-app --example rerender
//! ```
//!
//! Expected: twenty ticks from each of three drivers, and a window counting up with them.
//!
//! Observed: **one** wake in total across all three, then silence — while the hidden
//! `#heartbeat` element, clicked by the page every 250 ms, reaches its Rust handler *thirty-eight
//! times*. The loop is alive and dispatching events. It simply never polls the `VirtualDom`
//! again, so the timer that came ready long ago is never resumed and the component never
//! re-renders.
//!
//! That pair of numbers is the finding. It rules out the explanation that looked most likely at
//! first — a compositor throttling a surface nobody brings to the front — because a throttled
//! event loop does not handle thirty-eight clicks.
//!
//! `dioxus-desktop/src/waker.rs` names the mechanism: waking the dom sends
//! `UserWindowEvent::Poll` through tao's `EventLoopProxy`, and `launch.rs` turns that event into
//! `app.poll_vdom(id)`. Nothing else polls it — handling a DOM event does not. So when those
//! proxy events are not delivered, every future stalls and every signal write goes unrendered,
//! and the send result is discarded (`_ = arc_self.proxy.send_event(..)`) so nothing says so.
//!
//! There is no way out from application code: the proxy lives in a `pub(crate)` field.
//!
//! What it costs `mail-app`: the five-minute poll loop never runs a pass, the composer's autosave
//! never fires, and every button updates the database without updating the screen.

use dioxus::prelude::*;

/// A hidden element the page clicks on a timer.
///
/// dioxus re-polls the `VirtualDom` when `EventLoopProxy::send_event` delivers a `Poll` event
/// (see `dioxus-desktop/src/waker.rs`). If those never arrive, nothing after the first wake is
/// ever polled. A real DOM event *does* get through — a click reaches its handler — so this
/// drives the loop through the one door that is known to open.
const HEARTBEAT: &str = r#"<script>
document.addEventListener("DOMContentLoaded", () => {
  setInterval(() => {
    const beat = document.getElementById("heartbeat");
    if (beat) { beat.click(); }
  }, 250);
});
</script>"#;

fn main() {
    dioxus::LaunchBuilder::desktop()
        .with_cfg(dioxus::desktop::Config::new().with_custom_head(HEARTBEAT.to_owned()))
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

    rsx! {
        div { "timer {timer} channel {channel} page {from_page}" }
        // Clicked by the page every 250 ms. The handler does nothing: what matters is that
        // handling an event makes dioxus poll the VirtualDom afterwards.
        div { id: "heartbeat", onclick: move |_| { eprintln!("BEAT-CLICK"); }, style: "display:none" }
    }
}
