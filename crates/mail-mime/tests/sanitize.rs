//! HTML from strangers, rendered in a WebView. Every dangerous token must be gone.

use mail_mime::{RemoteImages, SanitizePolicy, sanitize};

struct Case {
    name: &'static str,
    html: &'static str,
    images: RemoteImages,
    /// Substrings the sanitized markup must still contain.
    keep: &'static [&'static str],
    /// Substrings that must not survive, compared case-insensitively.
    drop: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        name: "script element",
        html: "<p>ok</p><script>alert(1)</script>",
        images: RemoteImages::Blocked,
        keep: &["<p>", "ok"],
        drop: &["script", "alert"],
    },
    Case {
        name: "img onerror",
        html: "<img src=\"x\" onerror=\"alert(1)\" alt=\"pic\">",
        images: RemoteImages::Blocked,
        keep: &["pic"],
        drop: &["onerror", "alert"],
    },
    Case {
        name: "javascript href",
        html: "<a href=\"javascript:alert(1)\">go</a>",
        images: RemoteImages::Blocked,
        keep: &["go"],
        drop: &["javascript", "alert"],
    },
    Case {
        name: "mixed-case javascript href",
        html: "<a href=\"JaVaScRiPt:alert(1)\">go</a>",
        images: RemoteImages::Blocked,
        keep: &["go"],
        drop: &["javascript", "alert"],
    },
    Case {
        name: "entity-encoded javascript href",
        html: "<a href=\"&#106;avascript&#58;alert(1)\">go</a>",
        images: RemoteImages::Blocked,
        keep: &["go"],
        drop: &["javascript", "alert"],
    },
    Case {
        name: "javascript href with an embedded newline",
        html: "<a href=\"java&#10;script:alert(1)\">go</a>",
        images: RemoteImages::Blocked,
        keep: &["go"],
        drop: &["javascript", "alert"],
    },
    Case {
        name: "javascript href with an embedded NUL",
        html: "<a href=\"java&#0;script:alert(1)\">go</a>",
        images: RemoteImages::Blocked,
        keep: &["go"],
        drop: &["javascript", "alert"],
    },
    Case {
        name: "svg onload",
        html: "<svg onload=\"alert(1)\"></svg>",
        images: RemoteImages::Blocked,
        keep: &[],
        drop: &["svg", "onload", "alert"],
    },
    Case {
        name: "data text/html href",
        html: "<a href=\"data:text/html,<script>alert(1)</script>\">go</a>",
        images: RemoteImages::Blocked,
        keep: &["go"],
        drop: &["data:text/html", "alert", "script"],
    },
    Case {
        name: "iframe",
        html: "<iframe src=\"https://evil.test/frame\"></iframe><p>ok</p>",
        images: RemoteImages::Blocked,
        keep: &["<p>", "ok"],
        drop: &["iframe", "evil.test"],
    },
    Case {
        name: "css url in a style attribute",
        html: "<div style=\"background:url(https://tracker.test/x)\">Hi</div>",
        images: RemoteImages::Blocked,
        keep: &["Hi"],
        drop: &["tracker.test", "url("],
    },
    Case {
        name: "css url in a style element",
        html: "<style>body{background:url(https://tracker.test/x)}</style><p>Hi</p>",
        images: RemoteImages::Blocked,
        keep: &["<p>", "Hi"],
        drop: &["tracker.test", "style", "url("],
    },
    Case {
        name: "css url still blocked when images are allowed",
        html: "<div style=\"background:url(https://tracker.test/x)\">Hi</div>",
        images: RemoteImages::Allowed,
        keep: &["Hi"],
        drop: &["tracker.test", "url("],
    },
    Case {
        name: "remote img blocked",
        html: "<img src=\"http://tracker.test/a.png\" alt=\"one\"><img src=\"https://tracker.test/b.png\" alt=\"two\">",
        images: RemoteImages::Blocked,
        keep: &["one", "two"],
        drop: &["tracker.test"],
    },
    Case {
        name: "remote img allowed",
        html: "<img src=\"http://cdn.test/a.png\" alt=\"one\"><img src=\"https://cdn.test/b.png\" alt=\"two\">",
        images: RemoteImages::Allowed,
        keep: &["http://cdn.test/a.png", "https://cdn.test/b.png", "one"],
        drop: &[],
    },
    Case {
        name: "remote srcset blocked, cid src kept",
        html: "<img src=\"cid:logo@mail.test\" srcset=\"https://tracker.test/a.png 1x, https://tracker.test/b.png 2x\" alt=\"inline\">",
        images: RemoteImages::Blocked,
        keep: &["cid:logo@mail.test", "inline"],
        drop: &["tracker.test", "srcset"],
    },
    Case {
        name: "cid img allowed mode",
        html: "<img src=\"cid:logo@mail.test\" alt=\"inline\">",
        images: RemoteImages::Allowed,
        keep: &["cid:logo@mail.test"],
        drop: &[],
    },
    Case {
        name: "protocol-relative img blocked",
        html: "<img src=\"//tracker.test/p.png\" alt=\"pic\">",
        images: RemoteImages::Blocked,
        keep: &["pic"],
        drop: &["tracker.test"],
    },
    Case {
        name: "https link gains rel and target",
        html: "<a href=\"https://example.com/path\">docs</a>",
        images: RemoteImages::Blocked,
        keep: &[
            "href=\"https://example.com/path\"",
            "rel=\"noopener noreferrer\"",
            "target=\"_blank\"",
            "docs",
        ],
        drop: &[],
    },
    Case {
        name: "mailto link kept",
        html: "<a href=\"mailto:ada@example.test\">ada</a>",
        images: RemoteImages::Blocked,
        keep: &[
            "mailto:ada@example.test",
            "rel=\"noopener noreferrer\"",
            "target=\"_blank\"",
        ],
        drop: &[],
    },
    Case {
        name: "ordinary formatting",
        html: "<p>Hello <b>there</b></p><ul><li>one</li></ul>",
        images: RemoteImages::Blocked,
        keep: &["<p>", "<b>", "there", "</b>", "<ul>", "<li>", "one"],
        drop: &[],
    },
    Case {
        name: "form input button",
        html: "<form action=\"https://tracker.test/post\"><input type=\"text\" value=\"secret\"><button onclick=\"alert(1)\">Send</button></form>",
        images: RemoteImages::Blocked,
        keep: &["Send"],
        drop: &[
            "form",
            "input",
            "button",
            "onclick",
            "alert",
            "tracker.test",
            "secret",
        ],
    },
    Case {
        name: "base meta link",
        html: "<base href=\"https://tracker.test/\"><meta http-equiv=\"refresh\" content=\"0;url=https://tracker.test/m\"><link rel=\"stylesheet\" href=\"https://tracker.test/a.css\"><p>Hi</p>",
        images: RemoteImages::Blocked,
        keep: &["<p>", "Hi"],
        drop: &["base", "meta", "stylesheet", "tracker.test"],
    },
    Case {
        name: "object and embed",
        html: "<object data=\"https://tracker.test/o\"></object><embed src=\"https://tracker.test/e\">",
        images: RemoteImages::Blocked,
        keep: &[],
        drop: &["object", "embed", "tracker.test"],
    },
    Case {
        name: "poster and background",
        html: "<video poster=\"https://tracker.test/p.jpg\"></video><table background=\"https://tracker.test/bg.jpg\"><tr><td>cell</td></tr></table>",
        images: RemoteImages::Blocked,
        keep: &["cell"],
        drop: &["poster", "tracker.test", "video"],
    },
    Case {
        name: "javascript still blocked when images are allowed",
        html: "<a href=\"javascript:alert(1)\"><img src=\"https://cdn.test/a.png\" alt=\"pic\"></a>",
        images: RemoteImages::Allowed,
        keep: &["https://cdn.test/a.png"],
        drop: &["javascript", "alert"],
    },
    // Blocking remote images must not break inline images, which is how ordinary mail works.
    Case {
        name: "cid image survives blocked mode",
        html: r#"<img src="cid:part1.abc@example.test">"#,
        images: RemoteImages::Blocked,
        keep: &["cid:part1.abc@example.test"],
        drop: &[],
    },
    Case {
        name: "cid image survives allowed mode",
        html: r#"<img src="cid:part1.abc@example.test">"#,
        images: RemoteImages::Allowed,
        keep: &["cid:part1.abc@example.test"],
        drop: &[],
    },
    // Opting in must actually opt in, or the setting is a lie; and opting into images is not
    // opting into scripts.
    Case {
        name: "allowed mode keeps remote images but still blocks scripts",
        html: r#"<img src="https://cdn.test/logo.png"><script>alert(1)</script>"#,
        images: RemoteImages::Allowed,
        keep: &["cdn.test/logo.png"],
        drop: &["alert"],
    },
    // Over-stripping is its own failure: mail nobody can read is not safe, it is broken.
    Case {
        name: "ordinary mail is still readable",
        html: r#"<p>Hi <b>Ada</b>,</p><ul><li>one</li><li>two</li></ul>
           <blockquote>quoted</blockquote>
           <a href="https://example.test/doc">the doc</a>
           <table><tr><td>cell</td></tr></table>"#,
        images: RemoteImages::Blocked,
        keep: &[
            "<p>",
            "<b>",
            "<ul>",
            "<li>",
            "<blockquote>",
            "example.test/doc",
            "cell",
            "noopener",
            "_blank",
        ],
        drop: &[],
    },
];

