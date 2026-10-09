//! Provider marks on the real window at scale 2: the row's `@ imap` chip, a cached icon on a
//! row, and the account badge on the Space tile. Painted headlessly by Blitz over a store seeded
//! in a `TempDir`, with the icons handed in as a `Loaded` read from a `TempDir` too, so nothing
//! here reads or touches the real store, config or cache.

use ds_blitz::{FocusFallback, NetPolicy};
use ds_harness::{Backdrop, Clock, Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 200,
};

fn imap_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

fn google_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
}

fn plan(address: &str, host: &str) -> String {
    let plan = AccountPlan {
        address: address.to_owned(),
        incoming: Incoming::Imap {
            host: host.to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: host.to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    serde_json::to_string(&plan).unwrap()
}

fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    let accounts = [
        (imap_account(), "me@mail.example.test", "mail.example.test"),
        (google_account(), "me@gmail.com", "imap.gmail.com"),
    ];
    {
        let db = store.connection();
        for (n, (id, address, host)) in accounts.iter().enumerate() {
            db.execute(
                "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    id.to_string(),
                    address,
                    plan(address, host),
                    format!("2026-01-0{}T00:00:00Z", n + 1)
                ],
            )
            .unwrap();
        }
    }
    let now = chrono::Utc::now();
    for (n, (id, address, _)) in accounts.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: ada@example.test\r\nTo: {address}\r\nSubject: Note {n}\r\n\
             Date: {date}\r\nMessage-ID: <mark{n}@example.test>\r\n\r\nThe body.\r\n"
        );
        absorb(
            &store,
            id.clone(),
            MailboxRef {
                account: id.clone(),
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("mark{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    Arc::new(store)
}

/// A favicon with hard edges, cached as `mailo icons refresh` writes one: 96 px square.
fn icon_png() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(96, 96, |x, y| {
        let ring = x < 12 || y < 12 || x >= 84 || y >= 84;
        if ring {
            image::Rgba([0xea, 0x43, 0x35, 0xff])
        } else if x < 48 {
            image::Rgba([0x1a, 0x73, 0xe8, 0xff])
        } else {
            image::Rgba([0xff, 0xff, 0xff, 0xff])
        }
    });
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

struct Opened {
    harness: Harness,
    _dirs: [tempfile::TempDir; 2],
}

/// The window at scale 2 over an IMAP account and a Google one, Google's icon cached.
fn open() -> Opened {
    let mail = tempfile::tempdir().unwrap();
    let icons = tempfile::tempdir().unwrap();
    std::fs::write(icons.path().join("google.png"), icon_png()).unwrap();
    let loaded = mail_app::ui::native::read_icons(icons.path());
    let store = seeded(mail.path());
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        Some(mail_app::ui::space::built(
            vec![(
                "Mail".to_owned(),
                ds::prelude::SpaceLook::default(),
                mail_app::ui::space::Mail::over(mail_app::ui::space::Scope::All),
            )],
            0,
        )),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(loaded);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(Duration::from_millis(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 1);
    harness.advance(Duration::from_millis(600));
    Opened {
        harness,
        _dirs: [mail, icons],
    }
}

/// The pixel size of the PNG in a `data:` URI.
fn png_size(uri: &str) -> (u32, u32) {
    use base64::Engine as _;
    let data = uri
        .strip_prefix("data:image/png;base64,")
        .unwrap_or_else(|| panic!("not a png data uri: {uri:.40}"));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    let image = image::load_from_memory(&bytes).unwrap();
    (image.width(), image.height())
}

#[test]
fn each_icon_is_a_picture_as_many_pixels_across_as_the_screen_draws_it() {
    // The renderer resamples a picture to its box with a bilinear filter: one bigger or smaller
    // than the box's device pixels comes out soft. A row's mark and a tile's badge are each
    // handed one exactly their size at scale 2.
    let opened = open();
    let harness = &opened.harness;
    for selector in [
        ".via .ds-provider[*|data-kind=image] img",
        ".ds-pin-tile .ds-provider[*|data-kind=image] img",
    ] {
        let rect = harness
            .rect(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
        let src = harness
            .attr(selector, "src")
            .unwrap_or_else(|| panic!("{selector} has no src"));
        let drawn = (
            (rect.size.width.0 * 2.0).round() as u32,
            (rect.size.height.0 * 2.0).round() as u32,
        );
        assert_eq!(png_size(&src), drawn, "{selector}");
    }
}

#[test]
#[ignore = "picture generator: set MAILO_SHOTS to a directory and run with --ignored"]
fn marks_at_scale_two() {
    let out = std::path::PathBuf::from(std::env::var("MAILO_SHOTS").expect("MAILO_SHOTS"));
    std::fs::create_dir_all(&out).unwrap();
    let mut opened = open();
    let harness = &mut opened.harness;
    let shot = harness.render_over(Backdrop::Scheme).unwrap();
    shot.save(out.join("window.png")).unwrap();
    let scale = 2.0;
    for (name, selector) in [
        ("row-letter", ".via .ds-provider[*|data-kind=letter]"),
        ("row-image", ".via .ds-provider[*|data-kind=image]"),
        (
            "tile-badge-letter",
            ".ds-pin-tile .ds-provider[*|data-kind=letter]",
        ),
        (
            "tile-badge-image",
            ".ds-pin-tile .ds-provider[*|data-kind=image]",
        ),
    ] {
        let Some(rect) = harness.rect(selector) else {
            eprintln!("{name}: {selector} is not drawn");
            continue;
        };
        eprintln!(
            "{name}: x {} y {} w {} h {}",
            rect.origin.x.0, rect.origin.y.0, rect.size.width.0, rect.size.height.0
        );
        let pad = 6.0;
        let x = ((rect.origin.x.0 - pad) * scale).max(0.0) as u32;
        let y = ((rect.origin.y.0 - pad) * scale).max(0.0) as u32;
        let w = ((rect.size.width.0 + 2.0 * pad) * scale) as u32;
        let h = ((rect.size.height.0 + 2.0 * pad) * scale) as u32;
        let crop = image::imageops::crop_imm(&shot, x, y, w, h).to_image();
        let big =
            image::imageops::resize(&crop, w * 8, h * 8, image::imageops::FilterType::Nearest);
        big.save(out.join(format!("{name}.png"))).unwrap();
    }
}
