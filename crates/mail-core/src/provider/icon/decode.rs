//! ICO and PNG bytes in, a 32×32 PNG out.

use super::{IconError, MAX_BYTES, PNG_MAGIC};
use image::codecs::ico::IcoDecoder;
use image::codecs::png::{PngDecoder, PngEncoder};
use image::{DynamicImage, ImageDecoder};
use std::io::Cursor;

const ICO_MAGIC: [u8; 4] = [0x00, 0x00, 0x01, 0x00];
/// A frame larger than this is refused, not scaled down.
const MAX_SIDE: u32 = 256;
/// The largest frame we are willing to keep. Anything bigger is skipped.
const FRAME: u32 = 64;
const OUT: u32 = 32;

/// Decode an ICO or a PNG into a 32×32 PNG.
///
/// The format is the magic bytes, never the URL. A multi-frame ICO keeps the
/// largest frame whose sides are both at most 64 px. A frame over 256 px is
/// refused, not scaled down. More than 256 KiB is refused before decoding.
pub(crate) fn decode(bytes: &[u8]) -> Result<Vec<u8>, IconError> {
    if bytes.len() > MAX_BYTES {
        return Err(IconError::TooLarge { bytes: bytes.len() });
    }
    if bytes.starts_with(&PNG_MAGIC) {
        return decode_png(bytes);
    }
    if bytes.starts_with(&ICO_MAGIC) {
        return decode_ico(bytes);
    }
    Err(IconError::Unrecognized)
}

fn decode_png(bytes: &[u8]) -> Result<Vec<u8>, IconError> {
    let decoder = PngDecoder::new(Cursor::new(bytes)).map_err(as_decode)?;
    resize(decoder)
}

fn decode_ico(bytes: &[u8]) -> Result<Vec<u8>, IconError> {
    if bytes.len() < 6 {
        return Err(IconError::Unrecognized);
    }
    let count = usize::from(u16::from_le_bytes([bytes[4], bytes[5]]));
    let dir_bytes = count
        .checked_mul(16)
        .and_then(|n| n.checked_add(6))
        .ok_or_else(|| IconError::Decode("the icon directory does not fit".into()))?;
    if count == 0 || dir_bytes > bytes.len() {
        return Err(IconError::Decode(
            "the icon directory is empty or truncated".into(),
        ));
    }
    let mut best: Option<(u32, usize)> = None;
    for index in 0..count {
        let entry = &bytes[6 + index * 16..6 + index * 16 + 16];
        let width = ico_side(entry[0]);
        let height = ico_side(entry[1]);
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(IconError::Dimensions { width, height });
        }
        if width == 0 || height == 0 || width > FRAME || height > FRAME {
            continue;
        }
        let area = width.saturating_mul(height);
        if best.is_none_or(|(best_area, _)| area > best_area) {
            best = Some((area, index));
        }
    }
    let Some((_, index)) = best else {
        return Err(IconError::NoFrame);
    };
    let entry = &bytes[6 + index * 16..6 + index * 16 + 16];
    let size = le_u32(&entry[8..12])?;
    let offset = le_u32(&entry[12..16])?;
    let image_end = offset
        .checked_add(size)
        .ok_or_else(|| IconError::Decode("a frame's range overflows".into()))?;
    if size == 0 || offset < dir_bytes || image_end > bytes.len() {
        return Err(IconError::Decode("a frame points outside the file".into()));
    }
    let isolated = isolate(entry, &bytes[offset..image_end]);
    let decoder = IcoDecoder::new(Cursor::new(isolated)).map_err(as_decode)?;
    resize(decoder)
}

/// One-image ICO, so the decoder's "best entry" is the frame we already chose.
fn isolate(entry: &[u8], image: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(22 + image.len());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    let mut dir = [0u8; 16];
    dir[..8].copy_from_slice(&entry[..8]);
    dir[8..12].copy_from_slice(&u32::try_from(image.len()).unwrap_or(u32::MAX).to_le_bytes());
    dir[12..16].copy_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(&dir);
    out.extend_from_slice(image);
    out
}

fn resize(decoder: impl ImageDecoder) -> Result<Vec<u8>, IconError> {
    let (width, height) = decoder.dimensions();
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(IconError::Dimensions { width, height });
    }
    if width == 0 || height == 0 || width > FRAME || height > FRAME {
        return Err(IconError::NoFrame);
    }
    let image = DynamicImage::from_decoder(decoder).map_err(as_decode)?;
    let resized = image.resize_exact(OUT, OUT, image::imageops::FilterType::Triangle);
    let mut png = Vec::new();
    resized
        .write_with_encoder(PngEncoder::new(&mut png))
        .map_err(as_decode)?;
    Ok(png)
}

fn ico_side(byte: u8) -> u32 {
    match byte {
        0 => 256,
        n => u32::from(n),
    }
}

fn le_u32(bytes: &[u8]) -> Result<usize, IconError> {
    let raw: [u8; 4] = bytes
        .try_into()
        .map_err(|_| IconError::Decode("a directory entry is short".into()))?;
    usize::try_from(u32::from_le_bytes(raw))
        .map_err(|_| IconError::Decode("a frame offset does not fit".into()))
}

fn as_decode(err: image::ImageError) -> IconError {
    IconError::Decode(err.to_string())
}