#[test]
fn policy_strips_active_content_and_gates_remote_images() {
    assert_eq!(
        SanitizePolicy::CURRENT.remote_images,
        RemoteImages::Blocked,
        "the default policy blocks remote images"
    );
    for case in CASES {
        let policy = SanitizePolicy {
            remote_images: case.images,
            ..SanitizePolicy::CURRENT
        };
        let out = sanitize(case.html, policy);
        let markup = out.as_str();
        let folded = markup.to_ascii_lowercase();
        for token in case.drop {
            assert!(
                !folded.contains(&token.to_ascii_lowercase()),
                "{}: {markup:?} still contains {token:?}",
                case.name
            );
        }
        for token in case.keep {
            assert!(
                markup.contains(token),
                "{}: {markup:?} is missing {token:?}",
                case.name
            );
        }
    }
}

/// What was blocked, which the caller needs to know and could not ask.
///
/// The reader offered "Load remote images" above every conversation in the mailbox — on
/// plain-text mail, on mail with no images at all, on mail whose only image is its own inline
/// part. An offer that is always there is furniture, and a security control that has become
/// furniture is not a control. `sanitize` is the only thing that knows whether it dropped
/// anything, so it is the only thing that can say.
mod what_was_blocked {
    use super::*;

    fn blocked(html: &str, images: RemoteImages) -> u32 {
        sanitize(
            html,
            SanitizePolicy {
                remote_images: images,
                ..SanitizePolicy::CURRENT
            },
        )
        .blocked_remote()
    }

