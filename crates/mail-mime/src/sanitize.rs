//! Untrusted HTML to something safe to put in a WebView.

mod body;
mod css;

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Whether to let the message reach the network when it renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteImages {
    /// Default. A remote image is a read receipt the sender did not ask permission for.
    Blocked,
    Allowed,
}

/// Whether the sender's CSS survives: its `<style>` sheets, `style`, `class` and `id`
/// attributes, and the old presentational attributes (`bgcolor`, `width`, `align`...) a
/// newsletter's tables are laid out with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Styles {
    /// Default: the markup is lowered to blocks (the composer, paste, print), which draw it in
    /// mailo's own type and need none of it.
    Dropped,
    /// For the reader's sealed frame, which draws the message as the sender laid it out. Every
    /// way CSS can reach the network is scrubbed out first (`css.rs`); the frame's network
    /// refuses the rest.
    Kept,
}

/// How aggressively to sanitize, and which revision of that policy this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SanitizePolicy {
    pub remote_images: RemoteImages,
    pub styles: Styles,
    /// Bumped whenever the policy or the `ammonia` major changes.
    ///
    /// Part of the render cache key. Sanitizer output is never persisted — an upgrade would
    /// otherwise leave every previously-ingested message sanitized under the old rules.
    pub version: u32,
}

impl SanitizePolicy {
    /// The current default: remote images blocked, the sender's CSS dropped.
    pub const CURRENT: SanitizePolicy = SanitizePolicy {
        remote_images: RemoteImages::Blocked,
        styles: Styles::Dropped,
        version: 4,
    };

    /// The reader's frame: remote images blocked until the reader asks, the sender's CSS kept.
    /// What the frame is rendered, cached and warmed under.
    pub const FRAME: SanitizePolicy = SanitizePolicy {
        styles: Styles::Kept,
        ..SanitizePolicy::CURRENT
    };
}

/// HTML that has been through [`sanitize`], and what had to be removed to get there.
///
/// The count is not bookkeeping. "Load remote images" is an offer to make network requests on
/// the sender's behalf, and a reader that shows it on every message — including the ones with
/// no images at all, and the plain-text ones — has trained its user to ignore the one place it
/// matters. Only [`sanitize`] knows whether anything was actually dropped, so only it can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeHtml {
    html: String,
    blocked_remote: u32,
    remote: Vec<String>,
    body_style: Option<String>,
}

impl SafeHtml {
    /// The sanitized markup.
    pub fn as_str(&self) -> &str {
        &self.html
    }

    /// How many remote fetches were removed.
    ///
    /// Zero under [`RemoteImages::Allowed`] by construction: nothing is blocked when nothing
    /// is being blocked.
    pub fn blocked_remote(&self) -> u32 {
        self.blocked_remote
    }

    /// Every `http`/`https` URL this markup will fetch as it is shown, as the sender wrote it,
    /// in document order and without repeats.
    ///
    /// Empty under [`RemoteImages::Blocked`] by construction. Under `Allowed` it is exactly the
    /// fetch attributes the filter kept, collected as it kept them: a renderer that holds
    /// fetching to this list (the reader's Original frame on Blitz, where the app answers every
    /// request the frame makes) gets the sanitizer's own answer, not a second reading of its
    /// output by another parser. It widens nothing: the markup is the same with or without it.
    pub fn remote_fetches(&self) -> &[String] {
        &self.remote
    }

    /// The sender's `<body>` colours and style as one scrubbed declaration list, for the
    /// document the markup is shown in: the fragment itself has no `<body>`. Only under
    /// [`Styles::Kept`].
    pub fn body_style(&self) -> Option<&str> {
        self.body_style.as_deref()
    }

    /// Wrap already-sanitized markup. Only [`sanitize`] should call this.
    pub(crate) fn new(html: String, blocked_remote: u32, remote: Vec<String>) -> Self {
        Self {
            html,
            blocked_remote,
            remote,
            body_style: None,
        }
    }
}

