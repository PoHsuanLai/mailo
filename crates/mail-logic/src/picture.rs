//! How large the pictures a message carries are sent: the composer's Picture Size.
//!
//! The choice and what it means in pixels; making a picture that size is `mail-core`'s, which
//! decodes it.

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

    /// Where it falls in [`PictureSize::ALL`]: the slot a per-size table keeps it in.
    pub fn index(self) -> usize {
        match self {
            PictureSize::Small => 0,
            PictureSize::Medium => 1,
            PictureSize::Large => 2,
            PictureSize::Actual => 3,
        }
    }
}
