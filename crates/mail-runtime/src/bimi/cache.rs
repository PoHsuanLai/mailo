//! Drawn logos on disk, and the domains found to have none, each for a while.
//!
//! One index file per domain, `<domain>.json`, saying when it was looked up and which drawing it
//! came to, if any; the drawing is `<domain>.<hash>.png`, the hash over the PNG. Keyed by the
//! domain and the hash, so a domain that changes its logo gets a new file rather than a
//! half-overwritten one. The domain is a checked DNS name ([`super::normalise`]), so it is a
//! safe file name. Written through a temporary file and a rename.

use chrono::{DateTime, TimeDelta, Utc};
use sha2::Digest as _;
use std::path::{Path, PathBuf};

/// How long a drawn logo is used before it is looked up again.
pub const LOGO_FOR: TimeDelta = TimeDelta::days(7);

/// How long a domain found to have no logo is not asked again.
pub const NONE_FOR: TimeDelta = TimeDelta::days(1);

/// What the cache knows about a domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cached {
    /// Its logo, drawn: PNG bytes.
    Logo(Vec<u8>),
    /// It was found to have none, recently.
    None,
    /// Nothing recent: look it up.
    Unknown,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Index {
    checked: DateTime<Utc>,
    /// The drawing's hash, hex; `None` for a domain with no logo.
    logo: Option<String>,
}

/// What the cache under `dir` knows about `domain` at `now`.
pub fn cached(dir: &Path, domain: &str, now: DateTime<Utc>) -> Cached {
    let Some(domain) = super::normalise(domain) else {
        return Cached::Unknown;
    };
    let Some(index) = std::fs::read(index_path(dir, &domain))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
    else {
        return Cached::Unknown;
    };
    // A date in the future is a clock that moved back, or a file someone wrote: not fresh.
    let age = now - index.checked;
    if age < TimeDelta::zero() {
        return Cached::Unknown;
    }
    match index.logo {
        None if age < NONE_FOR => Cached::None,
        Some(hash) if age < LOGO_FOR && is_hex(&hash) => {
            match std::fs::read(png_path(dir, &domain, &hash)) {
                Ok(png) if png.starts_with(PNG_MAGIC) && hex(&png) == hash => Cached::Logo(png),
                _ => Cached::Unknown,
            }
        }
        _ => Cached::Unknown,
    }
}

/// Remember `domain`'s drawing (`None` for no logo) as of `now`. The drawing it replaces, if
/// another, is removed.
pub fn remember(
    dir: &Path,
    domain: &str,
    png: Option<&[u8]>,
    now: DateTime<Utc>,
) -> std::io::Result<()> {
    let Some(domain) = super::normalise(domain) else {
        return Err(std::io::Error::other("not a domain"));
    };
    std::fs::create_dir_all(dir)?;
    let before = std::fs::read(index_path(dir, &domain))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
        .and_then(|index| index.logo);
    let hash = png.map(hex);
    if let (Some(png), Some(hash)) = (png, &hash) {
        write_atomic(&png_path(dir, &domain, hash), png)?;
    }
    let index = Index {
        checked: now,
        logo: hash.clone(),
    };
    let json = serde_json::to_vec(&index).map_err(std::io::Error::other)?;
    write_atomic(&index_path(dir, &domain), &json)?;
    if let Some(old) = before
        && Some(&old) != hash.as_ref()
        && is_hex(&old)
    {
        let _ = std::fs::remove_file(png_path(dir, &domain, &old));
    }
    Ok(())
}

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

fn index_path(dir: &Path, domain: &str) -> PathBuf {
    dir.join(format!("{domain}.json"))
}

fn png_path(dir: &Path, domain: &str, hash: &str) -> PathBuf {
    dir.join(format!("{domain}.{hash}.png"))
}

/// The first 16 bytes of the SHA-256, hex.
fn hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn is_hex(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = path.with_file_name(format!(".{name}.part"));
    if let Err(e) = std::fs::write(&part, bytes) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn png(n: u8) -> Vec<u8> {
        let mut out = PNG_MAGIC.to_vec();
        out.push(n);
        out
    }

    #[test]
    fn a_logo_is_kept_a_week_and_no_logo_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 9, 27, 12, 0, 0).unwrap();
        assert_eq!(cached(dir.path(), "brand.example", at), Cached::Unknown);

        remember(dir.path(), "Brand.Example", Some(&png(1)), at).unwrap();
        let cases = [
            (TimeDelta::zero(), Cached::Logo(png(1))),
            (TimeDelta::days(6), Cached::Logo(png(1))),
            (TimeDelta::days(8), Cached::Unknown),
            (TimeDelta::days(-1), Cached::Unknown),
        ];
        for (later, want) in cases {
            assert_eq!(
                cached(dir.path(), "brand.example", at + later),
                want,
                "{later}"
            );
        }

        remember(dir.path(), "none.example", None, at).unwrap();
        assert_eq!(
            cached(dir.path(), "none.example", at + TimeDelta::hours(23)),
            Cached::None
        );
        assert_eq!(
            cached(dir.path(), "none.example", at + TimeDelta::hours(25)),
            Cached::Unknown
        );
    }

    #[test]
    fn a_new_logo_replaces_the_old_file_and_a_damaged_one_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 9, 27, 12, 0, 0).unwrap();
        remember(dir.path(), "brand.example", Some(&png(1)), at).unwrap();
        remember(dir.path(), "brand.example", Some(&png(2)), at).unwrap();
        assert_eq!(
            cached(dir.path(), "brand.example", at),
            Cached::Logo(png(2))
        );
        let pngs = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "png"))
            })
            .count();
        assert_eq!(pngs, 1, "the old drawing is gone");

        let index: Index = serde_json::from_slice(
            &std::fs::read(index_path(dir.path(), "brand.example")).unwrap(),
        )
        .unwrap();
        let path = png_path(dir.path(), "brand.example", &index.logo.unwrap());
        std::fs::write(&path, b"not a png").unwrap();
        assert_eq!(cached(dir.path(), "brand.example", at), Cached::Unknown);
    }

    #[test]
    fn a_name_that_is_not_a_domain_is_never_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 9, 27, 12, 0, 0).unwrap();
        assert!(remember(dir.path(), "../escape", None, at).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