/// Sanitize a message body for display.
///
/// Total: there is no error case. Anything that cannot be made safe is removed, because the
/// alternative — showing the user nothing — is worse than showing them the text.
///
/// ```
/// use mail_mime::{SanitizePolicy, sanitize};
///
/// let safe = sanitize(
///     r#"<p>hi</p><script>alert(1)</script><img src="https://tracker.example/p.gif">"#,
///     SanitizePolicy::CURRENT,
/// );
/// assert!(safe.as_str().contains("hi"));
/// assert!(!safe.as_str().contains("script"));
/// // A remote image is not fetched unless the reader asks: it is dropped and counted.
/// assert!(!safe.as_str().contains("tracker.example"));
/// assert_eq!(safe.blocked_remote(), 1);
/// ```
pub fn sanitize(html: &str, policy: SanitizePolicy) -> SafeHtml {
    // What the filter saw is this call's alone: cleared before and taken after, on this thread,
    // which is the one ammonia calls the filter on.
    SEEN.with_borrow_mut(|seen| *seen = Seen::default());
    let builder = builder(policy.styles, policy.remote_images);
    // Twice: a removed element's children stay where it stood, which can leave markup a parser
    // reads back as another tree (a `<table>` a dropped `<object>` left inside a `<p>`). The
    // second pass cleans the tree that markup is, so what is returned reads back as itself.
    let cleaned = builder.clean(&builder.clean(html).to_string()).to_string();
    let seen = SEEN.with_borrow_mut(std::mem::take);
    let (cleaned, body_style) = match policy.styles {
        Styles::Kept => (css::scrub_style_elements(&cleaned), body::body_style(html)),
        Styles::Dropped => (cleaned, None),
    };
    let mut safe = SafeHtml::new(cleaned, seen.blocked, seen.remote);
    safe.body_style = body_style;
    safe
}

/// What the attribute filter saw in one [`sanitize`]: the remote fetches it dropped, and the
/// ones it kept, for `SafeHtml::remote_fetches`.
#[derive(Debug, Default)]
struct Seen {
    blocked: u32,
    remote: Vec<String>,
}

thread_local! {
    /// The filter is shared by every call ([`builder`]) and must be `Send + Sync`, so what it
    /// counts is kept per thread: a call cleans on its own thread from start to end.
    static SEEN: std::cell::RefCell<Seen> = std::cell::RefCell::default();
}

/// The sanitizer for `styles` and `remote`, built once and shared: building one is a few dozen
/// sets and maps, which cost more than cleaning a short reply. `SanitizePolicy::version` is not
/// part of it, because it changes nothing the builder holds; it names the rules in a cache key.
fn builder(styles: Styles, remote: RemoteImages) -> &'static ammonia::Builder<'static> {
    static BUILT: [OnceLock<ammonia::Builder<'static>>; 4] = [const { OnceLock::new() }; 4];
    let at = 2 * usize::from(styles == Styles::Kept) + usize::from(remote == RemoteImages::Allowed);
    BUILT[at].get_or_init(|| build(styles, remote))
}

fn build(styles: Styles, remote: RemoteImages) -> ammonia::Builder<'static> {
    // Ammonia's defaults are an allowlist: script, style, iframe, object, embed, form,
    // input, button, base, meta, link, svg and every `on*` attribute are absent from it,
    // so they never reach the serializer. `style` stays off the list unless the policy keeps
    // the sender's CSS, and then every sheet and attribute is scrubbed of what fetches
    // (`css.rs`): ammonia's own CSS filter keeps `url()`.
    // `url_schemes` is global, so it cannot express "cid always, http(s) only on images and
    // only when the reader opted in". The attribute filter applies that second rule to
    // attributes ammonia has already scheme-checked.
    let mut builder = ammonia::Builder::new();
    if styles == Styles::Kept {
        builder
            .rm_clean_content_tags(&["style"])
            .add_tags(&["style", "font", "center"])
            .add_generic_attributes(&PRESENTATIONAL);
    }
    builder
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .strip_comments(true)
        .add_clean_content_tags(&UNRENDERED)
        .rm_tag_attributes("blockquote", &["cite"])
        .rm_tag_attributes("del", &["cite"])
        .rm_tag_attributes("ins", &["cite"])
        .rm_tag_attributes("q", &["cite"])
        .attribute_filter(move |element, attribute, value| {
            if attribute == "style" {
                return Some(css::scrub(value, css::Context::Declarations));
            }
            if attribute == "class" || attribute == "id" {
                return not_own(value);
            }
            if fetches_on_render(element, attribute) {
                let kept = keep_fetched_url(value, remote);
                if is_remote(value) {
                    SEEN.with_borrow_mut(|seen| {
                        if kept.is_none() {
                            seen.blocked += 1;
                        } else if !seen.remote.iter().any(|it| it == value) {
                            seen.remote.push(value.to_owned());
                        }
                    });
                }
                kept
            } else {
                Some(Cow::Borrowed(value))
            }
        });
    builder
}

