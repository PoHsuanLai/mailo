//! Pictures sent smaller, as the composer's Picture Size asks: a photo re-encoded to fit a
//! longest side, at send, from the original the draft keeps.
//!
//! Only JPEG and PNG, told by their first bytes as the preview tells them (`crate::preview`):
//! a GIF may be moving and a WebP is usually small already. A picture is never made larger, and
//! one whose smaller copy comes out no smaller in bytes, as a drawing saved as PNG can, goes as it
//! was. The copy carries no metadata: `image` writes none, so a photo's EXIF, and with it where it
//! was taken, is left behind. Its orientation is applied to the pixels first, so a photo taken
//! sideways does not arrive sideways without the tag that turned it.

use std::borrow::Cow;
use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::preview::{Kind, decodable, sniff};

/// How large the pictures a message carries are sent, as Apple Mail names the choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum PictureSize {
    Small,
    Medium,
    Large,
    /// As they were attached.
    #[default]
    Actual,
}

impl PictureSize {
    /// Smallest first.
    pub const ALL: [PictureSize; 4] = [
        PictureSize::Small,
        PictureSize::Medium,
        PictureSize::Large,
        PictureSize::Actual,
    ];

    /// The longest side a picture is fitted to, in pixels; `None` for as it was.
    pub fn longest_side(self) -> Option<u32> {
        match self {
            PictureSize::Small => Some(640),
            PictureSize::Medium => Some(1280),
            PictureSize::Large => Some(2048),
            PictureSize::Actual => None,
        }
    }

    fn index(self) -> usize {
        match self {
            PictureSize::Small => 0,
            PictureSize::Medium => 1,
            PictureSize::Large => 2,
            PictureSize::Actual => 3,
        }
    }
}

/// How well a JPEG made smaller is kept: past the point where the eye tells it from the original
/// at the size it is shown, short of where the bytes grow for nothing.
const JPEG_QUALITY: u8 = 85;

/// Whether `bytes` is a picture [`sized`] can make smaller: a JPEG or a PNG.
pub fn resizable(bytes: &[u8]) -> bool {
    format_of(bytes).is_some()
}

/// The bytes `bytes` is sent as at `size`: the picture made smaller, or the original when it is
/// not a JPEG or PNG, already fits, does not decode, or comes out no smaller.
pub fn sized(bytes: &[u8], size: PictureSize) -> Cow<'_, [u8]> {
    let Some(side) = size.longest_side() else {
        return Cow::Borrowed(bytes);
    };
    match Decoded::of(bytes).and_then(|decoded| decoded.fitted(side)) {
        Some(smaller) if smaller.len() < bytes.len() => Cow::Owned(smaller),
        _ => Cow::Borrowed(bytes),
    }
}

/// How many bytes `bytes` is sent as at each size, smallest size first, by the rule [`sized`]
/// follows. Decoded once for all four.
pub fn sizes(bytes: &[u8]) -> [u64; 4] {
    let original = bytes.len() as u64;
    let mut out = [original; 4];
    let Some(decoded) = Decoded::of(bytes) else {
        return out;
    };
    for size in PictureSize::ALL {
        if let Some(smaller) = size.longest_side().and_then(|side| decoded.fitted(side)) {
            out[size.index()] = original.min(smaller.len() as u64);
        }
    }
    out
}

/// The format a picture's first bytes say it is, of the two made smaller.
fn format_of(bytes: &[u8]) -> Option<ImageFormat> {
    match sniff(bytes.get(..16).unwrap_or(bytes))? {
        Kind::Jpeg => Some(ImageFormat::Jpeg),
        Kind::Png => Some(ImageFormat::Png),
        Kind::Gif | Kind::WebP | Kind::Pdf => None,
    }
}

/// A picture read whole, turned upright, with the format it goes back out in.
struct Decoded {
    picture: DynamicImage,
    format: ImageFormat,
}

impl Decoded {
    /// `bytes` decoded, or `None` when it is not a JPEG or PNG, claims more pixels than the
    /// preview would decode, or does not decode. The decoder runs under the preview's limits, so
    /// a header that lies about its size cannot allocate past them either.
    fn of(bytes: &[u8]) -> Option<Decoded> {
        let format = format_of(bytes)?;
        let header = ImageReader::with_format(Cursor::new(bytes), format);
        // A header that does not read is no picture to make smaller: it goes as it came.
        let (width, height) = header.into_dimensions().ok()?;
        if !decodable(width, height) {
            return None;
        }
        let decode = || {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(crate::preview::limits());
            // Nor does a picture with no decoder: it goes as it came.
            let mut decoder = reader.into_decoder().ok()?;
            // A picture whose orientation does not read is taken as it is stored.
            let orientation = decoder.orientation().ok();
            // Nor one whose pixels do not decode, or ask more memory than the limits allow.
            let mut picture = DynamicImage::from_decoder(decoder).ok()?;
            if let Some(orientation) = orientation {
                picture.apply_orientation(orientation);
            }
            Some(picture)
        };
        // A decoder that panics on what it was handed has decoded nothing.
        let picture = catch_unwind(AssertUnwindSafe(decode)).ok()??;
        Some(Decoded { picture, format })
    }

    /// The picture fitted into `side` × `side` and encoded again; `None` when it already fits,
    /// since a picture is never made larger, or when it does not encode.
    fn fitted(&self, side: u32) -> Option<Vec<u8>> {
        if self.picture.width().max(self.picture.height()) <= side {
            return None;
        }
        let smaller = self.picture.resize(side, side, FilterType::Triangle);
        let mut out = Vec::new();
        match self.format {
            ImageFormat::Jpeg => {
                // JPEG holds no alpha: what had any is laid on nothing, as a JPEG reader would.
                let encoder = JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
                let opaque = DynamicImage::ImageRgb8(smaller.to_rgb8());
                // A copy that does not encode is no copy: the original goes.
                opaque.write_with_encoder(encoder).ok()?;
            }
            _ => {
                let encoder = PngEncoder::new_with_quality(
                    &mut out,
                    CompressionType::Best,
                    PngFilter::Adaptive,
                );
                // A copy that does not encode is no copy: the original goes.
                smaller.write_with_encoder(encoder).ok()?;
            }
        }
        Some(out)
    }
}

#[cfg(test)]
#[path = "pictures_tests.rs"]
mod tests;
