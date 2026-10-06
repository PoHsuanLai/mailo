use super::{Context, scrub, scrub_style_elements};
use std::borrow::Cow;

#[test]
fn what_fetches_or_runs_is_removed_and_the_rest_kept() {
    const SHEET: &[(&str, &str)] = &[
        ("p{color:red}", "p{color:red}"),
        (
            "body{background:url(https://t.test/x)}",
            "body{background:none}",
        ),
        ("a{background:URL( 'x' )}", "a{background:none}"),
        // The renderer's tokenizer decodes the escape to `url(`; so does this one.
        (
            "a{background:u\\72l(https://t.test/x)}",
            "a{background:none}",
        ),
        (
            "@import url(https://t.test/s.css);p{margin:0}",
            "p{margin:0}",
        ),
        ("@IMPORT 'https://t.test/s.css';p{margin:0}", "p{margin:0}"),
        ("a{background:image-set('x.png' 1x)}", "a{background:none}"),
        (
            "a{background:-webkit-image-set(url(x) 1x)}",
            "a{background:none}",
        ),
        (
            "a{background-image:src(\"https://t.test/x\")}",
            "a{background-image:none}",
        ),
        ("a{width:expression(alert(1))}", "a{width:none}"),
        ("a{-moz-binding:url(x.xml#b);color:red}", "a{color:red}"),
        ("a{behavior:url(x.htc)}", "a{}"),
        // Only a declaration's name: a selector, a class or a longer property keeps its rule.
        (
            ".behavior a:hover{color:red} p{margin:0}",
            ".behavior a:hover{color:red} p{margin:0}",
        ),
        (".no-behavior{color:red}", ".no-behavior{color:red}"),
        ("a{scroll-behavior:smooth}", "a{scroll-behavior:smooth}"),
        ("a{background:curl(x)}", "a{background:curl(x)}"),
        // A quote inside an unquoted url, or a url( in a comment, loses nothing after it.
        (
            "a{background:url(it's.png)} p{margin:0}",
            "a{background:none} p{margin:0}",
        ),
        ("/* see url( */ p{margin:0}", " p{margin:0}"),
        (
            "a{background:url(\"a)b\")} p{margin:0}",
            "a{background:none} p{margin:0}",
        ),
        // Escapes in strings are characters, not text.
        (
            "li:before{content:\"\\2022\"}",
            "li:before{content:\"\u{2022}\"}",
        ),
        (
            "i:before{content:\"\\f101\"}",
            "i:before{content:\"\u{f101}\"}",
        ),
        // A `<` is never written raw.
        ("a{content:\"\\3c/style>\"}", "a{content:\"\\3c /style>\"}"),
        ("<!-- p{margin:0} -->", " p{margin:0} "),
        (
            "@media (min-width:600px){.x{background:url(y);color:red}}",
            "@media (min-width:600px){.x{background:none;color:red}}",
        ),
        ("ul>li{margin:0}", "ul>li{margin:0}"),
        (".x{font-family:'Ünï'}", ".x{font-family:'Ünï'}"),
        // A sheet that is walked writes its strings with cssparser's quotes.
        (
            "@media print{.x{font-family:'Ünï'}}",
            "@media print{.x{font-family:\"Ünï\"}}",
        ),
    ];
    for (css, want) in SHEET {
        assert_eq!(scrub(css, Context::Rules), *want, "{css}");
    }
    const ATTRIBUTE: &[(&str, &str)] = &[
        ("color:#fff;padding:24px", "color:#fff;padding:24px"),
        ("background:url(https://t.test/x)", "background:none"),
        ("behavior:url(x.htc);color:red", "color:red"),
        ("scroll-behavior:smooth", "scroll-behavior:smooth"),
    ];
    for (css, want) in ATTRIBUTE {
        assert_eq!(scrub(css, Context::Declarations), *want, "{css}");
    }
}

#[test]
fn plain_css_is_handed_back_as_it_came() {
    for css in [
        "color:#fff;padding:24px",
        "p{margin:0}",
        "td{vertical-align:top}",
    ] {
        assert!(
            matches!(scrub(css, Context::Declarations), Cow::Borrowed(_)),
            "{css}"
        );
    }
}

#[test]
fn only_the_text_inside_style_elements_is_scrubbed() {
    let html =
        "<style>p{background:url(x)}</style><p>url(y)</p><STYLE media=\"all\">@import 'z';</STYLE>";
    assert_eq!(
        scrub_style_elements(html),
        "<style>p{background:none}</style><p>url(y)</p><STYLE media=\"all\"></STYLE>"
    );
}
