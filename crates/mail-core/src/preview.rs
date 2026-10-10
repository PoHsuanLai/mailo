//! Pictures of attachments that are already here: a thumbnail for the reader's strip, and a
//! larger picture, or a PDF's pages, for the viewer it opens.
//!
//! Only stored bytes are ever read. A part still on the server has no preview until it is
//! downloaded, because fetching it to draw a thumbnail would tell its sender it was looked at,
//! and would do so for every message scrolled past.
//!
//! What a part is comes from its first bytes, never its declared type or its name: both are the
//! sender's claim. PNG, JPEG, GIF (its first frame) and WebP are drawn, and a PDF's pages. SVG is
//! never drawn: it is a document, with scripts and references of its own, not a picture.
//!
//! Every image is measured from its header before a pixel is decoded, and one past
//! [`MAX_SIDE`] or [`MAX_PIXELS`] is refused with a sentence instead: a few hundred bytes can
//! claim a picture that would take gigabytes to hold. The decoder is given the same limits, so a
//! header that lies about its size still cannot allocate past [`MAX_ALLOC`].

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use image::{ImageFormat, ImageReader, Limits};
use mail_domain::MessageId;
use mail_store::{SqliteStore, Store};

/// The most bytes of an image read to preview it. A photograph from a phone is a few MB.
pub const MAX_IMAGE_BYTES: u64 = 32 << 20;

/// The most bytes of a PDF read to preview it.
pub const MAX_PDF_BYTES: u64 = 64 << 20;

/// The longest side, in pixels, of an image decoded to preview it.
pub const MAX_SIDE: u32 = 12_000;

/// The most pixels an image decoded to preview it may have: 40 megapixels, 160 MB as RGBA.
pub const MAX_PIXELS: u64 = 40_000_000;

/// The most the decoder may allocate for one image.
const MAX_ALLOC: u64 = 256 << 20;

/// How many bytes decide what a part is.
const SNIFF: usize = 16;

/// What a stored part's first bytes say it is, of what can be previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    WebP,
    Pdf,
}

impl Kind {
    fn format(self) -> Option<ImageFormat> {
        match self {
            Kind::Png => Some(ImageFormat::Png),
            Kind::Jpeg => Some(ImageFormat::Jpeg),
            Kind::Gif => Some(ImageFormat::Gif),
            Kind::WebP => Some(ImageFormat::WebP),
            Kind::Pdf => None,
        }
    }

    /// The most bytes of this kind read to preview it.
    fn max_bytes(self) -> u64 {
        match self {
            Kind::Pdf => MAX_PDF_BYTES,
            Kind::Png | Kind::Jpeg | Kind::Gif | Kind::WebP => MAX_IMAGE_BYTES,
        }
    }
}

/// What `head`, a part's first bytes, is: one of the kinds drawn, or `None` for anything else,
/// SVG included.
pub fn sniff(head: &[u8]) -> Option<Kind> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Kind::Png)
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Kind::Jpeg)
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some(Kind::Gif)
    } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some(Kind::WebP)
    } else if head.starts_with(b"%PDF-") {
        Some(Kind::Pdf)
    } else {
        None
    }
}

/// The room a picture is fitted into, in device pixels. The aspect is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fit {
    pub width: u32,
    pub height: u32,
}

/// A strip thumbnail: 56 logical pixels square, drawn at up to 2x.
pub const THUMB: Fit = Fit {
    width: 112,
    height: 112,
};

/// The viewer's picture.
pub const VIEW: Fit = Fit {
    width: 1600,
    height: 1200,
};

/// The viewer's PDF page.
pub const PAGE: Fit = Fit {
    width: 1200,
    height: 1600,
};

/// A picture ready to draw: a `data:` URI of a PNG, and its size in pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub uri: String,
    pub width: u32,
    pub height: u32,
}

/// Why a part here is not drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Its header claims more pixels than are decoded to preview anything.
    TooLarge { width: u32, height: u32 },
    /// More bytes than are read to preview a part of its kind.
    TooManyBytes(u64),
    /// A PDF that needs a password.
    Locked,
    /// A PDF with no pages.
    Empty,
    /// Damaged, or not what its first bytes said.
    Unreadable,
}

impl Refusal {
    /// What the strip and the viewer say instead of a picture.
    pub fn sentence(self) -> String {
        match self {
            Refusal::TooLarge { width, height } => {
                format!("Too large to preview ({width} × {height})")
            }
            Refusal::TooManyBytes(size) => {
                format!("Too large to preview ({})", crate::attach::human_size(size))
            }
            Refusal::Locked => "Locked PDF, no preview".to_owned(),
            Refusal::Empty => "A PDF with no pages".to_owned(),
            Refusal::Unreadable => "No preview: the file could not be read".to_owned(),
        }
    }
}

/// Why a part has no preview at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unshown {
    /// Still on the server: nothing is fetched to preview it.
    NotHere,
    /// Not a kind that is drawn.
    NotAPicture,
    /// A kind that is drawn, refused.
    Refused(Refusal),
    /// The store could not answer.
    Store(String),
}

