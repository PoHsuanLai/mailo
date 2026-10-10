//! The cache directory: the atomic store, and reading it back once at startup.

use super::decode::SIDE;
use super::{IconError, PNG_MAGIC};
use crate::provider::Provider;
use base64::Engine as _;
use std::path::{Path, PathBuf};

/// Icons read once from the cache directory, each drawn ahead at the sizes a window asks for.
///
/// Missing files are absent. The chip then draws the letter, which is also what
/// a window with the setting on letters draws even when the file is there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    uris: [Vec<(u32, String)>; 6],
}

impl Loaded {
    /// Read every cached PNG under `dir`, and draw it at each of `sides` (in device pixels): the
    /// renderer then puts the picture on the screen pixel for pixel instead of resampling the
    /// 96 px file on the fly, which softens it. A file that is not a cached PNG is skipped.
    pub fn read(dir: &Path, sides: &[u32]) -> Self {
        let mut loaded = Self::default();
        for provider in Provider::ALL {
            let Some(path) = cached(dir, provider) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(image) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            else {
                continue;
            };
            loaded.uris[slot(provider)] = sides
                .iter()
                .filter_map(|&side| Some((side, data_uri(&drawn_at(&image, &bytes, side)?))))
                .collect();
        }
        loaded
    }

    /// The `data:image/png;base64,` URI for `provider` drawn `side` device pixels across, when one
    /// was cached: the size read for exactly that side, else the next larger, else the largest.
    pub fn uri(&self, provider: Provider, side: u32) -> Option<String> {
        let drawn = &self.uris[slot(provider)];
        drawn
            .iter()
            .filter(|(at, _)| *at >= side)
            .min_by_key(|(at, _)| *at)
            .or_else(|| drawn.iter().max_by_key(|(at, _)| *at))
            .map(|(_, uri)| uri.clone())
    }
}

/// `image` (whose file is `png`) as a PNG `side` px square: the file itself at its own size.
fn drawn_at(image: &image::DynamicImage, png: &[u8], side: u32) -> Option<Vec<u8>> {
    if side == SIDE {
        return Some(png.to_vec());
    }
    if side == 0 || side > SIDE {
        return None;
    }
    let resized = image.resize_exact(side, side, image::imageops::FilterType::Lanczos3);
    let mut out = Vec::new();
    resized
        .write_with_encoder(image::codecs::png::PngEncoder::new(&mut out))
        .ok()?;
    Some(out)
}

/// The side of a square PNG from its header, without decoding it.
fn png_side(bytes: &[u8]) -> Option<u32> {
    if !bytes.starts_with(&PNG_MAGIC) || bytes.get(12..16) != Some(b"IHDR".as_slice()) {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    (width == height).then_some(width)
}

/// The cache file for `provider`, when it is a regular file of today's size: one written at
/// another size predates drawing at the screen's scale, and is fetched again.
pub(crate) fn cached(dir: &Path, provider: Provider) -> Option<PathBuf> {
    let path = dir.join(format!("{}.png", file_stem(provider)));
    if !path.is_file() {
        return None;
    }
    let mut head = [0u8; 24];
    let mut file = std::fs::File::open(&path).ok()?;
    std::io::Read::read_exact(&mut file, &mut head).ok()?;
    (png_side(&head) == Some(SIDE)).then_some(path)
}

/// Write `png` as `<provider>.png`, via a temporary file in the same directory.
///
/// A failure removes the temporary file. The destination is replaced only by
/// rename, so a crash mid-write cannot leave a half-written PNG under the name
/// [`cached`] reads.
pub(crate) fn store(dir: &Path, provider: Provider, png: &[u8]) -> Result<(), IconError> {
    std::fs::create_dir_all(dir)
        .map_err(|err| IconError::Store(format!("{}: {err}", dir.display())))?;
    let name = format!("{}.png", file_stem(provider));
    let dest = dir.join(&name);
    let part = dir.join(format!(".{name}.part"));
    if let Err(err) = std::fs::write(&part, png) {
        let _ = std::fs::remove_file(&part);
        return Err(IconError::Store(format!("{}: {err}", part.display())));
    }
    if let Err(err) = std::fs::rename(&part, &dest) {
        let _ = std::fs::remove_file(&part);
        return Err(IconError::Store(format!("{}: {err}", dest.display())));
    }
    Ok(())
}

/// The name a provider's file and its report line go by.
pub fn file_stem(provider: Provider) -> &'static str {
    match provider {
        Provider::Google => "google",
        Provider::Microsoft => "microsoft",
        Provider::Fastmail => "fastmail",
        Provider::Icloud => "icloud",
        Provider::Yahoo => "yahoo",
        Provider::Imap => "imap",
    }
}

fn slot(provider: Provider) -> usize {
    match provider {
        Provider::Google => 0,
        Provider::Microsoft => 1,
        Provider::Fastmail => 2,
        Provider::Icloud => 3,
        Provider::Yahoo => 4,
        Provider::Imap => 5,
    }
}

fn data_uri(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}
