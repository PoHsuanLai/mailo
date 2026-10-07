//! The provider mark on a row or in a field: quire's `ProviderMark`, given the cached icon
//! when the setting says icons.

use dioxus::prelude::*;
use ds::components::content::provider_mark::{MarkProvider, MarkStyle};
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::provider::Provider;
use mail_core::provider::icon::Loaded;

/// Where the chip sits, which is the mark's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChipPlace {
    /// The provider name on a row.
    Row,
    /// Inline in a field: the composer's From.
    Inline,
}

/// The icon the chip should draw, subscribed to the startup load and to a refresh.
pub(crate) fn current() -> Loaded {
    let live = try_consume_context::<Signal<Loaded>>();
    let once = try_consume_context::<Loaded>();
    if let Some(icons) = live {
        icons.read().clone()
    } else {
        once.unwrap_or_default()
    }
}

/// Fetch the known providers' icons that are not cached yet, once, and show them as they land.
///
/// Called by each window's root right after it provides the icon signal. The fetch runs off the
/// draw (`refresh` is async); nothing waits for it, and a provider whose fetch fails keeps its
/// letter. Under a test binary [`mail_core::provider::icon::missing`] is empty, so no socket opens.
pub(crate) fn use_fetch_missing(mut icons: Signal<Loaded>) {
    use_hook(move || {
        let Some(root) = mail_core::config::cache_dir() else {
            return;
        };
        let dir = root.join("providers");
        let missing = mail_core::provider::icon::missing(&dir);
        if missing.is_empty() {
            return;
        }
        spawn(async move {
            let results = mail_core::provider::icon::refresh(&dir, &missing).await;
            for (provider, result) in &results {
                if let Err(err) = result {
                    eprintln!("provider icon: {provider:?}: {err}");
                }
            }
            icons.set(Loaded::read(&dir));
        });
    });
}

/// quire's name for `provider`: the two tables are the same six, letter for letter.
pub(crate) fn mark_of(provider: Provider) -> MarkProvider {
    match provider {
        Provider::Google => MarkProvider::Google,
        Provider::Microsoft => MarkProvider::Microsoft,
        Provider::Fastmail => MarkProvider::Fastmail,
        Provider::Icloud => MarkProvider::ICloud,
        Provider::Yahoo => MarkProvider::Yahoo,
        Provider::Imap => MarkProvider::Imap,
    }
}

/// mailo's provider behind quire's `mark`: the inverse of [`mark_of`], over the five it names.
/// IMAP and local folders have no icon of their own.
pub(crate) fn provider_of_mark(mark: MarkProvider) -> Option<Provider> {
    match mark {
        MarkProvider::Google => Some(Provider::Google),
        MarkProvider::Microsoft => Some(Provider::Microsoft),
        MarkProvider::Fastmail => Some(Provider::Fastmail),
        MarkProvider::ICloud => Some(Provider::Icloud),
        MarkProvider::Yahoo => Some(Provider::Yahoo),
        _ => None,
    }
}

/// How quire's `mark` is drawn under the provider-marks setting: [`mark_style`] for the provider
/// behind it, else the letter. For parts that name a mark rather than one of mailo's providers
/// (Add Account's list and its steps).
pub(crate) fn style_of_mark(mark: MarkProvider, marks: super::view::Marks) -> MarkStyle {
    provider_of_mark(mark).map_or(MarkStyle::Letter, |provider| mark_style(provider, marks))
}

/// How `provider` is drawn under the provider-marks setting: the cached icon when the setting
/// says icons and one is held, else the letter. Reads the startup load, so call it in a render.
pub(crate) fn mark_style(provider: Provider, marks: super::view::Marks) -> MarkStyle {
    let uri = match marks {
        super::view::Marks::Icons => current().uri(provider),
        super::view::Marks::Letters => None,
    };
    uri.map_or(MarkStyle::Letter, |uri| MarkStyle::Image(ImageSource(uri)))
}

/// The mark on a tile or a row: the cached icon, or the letter.
#[component]
pub(crate) fn ProvChip(provider: Provider, marks: super::view::Marks, place: ChipPlace) -> Element {
    let style = mark_style(provider, marks);
    let size = match place {
        ChipPlace::Row => ControlSize::Small,
        ChipPlace::Inline => ControlSize::Mini,
    };
    rsx! {
        ProviderMark { provider: mark_of(provider), size, style }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::Marks;

    #[test]
    fn quires_mark_names_the_same_provider_back() {
        // Add Account names marks, not providers: every known provider must come back through
        // its mark, or its list row would keep the letter while a row on the list has the icon.
        for provider in Provider::ALL {
            let back = provider_of_mark(mark_of(provider));
            match provider {
                Provider::Imap => assert_eq!(back, None),
                _ => assert_eq!(back, Some(provider), "{provider:?}"),
            }
        }
        assert_eq!(provider_of_mark(MarkProvider::Local), None);
        assert_eq!(
            style_of_mark(MarkProvider::Imap, Marks::Icons),
            MarkStyle::Letter
        );
    }

    #[tokio::test]
    async fn the_chip_draws_the_cached_icon_or_the_letter() {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        // `Loaded::read` takes any file that starts with the PNG signature; the chip never decodes.
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0];
        std::fs::write(dir.path().join("google.png"), png).unwrap_or_else(|err| panic!("{err}"));
        let loaded = Loaded::read(dir.path());
        let uri = loaded
            .uri(Provider::Google)
            .unwrap_or_else(|| panic!("no uri"));
        let cases = [
            (Marks::Icons, true, ChipPlace::Inline, true),
            (Marks::Icons, true, ChipPlace::Row, true),
            (Marks::Letters, true, ChipPlace::Inline, false),
            (Marks::Icons, false, ChipPlace::Inline, false),
        ];
        for (marks, with_file, place, image) in cases {
            let html = render_chip(
                marks,
                if with_file {
                    loaded.clone()
                } else {
                    Loaded::default()
                },
                place,
            );
            let name = format!("{marks:?} file={with_file} {place:?}");
            if image {
                assert!(html.contains("data-kind=\"image\""), "{name}: {html}");
                assert!(html.contains(&format!("src=\"{uri}\"")), "{name}: {html}");
                assert!(!html.contains("file:"), "{name}: {html}");
                assert!(
                    !html.contains(">G<"),
                    "{name} drew the letter as well: {html}"
                );
            } else {
                assert!(html.contains(">G<"), "{name}: {html}");
                assert!(!html.contains("data-kind=\"image\""), "{name}: {html}");
                assert!(!html.contains("data:image"), "{name}: {html}");
            }
        }
    }

    fn render_chip(marks: Marks, loaded: Loaded, place: ChipPlace) -> String {
        let mut dom = VirtualDom::new(ChipHarness)
            .with_root_context(ChipCase { marks, place })
            .with_root_context(loaded);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[derive(Clone)]
    struct ChipCase {
        marks: Marks,
        place: ChipPlace,
    }

    #[component]
    fn ChipHarness() -> Element {
        let case = consume_context::<ChipCase>();
        rsx! { ProvChip { provider: Provider::Google, marks: case.marks, place: case.place } }
    }
}