/// Attachment `index` of `message`'s stored bytes, if it is a kind that is drawn and not too
/// large to read. Blocking: reads the store.
pub fn load(
    store: &SqliteStore,
    message: MessageId,
    index: usize,
) -> Result<(Kind, Vec<u8>), Unshown> {
    let message = store
        .message(message)
        .map_err(|e| Unshown::Store(e.to_string()))?;
    let attachment = message
        .attachments
        .get(index)
        .ok_or_else(|| Unshown::Store(format!("there is no attachment {index}")))?;
    let blob = attachment.blob().ok_or(Unshown::NotHere)?;
    let blobs = store.blobs();
    let head = blobs
        .head(blob, SNIFF)
        .map_err(|e| Unshown::Store(e.to_string()))?;
    let kind = sniff(&head).ok_or(Unshown::NotAPicture)?;
    let size = blobs
        .size(blob)
        .map_err(|e| Unshown::Store(e.to_string()))?;
    if size > kind.max_bytes() {
        return Err(Unshown::Refused(Refusal::TooManyBytes(size)));
    }
    let bytes = blobs.get(blob).map_err(|e| Unshown::Store(e.to_string()))?;
    Ok((kind, bytes))
}

/// Whether an image of `width` × `height` is decoded to preview it.
pub fn decodable(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// `bytes`, an image of `kind`, fitted into `fit` and written as a PNG. The header is read
/// first, and an image [`decodable`] refuses is never decoded.
pub fn picture(bytes: &[u8], kind: Kind, fit: Fit) -> Result<Picture, Refusal> {
    let Some(format) = kind.format() else {
        return Err(Refusal::Unreadable);
    };
    // The header alone, under the reader's own default limits: the claim is what is measured,
    // so the preview's tighter limits would refuse it here without saying what it claimed.
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| Refusal::Unreadable)?;
    if !decodable(width, height) {
        return Err(Refusal::TooLarge { width, height });
    }
    let decode = || {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits());
        reader.decode()
    };
    let decoded = catch_unwind(AssertUnwindSafe(decode))
        .map_err(|_| Refusal::Unreadable)?
        .map_err(|_| Refusal::Unreadable)?;
    let shown = if decoded.width() > fit.width || decoded.height() > fit.height {
        decoded.thumbnail(fit.width, fit.height)
    } else {
        decoded
    };
    let mut png = Vec::new();
    shown
        .to_rgba8()
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|_| Refusal::Unreadable)?;
    Ok(Picture {
        uri: png_uri(&png),
        width: shown.width(),
        height: shown.height(),
    })
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    limits
}

fn png_uri(png: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// One page of a PDF, drawn, and how many it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub picture: Picture,
    /// Which page, from 0: the one asked for, held to the last there is.
    pub number: u32,
    pub count: u32,
}

/// Page `number` (from 0) of the PDF `bytes`, fitted into `fit` on white. A number past the end
/// is the last page. A panic in the reader on hostile bytes reads as unreadable.
pub fn pdf_page(bytes: Vec<u8>, number: u32, fit: Fit) -> Result<Page, Refusal> {
    catch_unwind(AssertUnwindSafe(|| render_page(bytes, number, fit)))
        .unwrap_or(Err(Refusal::Unreadable))
}

fn render_page(bytes: Vec<u8>, number: u32, fit: Fit) -> Result<Page, Refusal> {
    use pdfrum::{Document, Error, RenderOptions, VelloCpuBackend};
    let doc = match Document::from_bytes(bytes) {
        Ok(doc) => doc,
        Err(Error::WrongPassword) => return Err(Refusal::Locked),
        Err(_) => return Err(Refusal::Unreadable),
    };
    let count = doc.page_count();
    if count == 0 {
        return Err(Refusal::Empty);
    }
    let number = number.min(count - 1);
    let page = doc.page(number).map_err(|_| Refusal::Unreadable)?;
    let (width, height) = (page.width(), page.height());
    if !(width > 0.0 && height > 0.0) {
        return Err(Refusal::Unreadable);
    }
    let scale = (f64::from(fit.width) / width).min(f64::from(fit.height) / height);
    let options = RenderOptions::builder()
        .scale(scale)
        .background(pdfrum::Color::WHITE)
        .build();
    let pixmap = page
        .render_with(VelloCpuBackend, &options)
        .map_err(|_| Refusal::Unreadable)?;
    let (w, h) = (pixmap.width(), pixmap.height());
    let rgba =
        image::RgbaImage::from_raw(w, h, pixmap.to_straight_rgba()).ok_or(Refusal::Unreadable)?;
    let mut png = Vec::new();
    rgba.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|_| Refusal::Unreadable)?;
    Ok(Page {
        picture: Picture {
            uri: png_uri(&png),
            width: w,
            height: h,
        },
        number,
        count,
    })
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
