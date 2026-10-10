//! The provider mark on a row or in a field: quire's `ProviderMark`, given the cached icon
//! when the setting says icons.

use dioxus::prelude::*;
use ds::components::content::provider_mark::{MarkProvider, MarkStyle};
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::provider::Provider;
use mail_core::provider::icon::Loaded;

/// Where a mark is drawn, which is how many CSS px across its picture is.
///
/// quire draws a favicon at a size of its own choosing, and the renderer resamples whatever it is
/// handed to that size with a bilinear filter. A 96 px file squeezed into a 20 px box that way
/// comes out soft, and a 32 px file stretched over a 96 px header comes out blurred, so each place
/// is handed a picture already drawn at its own size on the screen ([`SIDES`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkAt {
    /// quire's `ProviderMark` at `ControlSize::Small`, on a row or in the composer's From: the
    /// 13 px chip less its padding (`ds/src/components/content/provider_mark.css`).
    Row,
    /// `ProviderMark` at `ControlSize::Regular`, the badge on a Space tile: 14 px less its padding.
    Tile,
    /// A row of Add Account's provider list: quire's `mark_leading` asks for 28 px
    /// (`ds-shell/src/accounts/adapter.rs`), and the settings-density row's leading box,
    /// `--row-avatar`, stretches it to 32 (`ds/src/components/lists/row/row.css`).
    List,
    /// The header of an Add Account step: quire's `Disc` at `Size48`.
    Header,
}

/// The scale the pictures are drawn for: a HiDPI screen's. A screen at 1 shows each at exactly
/// half, which the renderer's filter averages cleanly.
const SCALE: u32 = 2;

impl MarkAt {
    /// The picture's side in CSS px.
    pub(crate) const fn css(self) -> u32 {
        match self {
            MarkAt::Row => 10,
            MarkAt::Tile => 11,
            MarkAt::List => 32,
            MarkAt::Header => 48,
        }
    }

    /// The picture's side on a screen at [`SCALE`], in device px.
    pub(crate) const fn side(self) -> u32 {
        self.css() * SCALE
    }
}

/// Every side a picture is drawn at, in device px: what [`read`] draws each cached icon at.
pub(crate) const SIDES: [u32; 4] = [
    MarkAt::Row.side(),
    MarkAt::Tile.side(),
    MarkAt::List.side(),
    MarkAt::Header.side(),
];

/// The cached icons under `dir`, each drawn at every size a window shows it at: what a window's
/// `Loaded` context holds.
pub fn read(dir: &std::path::Path) -> Loaded {
    Loaded::read(dir, &SIDES)
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
        let missing = mail_core::provider::icon::missing(&dir, crate::edge::environment().program);
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
            icons.set(read(&dir));
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
pub(crate) fn style_of_mark(
    mark: MarkProvider,
    marks: super::view::Marks,
    at: MarkAt,
) -> MarkStyle {
    provider_of_mark(mark).map_or(MarkStyle::Letter, |provider| {
        mark_style(provider, marks, at)
    })
}

/// How `provider` is drawn under the provider-marks setting: the cached icon when the setting
/// says icons and one is held, drawn for `at`, else the letter. Reads the startup load, so call it
/// in a render.
pub(crate) fn mark_style(provider: Provider, marks: super::view::Marks, at: MarkAt) -> MarkStyle {
    let uri = match marks {
        super::view::Marks::Icons => current().uri(provider, at.side()),
        super::view::Marks::Letters => None,
    };
    uri.map_or(MarkStyle::Letter, |uri| MarkStyle::Image(ImageSource(uri)))
}

/// The mark on a row or in a field: the cached icon, or the letter, at a row's size wherever it
/// stands.
#[component]
pub(crate) fn ProvChip(provider: Provider, marks: super::view::Marks) -> Element {
    let style = mark_style(provider, marks, MarkAt::Row);
    rsx! {
        ProviderMark { provider: mark_of(provider), size: ControlSize::Small, style }
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
            style_of_mark(MarkProvider::Imap, Marks::Icons, MarkAt::Row),
            MarkStyle::Letter
        );
    }

    #[tokio::test]
    async fn the_chip_draws_the_cached_icon_or_the_letter() {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        std::fs::write(dir.path().join("google.png"), cached_png())
            .unwrap_or_else(|err| panic!("{err}"));
        let loaded = read(dir.path());
        let cases = [
            (Marks::Icons, true, true),
            (Marks::Letters, true, false),
            (Marks::Icons, false, false),
        ];
        for (marks, with_file, image) in cases {
            let html = render_chip(
                marks,
                if with_file {
                    loaded.clone()
                } else {
                    Loaded::default()
                },
            );
            let name = format!("{marks:?} file={with_file}");
            let uri = loaded
                .uri(Provider::Google, MarkAt::Row.side())
                .unwrap_or_else(|| panic!("no uri"));
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

    #[test]
    fn each_place_is_handed_a_picture_drawn_at_its_own_size_on_the_screen() {
        // A picture the renderer resamples comes out soft: each place gets one already as many
        // pixels across as it covers on a scale-2 screen.
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        std::fs::write(dir.path().join("google.png"), cached_png())
            .unwrap_or_else(|err| panic!("{err}"));
        let loaded = read(dir.path());
        for at in [MarkAt::Row, MarkAt::Tile, MarkAt::List, MarkAt::Header] {
            let uri = loaded
                .uri(Provider::Google, at.side())
                .unwrap_or_else(|| panic!("{at:?}: no uri"));
            let image = decoded(&uri);
            assert_eq!(
                (image.width(), image.height()),
                (at.css() * 2, at.css() * 2),
                "{at:?}"
            );
        }
    }

    /// A cached icon as `mailo icons refresh` writes one: 96 px square.
    fn cached_png() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(96, 96, image::Rgba([0x1a, 0x73, 0xe8, 0xff]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap_or_else(|err| panic!("{err}"));
        png
    }

    fn decoded(uri: &str) -> image::DynamicImage {
        use base64::Engine as _;
        let data = uri
            .strip_prefix("data:image/png;base64,")
            .unwrap_or_else(|| panic!("not a png data uri"));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap_or_else(|err| panic!("{err}"));
        image::load_from_memory(&bytes).unwrap_or_else(|err| panic!("{err}"))
    }

    fn render_chip(marks: Marks, loaded: Loaded) -> String {
        let mut dom = VirtualDom::new(ChipHarness)
            .with_root_context(ChipCase { marks })
            .with_root_context(loaded);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[derive(Clone)]
    struct ChipCase {
        marks: Marks,
    }

    #[component]
    fn ChipHarness() -> Element {
        let case = consume_context::<ChipCase>();
        rsx! { ProvChip { provider: Provider::Google, marks: case.marks } }
    }
}