/// A sender's `class` or `id` without mailo's own names (those starting `mailo-`, in any case):
/// the frame's document puts elements of the reader's own beside the sender's (the inset the
/// words are read in, a folded quote), and its rules for them must reach only them. Nothing is
/// left of an attribute that held only such names.
fn not_own(value: &str) -> Option<Cow<'_, str>> {
    if !value.split_ascii_whitespace().any(css::is_own) {
        return Some(Cow::Borrowed(value));
    }
    let kept: Vec<&str> = value
        .split_ascii_whitespace()
        .filter(|name| !css::is_own(name))
        .collect();
    (!kept.is_empty()).then(|| Cow::Owned(kept.join(" ")))
}

/// Elements whose content a browser never draws as text, removed with everything in them.
///
/// Ammonia removes a tag it does not allow and keeps the text inside, which is right for a tag
/// that wraps what the reader sees (`main`, `font` without the style policy) and wrong for one
/// whose content is never drawn: that content would come out as loose text above or inside the
/// letter, as a `<title>` with a link written into it did. The rule: what a browser leaves
/// undrawn, the reader leaves out:
/// - `title`: it names a window, and is never the page's text;
/// - `template`: inert markup a page clones from script;
/// - `xml`: Outlook's and Word's settings (`<xml><o:OfficeDocumentSettings>`);
/// - `svg`: a drawing, whose `<title>`, `<desc>` and `<text>` are the picture's, not the letter's;
/// - `iframe`, `noembed`, `noframes`, `audio`, `video`, `canvas`: what each shows only where it
///   cannot play;
/// - `select`, `datalist`, `option`: a form's choices, drawn as a control, never as loose words.
///
/// `script` and `style` are already ammonia's own (and `style` is kept as a sheet under
/// [`Styles::Kept`]). Not here, because a mail client does draw their content: `noscript` (no
/// mail client runs script), `object` and `embed` (whose content is the fallback shown when the
/// plugin is not), and `textarea`, `button` and `math`. Nor `head`: mail is parsed as a
/// fragment, where `<head>` is no element and what was in it (a `<style>` kept under
/// [`Styles::Kept`] among it) stands on its own.
const UNRENDERED: [&str; 13] = [
    "title", "template", "xml", "svg", "iframe", "noembed", "noframes", "audio", "video", "canvas",
    "select", "datalist", "option",
];

/// The attributes a sender's layout is written in, kept with [`Styles::Kept`]. `background`
/// is not among them: it is a fetch, and `fetches_on_render` drops it.
const PRESENTATIONAL: [&str; 14] = [
    "style",
    "class",
    "id",
    "bgcolor",
    "width",
    "height",
    "align",
    "valign",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "face",
    "size",
];

/// Whether a dropped URL was one the reader could choose to load.
///
/// Only `http`/`https` are: a `javascript:` or `data:` src is not something to offer, and
/// counting it would put the button in front of a user whose answer can only make things worse.
fn is_remote(value: &str) -> bool {
    ammonia::Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Attributes that cause a fetch when the document is shown.
///
/// `href` on an anchor is a click, not a fetch. `srcset`, `poster` and `background` are not
/// on the allowlist; matching them here drops them if a later edit allows the attribute
/// without also scheme-checking it. Ammonia does not treat `srcset` as a URL.
fn fetches_on_render(element: &str, attribute: &str) -> bool {
    match attribute {
        "src" | "srcset" | "poster" | "background" => true,
        "href" | "xlink:href" => matches!(element, "image" | "use" | "feimage"),
        _ => false,
    }
}

/// Keep a URL ammonia has already accepted, or drop it.
///
/// `cid:` is this message's own part and must survive in both modes. `http` and `https`
/// are a network fetch, kept only when the reader allowed remote images. Anything that is
/// not a single parseable URL — a `srcset` candidate list, in particular — is dropped
/// rather than split with a hand-rolled scanner.
fn keep_fetched_url(value: &str, remote: RemoteImages) -> Option<Cow<'_, str>> {
    let url = ammonia::Url::parse(value).ok()?;
    let keep = match url.scheme() {
        "cid" => true,
        "http" | "https" => remote == RemoteImages::Allowed,
        _ => false,
    };
    keep.then_some(Cow::Borrowed(value))
}
