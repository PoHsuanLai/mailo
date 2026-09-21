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
