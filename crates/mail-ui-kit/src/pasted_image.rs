//! A picture on the clipboard, pasted into the composer's body.
//!
//! quire's clipboard (`ds_blitz::clipboard`) reads text, HTML and files, and an edit surface's
//! paste hands over only text or HTML (`ds::host::pasted::Pasted`): a clipboard that holds only a
//! picture (a screenshot, an image copied from a browser or a viewer) pastes nothing at all. So
//! when the paste chord brought the body nothing, the body asks here, and the picture is read
//! from the clipboard as PNG. gap(quire): a `Pasted::Image` from the surface would make this one
//! paste, the Edit menu's Paste included.
//!
//! What reads it is a [`ClipboardImage`] seam, like `pick::Dialogs`: a test hands the window its
//! own picture (a root context, built with [`ClipboardImage::with_answer`] from outside the
//! crate), and in the window's own tests the real one has none, so no test reads the desktop's
//! clipboard.

use std::sync::Arc;

use dioxus::prelude::*;

/// The most pixels a pasted picture may have: a 5K screen's twice over. Past it the paste is
/// refused rather than held, four bytes a pixel, while it is written out as PNG.
const MOST_PIXELS: usize = 2 * 5120 * 2880;

/// What reads the clipboard's picture, as PNG. Nothing, when it holds none.
pub type Reader = dyn Fn() -> Option<Vec<u8>> + Send + Sync;

/// Whatever reads a picture off the window's clipboard, as a root context.
#[derive(Clone)]
pub struct ClipboardImage(pub Arc<Reader>);

impl std::fmt::Debug for ClipboardImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClipboardImage")
    }
}

impl ClipboardImage {
    /// A clipboard whose picture `answer` gives, as PNG, called off the UI thread as the
    /// desktop's would be. For tests: no real clipboard.
    pub fn with_answer(answer: impl Fn() -> Option<Vec<u8>> + Send + Sync + 'static) -> Self {
        ClipboardImage(Arc::new(answer))
    }

    /// The desktop's clipboard. In the window's own tests, one that holds nothing: a window drawn by
    /// a test has no business reading the desktop's clipboard.
    fn real() -> Self {
        if cfg!(any(test, feature = "testing")) {
            return ClipboardImage(Arc::new(|| None));
        }
        ClipboardImage(Arc::new(read_system))
    }
}

/// Read the clipboard's picture off the UI thread, and hand it to `got` as PNG. Nothing, when
/// the clipboard holds none.
pub fn read(got: impl FnOnce(Vec<u8>) + 'static) {
    let clipboard = try_consume_context::<ClipboardImage>().unwrap_or_else(ClipboardImage::real);
    spawn(async move {
        let read = tokio::task::spawn_blocking(move || (clipboard.0)()).await;
        if let Ok(Some(png)) = read {
            got(png);
        }
    });
}

/// The system clipboard's picture, as PNG.
fn read_system() -> Option<Vec<u8>> {
    // No clipboard, or none with a picture on it: nothing to paste, which the paste says.
    let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
    png_of(image.width, image.height, image.bytes.into_owned())
}

/// `rgba`, `width` by `height` pixels of four bytes each, as a PNG. `None` when the bytes are not
/// that many, or the picture is larger than [`MOST_PIXELS`].
fn png_of(width: usize, height: usize, rgba: Vec<u8>) -> Option<Vec<u8>> {
    if width.checked_mul(height)? > MOST_PIXELS {
        return None;
    }
    // A side past `u32` is past `MOST_PIXELS` too: refused as one.
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let pixels = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    // Writing into memory fails only on pixels it cannot encode: no picture to paste.
    pixels.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::{MOST_PIXELS, png_of};

    #[test]
    fn pixels_are_written_as_a_png_that_reads_back_as_them() {
        let rgba = vec![255, 0, 0, 255, 0, 0, 255, 128];
        let png = png_of(2, 1, rgba.clone()).unwrap_or_else(|| panic!("a PNG"));
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap_or_else(|why| panic!("reads back: {why}"))
            .into_rgba8();
        assert_eq!((back.width(), back.height()), (2, 1));
        assert_eq!(back.into_raw(), rgba);
    }

    #[test]
    fn what_is_not_a_picture_of_its_size_is_refused() {
        let cases: [(&str, usize, usize, usize); 3] = [
            ("too few bytes", 2, 2, 12),
            ("too many pixels", MOST_PIXELS, 2, 0),
            ("a size that overflows", usize::MAX, 2, 0),
        ];
        for (name, width, height, len) in cases {
            assert!(png_of(width, height, vec![0; len]).is_none(), "{name}");
        }
    }
}
