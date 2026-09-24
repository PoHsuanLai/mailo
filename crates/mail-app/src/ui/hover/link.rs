//! The link pill: where a link in the reader really goes, read from the parsed blocks.

use super::hover;
use crate::trust::Destination;
use dioxus::prelude::*;
use ds::LinkTarget;

/// Where the link under the pointer goes, like a browser's status bar, and loud when its text
/// names somewhere else. Drawn in the reader; the link reports itself from the parsed blocks.
#[component]
pub(in crate::ui) fn LinkPill() -> Element {
    let Some(state) = hover() else {
        return rsx! {};
    };
    let Some(link) = state.link.read().clone() else {
        return rsx! {};
    };
    // quire's pill for a web address, honest or lying; keyed by where it goes, so a new link
    // plays its entrance. A target with no registered domain (a `mailto:`) has no quire form.
    match link {
        Destination::Web {
            scheme,
            sub,
            registered,
            path,
        } => rsx! {
            ds::LinkPill {
                key: "{scheme}{sub}{registered}{path}",
                target: LinkTarget::Honest { scheme_sub: format!("{scheme}{sub}"), registered, path },
            }
        },
        Destination::Lies { goes_to, claims } => rsx! {
            ds::LinkPill {
                key: "{goes_to} {claims}",
                target: LinkTarget::Lying { registered: goes_to, shown: claims },
            }
        },
        Destination::Other(href) => rsx! {
            div { class: "linkpill", role: "status", span { class: "dim", "{href}" } }
        },
    }
}

/// A web address with its registered domain in bold and the rest dimmed: the part a lookalike
/// cannot fake, made the part that is read.
fn web(scheme: &str, sub: &str, registered: &str, path: &str) -> Element {
    rsx! {
        span { class: "dim", "{scheme}{sub}" }
        b { "{registered}" }
        span { class: "dim", "{path}" }
    }
}

/// `url` drawn the way the pill draws a link's target. Only the address is read: its text is
/// the address itself, so there is nothing for it to claim.
pub(in crate::ui) fn url_spans(url: &str) -> Element {
    match crate::trust::destination(url, url) {
        Destination::Web {
            scheme,
            sub,
            registered,
            path,
        } => web(&scheme, &sub, &registered, &path),
        Destination::Lies { .. } | Destination::Other(_) => {
            rsx! { span { class: "dim", "{url}" } }
        }
    }
}

/// A link in the reader was entered or left. Called from the parsed blocks.
pub(in crate::ui) fn link_over(text: &str, href: &str) {
    if let Some(mut state) = hover() {
        state.link.set(Some(crate::trust::destination(text, href)));
    }
}

pub(in crate::ui) fn link_out() {
    if let Some(mut state) = hover() {
        state.link.set(None);
    }
}