    /// `(name, html, images, count)`. A link is a click, not a fetch; a `cid:` image is this
    /// message's own bytes; a `javascript:` or `data:` src is dropped too, and offering to load
    /// it would put a button in front of a user whose only possible answer makes things worse.
    const CASES: &[(&str, &str, RemoteImages, u32)] = &[
        (
            "a dropped remote image is counted",
            r#"<p>hi</p><img src="https://tracker.test/pixel.gif">"#,
            RemoteImages::Blocked,
            1,
        ),
        (
            "each dropped remote image is counted",
            r#"<img src="https://a.test/1.png"><img src="http://b.test/2.png">"#,
            RemoteImages::Blocked,
            2,
        ),
        (
            // Under `Allowed` the URLs are kept, so there is nothing to offer to load.
            "nothing is blocked when images are allowed",
            r#"<img src="https://tracker.test/pixel.gif">"#,
            RemoteImages::Allowed,
            0,
        ),
        (
            // The common case, and the one that put the button in front of the user for nothing.
            "just words",
            "<p>just words</p>",
            RemoteImages::Blocked,
            0,
        ),
        (
            "a link is a click, not a fetch",
            r#"<a href="https://example.test">a link</a>"#,
            RemoteImages::Blocked,
            0,
        ),
        (
            "an inline part is this message's own bytes and is never blocked",
            r#"<img src="cid:logo@example">"#,
            RemoteImages::Blocked,
            0,
        ),
        (
            "a javascript src is not something the reader could load",
            r#"<img src="javascript:alert(1)">"#,
            RemoteImages::Blocked,
            0,
        ),
        (
            "a data src is not something the reader could load",
            r#"<img src="data:text/html,<script>alert(1)</script>">"#,
            RemoteImages::Blocked,
            0,
        ),
        (
            "a src that is not a url is not something the reader could load",
            r#"<img src="not a url at all">"#,
            RemoteImages::Blocked,
            0,
        ),
    ];

    #[test]
    fn blocked_remote_counts_only_what_a_reader_could_load() {
        for (name, html, images, count) in CASES {
            assert_eq!(blocked(html, *images), *count, "{name}");
        }
    }
}

/// What the sanitized markup will fetch, as the sanitizer kept it.
///
/// The reader's Original frame on Blitz fetches nothing the app does not answer, and the app
/// answers only what this list names for the message the reader consented to. So the list must
/// be exactly the kept fetch attributes: a link is a click, not a fetch, and `cid:` is the
/// message's own part.
mod what_will_be_fetched {
    use super::*;

    fn fetches(html: &str, images: RemoteImages) -> Vec<String> {
        sanitize(
            html,
            SanitizePolicy {
                remote_images: images,
                ..SanitizePolicy::CURRENT
            },
        )
        .remote_fetches()
        .to_vec()
    }

    const BODY: &str = r#"<p><a href="https://shop.test/offer">Offer</a></p>
        <img src="https://cdn.test/a.png"><img src="cid:logo@here">
        <img src="http://cdn.test/b.png?x=1&amp;y=2"><img src="https://cdn.test/a.png">
        <img src="javascript:alert(1)"><img src="file:///etc/passwd">"#;

