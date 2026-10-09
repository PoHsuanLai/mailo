//! A sender's brand logo in place of their initial: in the reader's head, and on the sender card.
//!
//! Found off the thread that draws, once per message and body, like the checks line beside it
//! (`checks.rs`): the switch, the believed `Authentication-Results` and the cache are read on a
//! blocking thread, and only a domain the cache knows nothing about is looked up, on the async
//! runtime (`mail_core::bimi`, `mail_runtime::bimi`). The window hands the cache's directory in as
//! a [`BrandCache`]; without one, and without a config directory to read the switch from,
//! nothing is looked up at all. The logo reaches the page as a PNG `data:` URL, drawn from the
//! SVG beforehand; the SVG itself is never put in the page.

use crate::ui::appearance::WindowDirs;
use base64::Engine as _;
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use mail_core::bimi::{Setting, domain_of};
use mail_domain::{BlobId, MessageId};
use mail_runtime::bimi::{Cached, Lookup, cached};
use mail_store::SqliteStore;
use std::path::PathBuf;
use std::sync::Arc;

/// Where drawn logos are kept: the cache directory's `bimi/`. A root context; a window without
/// one shows no logos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandCache(pub PathBuf);

/// What the blocking half found.
enum Local {
    /// Nothing to show and nothing to ask: the switch is off, DMARC did not pass for the sender,
    /// or the cache says the domain has none.
    Nothing,
    /// A drawn logo, from the cache.
    Drawn(Vec<u8>),
    /// Worth asking, with these roots.
    Ask(Vec<mail_mime::smime::Cert>, mail_mime::AuthResults),
}

/// The switch, the checks and the cache, read from disk. Blocking.
fn local(
    store: &SqliteStore,
    logos: crate::settings::BrandLogos,
    config: &std::path::Path,
    cache: &std::path::Path,
    message: MessageId,
    raw: BlobId,
    from: &str,
) -> Local {
    if Setting::from(logos) == Setting::Off {
        return Local::Nothing;
    }
    let Some(domain) = domain_of(from) else {
        return Local::Nothing;
    };
    let Some(results) = super::checks::lookup(store, message, raw) else {
        return Local::Nothing;
    };
    if !mail_mime::bimi::dmarc_passed_for(&results, domain) {
        return Local::Nothing;
    }
    match cached(cache, domain, chrono::Utc::now()) {
        Cached::Logo(png) => Local::Drawn(png),
        Cached::None => Local::Nothing,
        Cached::Unknown => Local::Ask(mail_core::bimi::anchors(config), results),
    }
}

/// The sender's logo as a `data:` URL, once found; `None` before, and for a sender without one.
fn use_brand_logo(
    message: MessageId,
    body: Option<BlobId>,
    from: String,
) -> Signal<Option<String>> {
    let mut logo = use_signal(|| None::<String>);
    let settings = crate::ui::prefs::use_settings();
    let _find = use_resource(move || {
        // Read here, so turning brand logos on or off looks again.
        let logos = settings.read().reading.brand_logos;
        let store = consume_context::<Arc<SqliteStore>>();
        let dirs = try_consume_context::<WindowDirs>();
        let place = try_consume_context::<BrandCache>();
        let from = from.clone();
        async move {
            let (Some(dirs), Some(BrandCache(dir)), Some(raw)) = (dirs, place, body) else {
                return;
            };
            let (asked, at) = (from.clone(), dir.clone());
            let found = tokio::task::spawn_blocking(move || {
                local(&store, logos, &dirs.config, &at, message, raw, &asked)
            })
            .await;
            let png = match found {
                Ok(Local::Drawn(png)) => Some(png),
                Ok(Local::Ask(anchors, results)) => ask(&anchors, &results, &from, &dir).await,
                Ok(Local::Nothing) | Err(_) => None,
            };
            if let Some(png) = png {
                logo.set(Some(format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(png)
                )));
            }
        }
    });
    logo
}

/// Look the sender's domain up with the system's resolver.
async fn ask(
    anchors: &[mail_mime::smime::Cert],
    results: &mail_mime::AuthResults,
    from: &str,
    dir: &std::path::Path,
) -> Option<Vec<u8>> {
    let dns = mail_runtime::lookup::SystemDns::new().ok()?;
    let http = mail_runtime::bimi::client_builder().build().ok()?;
    let lookup = Lookup {
        dns: &dns,
        http: &http,
        anchors,
        now: chrono::Utc::now(),
    };
    mail_core::bimi::brand_logo(Setting::On, Some(results), from, &lookup, dir).await
}

/// How many px across an avatar of `size` is: quire's `data-size`, which it keeps to itself. A
/// logo stands where the avatar would, at the same size.
fn side_of(size: AvatarSize) -> u8 {
    match size {
        AvatarSize::Size16 => 16,
        AvatarSize::Size18 => 18,
        AvatarSize::Size20 => 20,
        AvatarSize::Size22 => 22,
        AvatarSize::Size26 => 26,
        AvatarSize::Size28 => 28,
        AvatarSize::Size30 => 30,
        AvatarSize::Size34 => 34,
        AvatarSize::Size48 => 48,
        AvatarSize::Size64 => 64,
    }
}

/// The message header's avatar: the sender's logo when there is one, else quire's avatar with
/// their initial. Keyed by its parent on the message and its body, as the checks line is.
#[component]
pub(in crate::ui) fn ReaderAvatar(
    message: MessageId,
    body: Option<BlobId>,
    from: String,
    face: ds::components::content::avatar::AvatarFace,
) -> Element {
    let logo = use_brand_logo(message, body, from.clone());
    match logo() {
        Some(src) => {
            let alt = format!("Logo of {}", domain_of(&from).unwrap_or_default());
            let side = side_of(face.size);
            rsx! {
                div { class: "reader-av reader-logo", style: "width: {side}px; height: {side}px;",
                    img { src: "{src}", alt: "{alt}" }
                }
            }
        }
        None => rsx! {
            div { class: "reader-av",
                ds::components::content::avatar::Avatar { initial: face.initial, size: face.size, tone: face.tone }
            }
        },
    }
}
