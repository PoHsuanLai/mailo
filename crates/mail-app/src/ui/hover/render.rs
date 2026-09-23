//! Each hover card open, written to `target/` for a person or a headless browser to look at.
//!
//! The cards are opened the way the window opens them: a pointer event on the hook, then the
//! timer's rest. The page has no layout of its own here, so a small script places each card
//! against its hook with the same arithmetic the window uses (`cards.rs`), which is only in the
//! file, never in the app.

use crate::trust::destination;
use crate::ui::app::App;
use crate::ui::fixtures::{FakePointer, dispatching, pointer, rebuild_into, thread_like, work};
use crate::ui::paint::appearance_script;
use crate::ui::style::STYLE;
use crate::view::Theme;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};

async fn wait(dom: &mut VirtualDom, for_ms: u64) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_millis(for_ms);
    loop {
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = tokio::time::timeout(left, dom.wait_for_work()).await;
        dom.render_immediate(&mut NoOpMutations);
    }
}

const RESTING: FakePointer = FakePointer {
    client: (400.0, 120.0),
    offset: (10.0, 10.0),
    held: false,
};

/// Place the open card against `[data-hc=hook]`, as `HoverLayer` does from the pointer.
fn placing(hook: &str, kind: &str) -> String {
    format!(
        "addEventListener('load',()=>{{const h=document.querySelector('[data-hc=\"{hook}\"]');\
         const c=document.querySelector('.hc');if(!h||!c)return;const r=h.getBoundingClientRect();\
         const k='{kind}';if(k==='thread'){{c.style.top=Math.max(8,r.top-12)+'px';}}\
         else if(k==='sender'){{c.style.left=r.left+'px';c.style.top=(r.top+22)+'px';}}\
         else if(k==='time'){{c.style.left=Math.max(8,r.left-140)+'px';c.style.top=(r.top+20)+'px';}}\
         else{{c.style.left='244px';c.style.top=Math.max(8,r.top-6)+'px';}}}});"
    )
}

fn write(name: &str, body: &str, extra_script: &str) {
    let built_space = crate::space::Space::default();
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    for (suffix, theme) in [("", Theme::Light), ("-dark", Theme::Dark)] {
        let script = appearance_script(&crate::space::Space {
            theme,
            ..built_space.clone()
        });
        let theme_attr = theme
            .attribute()
            .map(|value| format!(" data-theme=\"{value}\""))
            .unwrap_or_default();
        let page = format!(
            "<!doctype html>\n<html lang=\"en\"{theme_attr}><head><meta charset=\"utf-8\">\
             <style>{STYLE}</style><script>{script}</script><script>{extra_script}</script></head>\
             <body>{body}</body></html>\n"
        );
        let out = target.join(format!("{name}{suffix}.html"));
        std::fs::write(&out, page).unwrap();
        println!("wrote {}", out.display());
    }
}

/// A reader with the link pill showing, for each way a link can read.
#[component]
fn Pills(which: usize) -> Element {
    let mut hover = super::use_hover();
    let links = [
        (
            "rfc-editor.org",
            "https://www.rfc-editor.org/rfc/rfc1939#section-7",
        ),
        ("google.com", "https://g00gle-security.xyz/verify"),
    ];
    let (text, href) = links[which % links.len()];
    use_hook(move || hover.link.set(Some(destination(text, href))));
    rsx! {
        section { class: "reader", style: "height:120px;width:520px;margin:20px",
            super::LinkPill {}
        }
    }
}

#[tokio::test]
#[ignore = "writes target/hover-*.html for a person or a headless browser to look at"]
async fn render_the_hover_cards_to_a_file() {
    dispatching();
    for kind in ["thread", "sender", "time", "pin", "today"] {
        // Thread ids are fresh in every fixture, so the hooks are named against this one.
        let built = work();
        let dana = built.dana;
        let spoof = thread_like(&built.store, "Unusual sign-in attempt blocked");
        let hooks = match kind {
            "thread" => vec![format!("thread:{dana}")],
            "sender" => vec![format!("thread:{spoof}"), format!("sender:{spoof}")],
            "time" => vec![format!("thread:{dana}"), format!("time:{dana}")],
            "pin" => vec!["pin:0".to_owned()],
            _ => vec![format!("today:{dana}")],
        };
        let mut dom = VirtualDom::new(App)
            .with_root_context(built.store.clone())
            .with_root_context(built.dirs.clone());
        let seen = rebuild_into(&mut dom);
        for (step, hook) in hooks.iter().enumerate() {
            let element = seen.one("data-hc", hook);
            if step == 0 {
                pointer(&mut dom, "pointerenter", element, RESTING.clone());
            }
            pointer(&mut dom, "pointerover", element, RESTING.clone());
        }
        wait(&mut dom, 700).await;
        let body = dioxus_ssr::render(&dom);
        assert!(body.contains("role=\"tooltip\""), "no {kind} card opened");
        let last = hooks.last().cloned().unwrap_or_default();
        write(&format!("hover-{kind}"), &body, &placing(&last, kind));
    }

    // The motion that has a resting picture: the toast after an archive, and a drag held over
    // Archive with the ghost under the pointer.
    let built = work();
    let dana = built.dana;
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let build_bot = seen.all("aria-label", "Archive")[3];
    crate::ui::fixtures::click(&mut dom, build_bot);
    wait(&mut dom, 200).await;
    let row = seen.one("data-hc", &format!("thread:{dana}"));
    let archive = seen.one("data-place", "Archive");
    let held = |x: f64, y: f64| FakePointer {
        client: (x, y),
        offset: (10.0, 10.0),
        held: true,
    };
    pointer(&mut dom, "pointerdown", row, held(420.0, 140.0));
    pointer(&mut dom, "pointermove", row, held(160.0, 262.0));
    pointer(&mut dom, "pointerenter", archive, held(160.0, 262.0));
    let body = dioxus_ssr::render(&dom);
    assert!(body.contains("ghost-row") && body.contains("class=\"toast\""));
    write("hover-motion", &body, "");

    let mut pills = String::new();
    for which in 0..2 {
        let mut dom = VirtualDom::new_with_props(Pills, PillsProps { which });
        dom.rebuild_in_place();
        pills.push_str(&dioxus_ssr::render(&dom));
    }
    write("hover-links", &pills, "");
}