    #[test]
    fn remote_fetches_list_the_kept_images_once_and_change_nothing() {
        assert!(
            fetches(BODY, RemoteImages::Blocked).is_empty(),
            "blocked: nothing is fetched while remote images are blocked"
        );

        assert_eq!(
            fetches(BODY, RemoteImages::Allowed),
            vec![
                "https://cdn.test/a.png".to_owned(),
                "http://cdn.test/b.png?x=1&y=2".to_owned(),
            ],
            "allowed: each kept image once, and nothing else"
        );

        let policy = SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            ..SanitizePolicy::CURRENT
        };
        let safe = sanitize(BODY, policy);
        for url in safe.remote_fetches() {
            let written = url.replace('&', "&amp;");
            assert!(
                safe.as_str().contains(&written),
                "markup: {url} not in {}",
                safe.as_str()
            );
        }
        assert!(
            !safe.as_str().contains("javascript:"),
            "markup: javascript:"
        );
        assert!(!safe.as_str().contains("file:"), "markup: file:");
    }
}

/// The reader's frame keeps the sender's layout: sheets, classes, presentational attributes.
/// Whatever could fetch or run is gone from it all the same.
mod kept_styles {
    use mail_mime::{RemoteImages, SanitizePolicy, Styles, sanitize};

    fn kept(html: &str) -> String {
        let policy = SanitizePolicy {
            styles: Styles::Kept,
            ..SanitizePolicy::CURRENT
        };
        sanitize(html, policy).as_str().to_owned()
    }

    #[test]
    fn the_layout_survives_and_nothing_that_fetches_or_runs_does() {
        const CASES: &[(&str, &str, &[&str], &[&str])] = &[
            (
                "a sheet and the classes it styles",
                "<style>.wrap{max-width:600px;margin:0 auto}</style><div class=\"wrap\" id=\"top\">Hi</div>",
                &[
                    "<style>.wrap{max-width:600px;margin:0 auto}</style>",
                    "class=\"wrap\"",
                    "id=\"top\"",
                ],
                &[],
            ),
            (
                "a newsletter's table attributes",
                "<table width=\"600\" cellpadding=\"24\" bgcolor=\"#1d3557\" align=\"center\"><tr><td valign=\"top\" style=\"color:#fff\">x</td></tr></table>",
                &[
                    "width=\"600\"",
                    "cellpadding=\"24\"",
                    "bgcolor=\"#1d3557\"",
                    "align=\"center\"",
                    "valign=\"top\"",
                    "style=\"color:#fff\"",
                ],
                &[],
            ),
            (
                "a tracker in a sheet",
                "<style>body{background:url(https://tracker.test/x)}@import 'https://tracker.test/s.css';</style><p>Hi</p>",
                &["body{background:none}"],
                &["tracker.test", "url(", "@import"],
            ),
            (
                "a tracker in a style attribute",
                "<div style=\"background:url(https://tracker.test/x)\">Hi</div>",
                &["style=\"background:none\""],
                &["tracker.test"],
            ),
            (
                "an escaped url in a sheet",
                "<style>p{background:u\\72l(https://tracker.test/x)}</style>",
                &["<style>"],
                &["\\", "url("],
            ),
            (
                "a background attribute",
                "<table background=\"https://tracker.test/bg.png\"><tr><td>x</td></tr></table>",
                &["<table"],
                &["tracker.test", "background"],
            ),
            (
                "markup inside a sheet",
                "<style><img src=x onerror=alert(1)></style><p>ok</p>",
                &["<p>ok</p>"],
                &["<img"],
            ),
            (
                "script and handlers still go",
                "<p onclick=\"alert(1)\">ok</p><script>alert(2)</script><link rel=\"stylesheet\" href=\"https://tracker.test/a.css\">",
                &["ok"],
                &["onclick", "script", "alert", "tracker.test"],
            ),
        ];
        for (name, html, keep, drop) in CASES {
            let out = kept(html);
            let folded = out.to_ascii_lowercase();
            for token in *keep {
                assert!(out.contains(token), "{name}: {out:?} lost {token:?}");
            }
            for token in *drop {
                assert!(
                    !folded.contains(&token.to_ascii_lowercase()),
                    "{name}: {out:?} still has {token:?}"
                );
            }
        }
    }

    #[test]
    fn the_default_still_drops_the_sender_s_css() {
        let out = sanitize(
            "<style>p{color:red}</style><p class=\"x\" style=\"color:red\" bgcolor=\"red\">Hi</p>",
            SanitizePolicy::CURRENT,
        );
        for token in ["<style", "class=", "style=", "bgcolor"] {
            assert!(
                !out.as_str().contains(token),
                "{:?} has {token}",
                out.as_str()
            );
        }
        assert_eq!(SanitizePolicy::CURRENT.remote_images, RemoteImages::Blocked);
    }
}
