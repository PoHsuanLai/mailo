//! A provider's own icon, fetched once and cached.
//!
//! The address is a fixed table keyed by [`super::Provider`]. Nothing here reads a
//! message, so a sender domain cannot become a request. The letter on the chip is
//! what shows until a file is cached, and whenever the setting says letters.
//!
//! Cached files live at `<cache>/mailo/providers/<provider>.png` (`<cache>` is `$XDG_CACHE_HOME`
//! on Linux), 96 px square, and do not expire; one of another size was written before marks were
//! drawn at the screen's scale and counts as not cached, so it is fetched again. The bytes shown in
//! the window are `data:image/png;base64,` URIs read once at startup, one per size a window draws
//! the mark at ([`Loaded::read`]): a `file:` URL would be the protocol-handler problem in F42.

mod cache;
mod decode;
mod fetch;
mod refresh;
#[cfg(test)]
mod tests;

pub use cache::{Loaded, file_stem};
pub use refresh::{fetch_if_missing, missing, providers_of, refresh};

use mail_domain::{Retry, Retryable};
use std::time::Duration;

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// Refused before any decoder runs.
const MAX_BYTES: usize = 256 * 1024;

/// Why an icon was not cached.
#[derive(Debug, thiserror::Error)]
pub enum IconError {
    /// [`url`](fetch::url) is [`None`] for this provider.
    #[error("this provider has no icon")]
    Unmapped,
    /// The body was over 256 KiB, and was not decoded.
    #[error("the icon is {bytes} bytes, and anything over 256 KiB is refused before decoding")]
    TooLarge { bytes: usize },
    /// The magic bytes were neither PNG nor ICO.
    #[error("not a png or an ico")]
    Unrecognized,
    /// A frame was over 256 px. It is refused rather than scaled down.
    #[error("a frame is {width} by {height}, and anything over 256 px is refused")]
    Dimensions { width: u32, height: u32 },
    /// The image, or every frame in the directory, has no width or no height.
    #[error("no frame has a size")]
    NoFrame,
    /// The magic matched and the decoder still refused the bytes.
    #[error("{0}")]
    Decode(String),
    /// The address, or a redirect, was not https.
    #[error("the address is not https")]
    NotHttps,
    /// The server answered, and not with a body we can keep.
    #[error("the server answered {0}")]
    Status(u16),
    /// The request did not complete.
    #[error("{0}")]
    Fetch(String),
    /// The cache file could not be written.
    #[error("{0}")]
    Store(String),
}

impl Retryable for IconError {
    fn retry(&self) -> Retry {
        match self {
            IconError::Fetch(_) | IconError::Store(_) | IconError::Status(_) => {
                Retry::After(Duration::from_secs(5))
            }
            IconError::Unmapped
            | IconError::TooLarge { .. }
            | IconError::Unrecognized
            | IconError::Dimensions { .. }
            | IconError::NoFrame
            | IconError::Decode(_)
            | IconError::NotHttps => Retry::Fatal(self.to_string()),
        }
    }
}
