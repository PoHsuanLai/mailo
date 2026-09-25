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
            version: SanitizePolicy::CURRENT.version,
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
                version: SanitizePolicy::CURRENT.version,
            },
        )
        .blocked_remote()
    }

    #[test]
    fn a_remote_image_that_was_dropped_is_counted() {
        assert_eq!(
            blocked(
                r#"<p>hi</p><img src="https://tracker.test/pixel.gif">"#,
                RemoteImages::Blocked
            ),
            1
        );
        assert_eq!(
            blocked(
                r#"<img src="https://a.test/1.png"><img src="http://b.test/2.png">"#,
                RemoteImages::Blocked
            ),
            2
        );
    }

    #[test]
    fn nothing_is_blocked_when_nothing_is_being_blocked() {
        // Under `Allowed` the URLs are kept, so there is nothing to offer to load.
        assert_eq!(
            blocked(
                r#"<img src="https://tracker.test/pixel.gif">"#,
                RemoteImages::Allowed
            ),
            0
        );
    }

    #[test]
    fn a_message_with_no_remote_images_reports_none() {
        // The common case, and the one that put the button in front of the user for nothing.
        assert_eq!(blocked("<p>just words</p>", RemoteImages::Blocked), 0);
        assert_eq!(
            blocked(
                r#"<a href="https://example.test">a link</a>"#,
                RemoteImages::Blocked
            ),
            0,
            "a link is a click, not a fetch"
        );
        assert_eq!(
            blocked(r#"<img src="cid:logo@example">"#, RemoteImages::Blocked),
            0,
            "an inline part is this message's own bytes and is never blocked"
        );
    }

    #[test]
    fn a_url_the_reader_could_not_choose_to_load_is_not_counted() {
        // `javascript:` and `data:` srcs are dropped too, and offering to load them would put
        // a button in front of a user whose only possible answer makes things worse.
        for hostile in [
            r#"<img src="javascript:alert(1)">"#,
            r#"<img src="data:text/html,<script>alert(1)</script>">"#,
            r#"<img src="not a url at all">"#,
        ] {
            assert_eq!(
                blocked(hostile, RemoteImages::Blocked),
                0,
                "counted {hostile:?} as something the reader could load"
            );
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
                version: SanitizePolicy::CURRENT.version,
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
    fn nothing_is_fetched_while_remote_images_are_blocked() {
        assert!(fetches(BODY, RemoteImages::Blocked).is_empty());
    }

    #[test]
    fn allowed_images_are_listed_once_each_and_nothing_else() {
        assert_eq!(
            fetches(BODY, RemoteImages::Allowed),
            vec![
                "https://cdn.test/a.png".to_owned(),
                "http://cdn.test/b.png?x=1&y=2".to_owned(),
            ]
        );
    }

    #[test]
    fn the_list_changes_nothing_in_the_markup() {
        let policy = SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            version: SanitizePolicy::CURRENT.version,
        };
        let safe = sanitize(BODY, policy);
        for url in safe.remote_fetches() {
            let written = url.replace('&', "&amp;");
            assert!(
                safe.as_str().contains(&written),
                "{url} not in {}",
                safe.as_str()
            );
        }
        assert!(!safe.as_str().contains("javascript:"));
        assert!(!safe.as_str().contains("file:"));
    }
}
