//! What a browser never draws, the reader does not show.
//!
//! Ammonia removes a tag it does not allow and keeps the text inside it. For an element whose
//! content a browser never draws as text, that text would come out loose in the reader: a
//! `<title>` with a link written into it showed as raw `<a href=…>` above the letter. Each sample
//! here is HTML a sender wrote, set against the text a browser shows for it; the sanitized
//! markup, under both policies, must show the same text, no more and no less.

use mail_mime::{SanitizePolicy, sanitize};

struct Sample {
    name: &'static str,
    html: &'static str,
    /// The words a browser draws for `html`, whitespace collapsed.
    shown: &'static str,
}

const SAMPLES: &[Sample] = &[
    Sample {
        // The shape of a real notification: a link written into the title, then the letter.
        name: "a link written into the title",
        html: "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01 Transitional//EN\">\n\
               <html lang=\"en\"><head>\n\
               <meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">\n\
               <title>    <a href=\"https://example.test/welcome\" style=\"color: #4183C4;\">Welcome aboard</a>\n</title>\n\
               <style>body { margin: 0; }</style></head>\n\
               <body><table><tr><td><h1>Welcome aboard</h1><p>Hi Ada,</p>\
               <p>Your <a href=\"https://example.test/plan\">plan</a> is ready.</p></td></tr></table></body></html>",
        shown: "Welcome aboard Hi Ada, Your plan is ready.",
    },
    Sample {
        name: "a plain title",
        html: "<title>Your receipt</title><p>Thanks for your order.</p>",
        shown: "Thanks for your order.",
    },
    Sample {
        // Outlook and Word write their settings outside the conditional comment as often as in.
        name: "an Office settings block",
        html: "<xml><o:OfficeDocumentSettings><o:AllowPNG/><o:PixelsPerInch>96</o:PixelsPerInch>\
               </o:OfficeDocumentSettings></xml><p class=\"MsoNormal\">Hello<o:p></o:p></p>",
        shown: "Hello",
    },
    Sample {
        name: "an icon drawn in SVG, with its title, description and text",
        html: "<p><a href=\"https://example.test/social\"><svg viewBox=\"0 0 24 24\">\
               <title>Social icon</title><desc>A bird</desc><text x=\"0\" y=\"12\">S</text>\
               </svg></a> Follow us</p>",
        shown: "Follow us",
    },
    Sample {
        name: "a template",
        html: "<template><p>Hidden copy</p></template><p>Shown copy</p>",
        shown: "Shown copy",
    },
    Sample {
        name: "a frame's fallback",
        html: "<iframe src=\"https://example.test/embed\">Your mail app cannot show frames.</iframe><p>Body</p>",
        shown: "Body",
    },
    Sample {
        name: "noembed and noframes",
        html: "<noembed>No plugins.</noembed><noframes>No frames.</noframes><p>Body</p>",
        shown: "Body",
    },
    Sample {
        name: "a player's and a canvas's fallback",
        html: "<video src=\"https://example.test/v.mp4\">Your client cannot play video.</video>\
               <audio src=\"https://example.test/a.mp3\">No audio.</audio>\
               <canvas>No canvas.</canvas><p>Watch the talk</p>",
        shown: "Watch the talk",
    },
    Sample {
        name: "a form's choices",
        html: "<p>Rate us: <select><option>Good</option><option>Bad</option></select>\
               <datalist><option value=\"x\">Extra</option></datalist> thanks</p>",
        shown: "Rate us: thanks",
    },
    Sample {
        name: "the undrawn, nested inside a layout table",
        html: "<table><tr><td><svg><title>Logo</title></svg></td><td><title>Stray</title>Hello</td></tr></table>",
        shown: "Hello",
    },
    // What a mail client does draw stays.
    Sample {
        name: "noscript, since no mail client runs script",
        html: "<noscript>Open this in a browser.</noscript><p>Body</p>",
        shown: "Open this in a browser. Body",
    },
    Sample {
        name: "an object's fallback, shown where the plugin is not",
        html: "<object data=\"https://example.test/o\">The chart</object><p>Body</p>",
        shown: "The chart Body",
    },
    Sample {
        name: "a wrapper ammonia does not allow",
        html: "<main><section><font color=\"red\">Kept text</font></section></main>",
        shown: "Kept text",
    },
    Sample {
        name: "markup the sender escaped themselves, which a browser shows as text",
        html: "<p>&lt;a href=\"x\"&gt;link&lt;/a&gt;</p>",
        shown: "<a href=\"x\">link</a>",
    },
    Sample {
        name: "a style sheet",
        html: "<style>p { color: red; }</style><p>Red</p>",
        shown: "Red",
    },
];

#[test]
fn the_reader_shows_what_a_browser_draws() {
    for policy in [SanitizePolicy::CURRENT, SanitizePolicy::FRAME] {
        for sample in SAMPLES {
            let safe = sanitize(sample.html, policy);
            assert_eq!(
                text_of(safe.as_str()),
                sample.shown,
                "{} under {:?}: {:?}",
                sample.name,
                policy.styles,
                safe.as_str()
            );
        }
    }
}

/// The text a browser draws for ammonia's serialized markup: the characters outside tags,
/// entities decoded, whitespace collapsed. A `<style>` sheet's text is not drawn.
fn text_of(markup: &str) -> String {
    let mut text = String::new();
    let mut rest = markup;
    let mut in_style = false;
    while let Some(open) = rest.find('<') {
        if !in_style {
            text.push_str(&rest[..open]);
        }
        let tag = &rest[open..];
        let close = tag_end(tag);
        let name = tag[1..close]
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if name == "style" {
            in_style = !tag.starts_with("</");
        }
        // A block element's edge separates words the way a browser's line break does.
        text.push(' ');
        rest = &tag[close + 1..];
    }
    if !in_style {
        text.push_str(rest);
    }
    let decoded = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    let words: Vec<&str> = decoded.split_whitespace().collect();
    // Punctuation follows its word, as it does on screen, where no tag edge stands between.
    words
        .join(" ")
        .replace(" ,", ",")
        .replace(" .", ".")
        .replace(" :", ":")
}

/// Where the tag at the start of `tag` ends: its `>`, outside any quoted attribute value.
fn tag_end(tag: &str) -> usize {
    let mut quote = None;
    for (at, c) in tag.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return at,
            _ => {}
        }
    }
    tag.len() - 1
}
