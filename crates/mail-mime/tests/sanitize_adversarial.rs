//! An independent adversarial pass over `sanitize`, written against the delegated
//! implementation rather than alongside it.
//!
//! The suite that ships with a sanitizer tends to test the vectors its author thought of. These
//! are deliberately different payloads: obfuscation that defeats string scanning, mutation-XSS
//! shapes that exploit the parser rather than the filter, and every attribute that makes a
//! browser fetch something without the reader clicking.

use mail_mime::{RemoteImages, SanitizePolicy, sanitize};

fn blocked() -> SanitizePolicy {
    SanitizePolicy::CURRENT
}

fn allowed() -> SanitizePolicy {
    SanitizePolicy {
        remote_images: RemoteImages::Allowed,
        version: SanitizePolicy::CURRENT.version,
    }
}

/// No payload may leave behind anything that executes.
#[test]
fn nothing_executable_survives() {
    const PAYLOADS: &[(&str, &str)] = &[
        ("plain script", "<script>alert(1)</script>"),
        ("img onerror", "<img src=x onerror=alert(1)>"),
        ("svg onload", "<svg onload=alert(1)>"),
        ("body onload", "<body onload=alert(1)>x</body>"),
        (
            "details ontoggle",
            "<details ontoggle=alert(1) open>x</details>",
        ),
        ("javascript href", r#"<a href="javascript:alert(1)">x</a>"#),
        ("uppercase scheme", r#"<a href="jAvAsCrIpT:alert(1)">x</a>"#),
        ("leading space", r#"<a href=" javascript:alert(1)">x</a>"#),
        ("embedded tab", "<a href=\"java\tscript:alert(1)\">x</a>"),
        (
            "embedded newline",
            "<a href=\"java\nscript:alert(1)\">x</a>",
        ),
        (
            "entity encoded",
            r#"<a href="&#106;avascript:alert(1)">x</a>"#,
        ),
        ("hex entity", r#"<a href="&#x6a;avascript:alert(1)">x</a>"#),
        (
            "colon entity",
            r#"<a href="javascript&colon;alert(1)">x</a>"#,
        ),
        ("vbscript", r#"<a href="vbscript:msgbox(1)">x</a>"#),
        (
            "data html",
            r#"<a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==">x</a>"#,
        ),
        (
            "iframe srcdoc",
            r#"<iframe srcdoc="<script>alert(1)</script>"></iframe>"#,
        ),
        (
            "object data",
            r#"<object data="javascript:alert(1)"></object>"#,
        ),
        ("embed src", r#"<embed src="javascript:alert(1)">"#),
        (
            "form formaction",
            r#"<form><button formaction="javascript:alert(1)">x</button></form>"#,
        ),
        (
            "meta refresh",
            r#"<meta http-equiv="refresh" content="0;url=javascript:alert(1)">"#,
        ),
        (
            "base tag",
            r#"<base href="javascript:"><a href="alert(1)">x</a>"#,
        ),
        // Mutation XSS: the payload is inert until a browser re-parses the serialized output.
        (
            "noscript mXSS",
            r#"<noscript><p title="</noscript><img src=x onerror=alert(1)>">"#,
        ),
        (
            "style mXSS",
            r#"<style><img src=x onerror=alert(1)></style>"#,
        ),
        (
            "textarea mXSS",
            r#"<textarea><img src=x onerror=alert(1)></textarea>"#,
        ),
        (
            "svg foreignObject",
            r#"<svg><foreignObject><script>alert(1)</script></foreignObject></svg>"#,
        ),
        (
            "xlink href",
            r#"<svg><use xlink:href="javascript:alert(1)"/></svg>"#,
        ),
        (
            "css expression",
            r#"<div style="width:expression(alert(1))">x</div>"#,
        ),
        (
            "import in style",
            r#"<style>@import url("javascript:alert(1)");</style>"#,
        ),
    ];

    for (name, payload) in PAYLOADS {
        let out = sanitize(payload, blocked()).as_str().to_ascii_lowercase();

        // Only MARKUP can execute. Escaped text such as `&lt;img onerror=...&gt;` renders as
        // literal characters and is inert, so scanning the raw output for "alert(" reports a
        // vulnerability where there is none. Scan the tags instead.
        let markup = tags_of(&out);
        for forbidden in [
            "javascript",
            "vbscript",
            "onerror",
            "onload",
            "ontoggle",
            "expression(",
        ] {
            assert!(
                !markup.contains(forbidden),
                "{name}: {forbidden:?} survived inside live markup\n  in:  {payload}\n  out: {out}"
            );
        }
        for tag in [
            "<script", "<iframe", "<object", "<embed", "<form", "<meta", "<base", "<style",
        ] {
            assert!(!out.contains(tag), "{name}: {tag} survived\n  out: {out}");
        }
    }
}

/// The parts of `html` that a browser parses as tags, joined.
///
/// Everything outside a tag is text: it may legitimately contain `alert(` or `javascript:` as
/// characters on the page, which is exactly what a sanitizer neutralising a payload produces.
fn tags_of(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                inside = true;
                out.push(ch);
            }
            '>' if inside => {
                inside = false;
                out.push(ch);
            }
            _ if inside => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Every attribute a browser fetches without the reader doing anything. A remote image is a
/// read receipt the sender never asked permission for.
#[test]
fn nothing_fetches_the_network_when_remote_images_are_blocked() {
    const FETCHERS: &[(&str, &str)] = &[
        ("img src", r#"<img src="https://tracker.test/pixel.gif">"#),
        (
            "img srcset",
            r#"<img srcset="https://tracker.test/1x.png 1x, https://tracker.test/2x.png 2x">"#,
        ),
        (
            "video poster",
            r#"<video poster="https://tracker.test/p.jpg"></video>"#,
        ),
        (
            "body background",
            r#"<body background="https://tracker.test/b.gif">x</body>"#,
        ),
        (
            "table background",
            r#"<table background="https://tracker.test/t.gif"><tr><td>x</td></tr></table>"#,
        ),
        (
            "input src",
            r#"<input type="image" src="https://tracker.test/i.gif">"#,
        ),
        (
            "link stylesheet",
            r#"<link rel="stylesheet" href="https://tracker.test/s.css">"#,
        ),
        (
            "style background",
            r#"<div style="background:url(https://tracker.test/x)">y</div>"#,
        ),
        (
            "svg image href",
            r#"<svg><image href="https://tracker.test/s.png"/></svg>"#,
        ),
        (
            "http not just https",
            r#"<img src="http://tracker.test/pixel.gif">"#,
        ),
    ];

    for (name, payload) in FETCHERS {
        let out = sanitize(payload, blocked()).as_str().to_ascii_lowercase();
        assert!(
            !out.contains("tracker.test"),
            "{name}: a remote fetch survived blocking\n  in:  {payload}\n  out: {out}"
        );
    }
}

/// Blocking remote images must not break inline images, which is how ordinary mail works.
#[test]
fn cid_images_survive_in_both_modes() {
    for (mode, policy) in [("blocked", blocked()), ("allowed", allowed())] {
        let out = sanitize(r#"<img src="cid:part1.abc@example.test">"#, policy);
        assert!(
            out.as_str().contains("cid:part1.abc@example.test"),
            "{mode}: a cid image is local and must survive\n  out: {}",
            out.as_str()
        );
    }
}

/// Opting in must actually opt in, or the setting is a lie.
#[test]
fn allowed_mode_keeps_remote_images_but_still_blocks_scripts() {
    let out = sanitize(
        r#"<img src="https://cdn.test/logo.png"><script>alert(1)</script>"#,
        allowed(),
    );
    assert!(
        out.as_str().contains("cdn.test/logo.png"),
        "opt-in must work"
    );
    assert!(
        !out.as_str().contains("alert"),
        "opting into images is not opting into scripts"
    );
}

/// Over-stripping is its own failure: mail nobody can read is not safe, it is broken.
#[test]
fn ordinary_mail_is_still_readable() {
    let out = sanitize(
        r#"<p>Hi <b>Ada</b>,</p><ul><li>one</li><li>two</li></ul>
           <blockquote>quoted</blockquote>
           <a href="https://example.test/doc">the doc</a>
           <table><tr><td>cell</td></tr></table>"#,
        blocked(),
    );
    let s = out.as_str();
    for kept in [
        "<p>",
        "<b>",
        "<ul>",
        "<li>",
        "<blockquote>",
        "example.test/doc",
        "cell",
    ] {
        assert!(
            s.contains(kept),
            "{kept} should have been preserved\n  out: {s}"
        );
    }
    assert!(s.contains("noopener"), "links need rel=noopener");
    assert!(s.contains("_blank"), "links need target=_blank");
}

/// A mail body is attacker-controlled bytes; none of these may panic or hang.
#[test]
fn hostile_shapes_do_not_panic() {
    let deep = "<div>".repeat(500) + "x" + &"</div>".repeat(500);
    let unclosed = "<div><span><b>".repeat(200);
    for payload in [
        "",
        "<",
        "<<<<>>>>",
        "<a href=",
        "\u{0}<script>alert(1)</script>",
        "<img src=\"\u{0}javascript:alert(1)\">",
        &deep,
        &unclosed,
        &"a".repeat(100_000),
    ] {
        let out = sanitize(payload, blocked());
        assert!(!out.as_str().to_ascii_lowercase().contains("alert("));
    }
}
