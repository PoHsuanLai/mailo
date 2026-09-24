use super::cache::{Loaded, cached, file_stem, store};
use super::chip::{ChipPlace, ProvChip};
use super::decode::decode;
use super::fetch::{client, fetch, url};
use super::refresh::{providers_of, report, user_binary};
use super::{IconError, PNG_MAGIC};
use crate::provider::Provider;
use crate::view::Marks;
use dioxus::prelude::*;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::codecs::png::{PngDecoder, PngEncoder};
use image::{DynamicImage, ImageDecoder};
use image::{ExtendedColorType, GenericImageView, ImageEncoder};
use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing, SaslMech, Tls, Username};
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

/// `url` takes a [`Provider`] and nothing else. A sender domain cannot be passed.
fn _url_takes_a_provider(provider: Provider) -> Option<&'static str> {
    url(provider)
}

#[test]
fn the_table_is_exactly_those_five_addresses() {
    const EXPECTED: &[(Provider, Option<&str>)] = &[
        (
            Provider::Google,
            Some("https://ssl.gstatic.com/ui/v1/icons/mail/rfr/gmail.ico"),
        ),
        (
            Provider::Microsoft,
            Some("https://outlook.live.com/favicon.ico"),
        ),
        (
            Provider::Fastmail,
            Some("https://www.fastmail.com/favicon.ico"),
        ),
        (Provider::Icloud, Some("https://www.icloud.com/favicon.ico")),
        (Provider::Yahoo, Some("https://mail.yahoo.com/favicon.ico")),
        (Provider::Imap, None),
    ];
    assert_eq!(Provider::ALL.len(), EXPECTED.len());
    for &(provider, address) in EXPECTED {
        assert_eq!(url(provider), address, "{provider:?}");
        if let Some(address) = address {
            assert!(address.starts_with("https://"), "{provider:?} {address}");
        }
    }
    assert_eq!(_url_takes_a_provider(Provider::Imap), None);
}

#[test]
fn the_word_favicon_occurs_only_in_the_icon_module() {
    // A sender-domain fetch would name that word next to a host taken from a message.
    // The address table in `fetch.rs`, and these tests of it, are the only places it
    // is allowed: every file that says it lives under `provider/icon/`.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    walk(&root, &root, &mut hits);
    hits.sort();
    assert!(
        hits.contains(&"provider/icon/fetch.rs".to_owned()),
        "the address table moved: {hits:?}"
    );
    assert!(
        hits.iter().all(|hit| hit.starts_with("provider/icon/")),
        "{hits:?}"
    );
}

fn walk(dir: &Path, root: &Path, hits: &mut Vec<String>) {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("{}: {err}", dir.display()))
        .map(|entry| entry.unwrap_or_else(|err| panic!("{err}")).path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            walk(&path, root, hits);
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        if text.contains("favicon") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            hits.push(rel);
        }
    }
}

#[test]
fn decode_keeps_the_largest_frame_at_most_64_px() {
    // A 512 px frame is rejected, not downscaled. A frame between 65 and 256 is
    // skipped; if it was the only one, there is nothing to draw.
    let cases: &[(&str, Vec<u8>, DecodeExpect)] = &[
        (
            "ico 16 red, 32 green, 48 blue",
            ico(&[
                (16, [255, 0, 0, 255]),
                (32, [0, 255, 0, 255]),
                (48, [0, 0, 255, 255]),
            ]),
            DecodeExpect::Blue,
        ),
        (
            "ico 96 green and 16 red skips the 96",
            ico(&[(96, [0, 255, 0, 255]), (16, [255, 0, 0, 255])]),
            DecodeExpect::Red,
        ),
        (
            "png 16 red",
            solid_png(16, 16, [255, 0, 0, 255]),
            DecodeExpect::Red,
        ),
        (
            "garbage",
            b"this is not an image".to_vec(),
            DecodeExpect::Unrecognized,
        ),
        (
            "cursor magic is not an ico",
            vec![0, 0, 2, 0, 0, 0],
            DecodeExpect::Unrecognized,
        ),
        (
            "300 KiB is refused before decoding",
            padded_png(300 * 1024),
            DecodeExpect::TooLarge,
        ),
        (
            "512 png is refused, not downscaled",
            solid_png(512, 512, [0, 0, 255, 255]),
            DecodeExpect::Dimensions,
        ),
        (
            "80 png has no frame at most 64",
            solid_png(80, 80, [0, 255, 0, 255]),
            DecodeExpect::NoFrame,
        ),
    ];
    for (name, bytes, expect) in cases {
        let got = decode(bytes);
        match expect {
            DecodeExpect::Red => {
                assert_channel(name, &got.unwrap_or_else(|err| panic!("{name}: {err}")), 0)
            }
            DecodeExpect::Blue => {
                assert_channel(name, &got.unwrap_or_else(|err| panic!("{name}: {err}")), 2)
            }
            DecodeExpect::Unrecognized => {
                assert!(
                    matches!(got, Err(IconError::Unrecognized)),
                    "{name}: {got:?}"
                )
            }
            DecodeExpect::TooLarge => {
                assert!(
                    matches!(got, Err(IconError::TooLarge { .. })),
                    "{name}: {got:?}"
                )
            }
            DecodeExpect::Dimensions => {
                assert!(
                    matches!(
                        got,
                        Err(IconError::Dimensions {
                            width: 512,
                            height: 512
                        })
                    ),
                    "{name}: {got:?}"
                )
            }
            DecodeExpect::NoFrame => {
                assert!(matches!(got, Err(IconError::NoFrame)), "{name}: {got:?}")
            }
        }
    }
}

#[derive(Clone, Copy)]
enum DecodeExpect {
    Red,
    Blue,
    Unrecognized,
    TooLarge,
    Dimensions,
    NoFrame,
}

fn assert_channel(name: &str, png: &[u8], channel: usize) {
    assert!(
        png.starts_with(&PNG_MAGIC),
        "{name} did not come out as a png"
    );
    let decoder = PngDecoder::new(Cursor::new(png)).unwrap_or_else(|err| panic!("{name}: {err}"));
    assert_eq!(decoder.dimensions(), (32, 32), "{name}");
    let image = DynamicImage::from_decoder(decoder).unwrap_or_else(|err| panic!("{name}: {err}"));
    let px = image.get_pixel(16, 16).0;
    assert!(px[3] > 200, "{name} center is transparent: {px:?}");
    assert!(px[channel] > 200, "{name} picked the wrong frame: {px:?}");
    for (index, value) in px.into_iter().take(3).enumerate() {
        if index != channel {
            assert!(value < 40, "{name} picked the wrong frame: {px:?}");
        }
    }
}

fn solid_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let raw = solid_raw(width, height, rgba);
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&raw, width, height, ExtendedColorType::Rgba8)
        .unwrap_or_else(|err| panic!("{width}x{height}: {err}"));
    png
}

fn solid_raw(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        raw.extend_from_slice(&rgba);
    }
    raw
}

fn ico(frames: &[(u32, [u8; 4])]) -> Vec<u8> {
    let encoded: Vec<_> = frames
        .iter()
        .map(|(side, rgba)| {
            let raw = solid_raw(*side, *side, *rgba);
            IcoFrame::as_png(&raw, *side, *side, ExtendedColorType::Rgba8)
                .unwrap_or_else(|err| panic!("{side}: {err}"))
        })
        .collect();
    let mut out = Vec::new();
    IcoEncoder::new(&mut out)
        .encode_images(&encoded)
        .unwrap_or_else(|err| panic!("{err}"));
    out
}

fn padded_png(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    bytes[..PNG_MAGIC.len()].copy_from_slice(&PNG_MAGIC);
    bytes
}

#[test]
fn a_cached_icon_round_trips_and_a_failed_write_leaves_no_partial_file() {
    let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
    let png = decode(&solid_png(16, 16, [9, 8, 7, 255])).unwrap_or_else(|err| panic!("{err}"));
    store(dir.path(), Provider::Yahoo, &png).unwrap_or_else(|err| panic!("{err}"));
    let path =
        cached(dir.path(), Provider::Yahoo).unwrap_or_else(|| panic!("yahoo was not cached"));
    assert_eq!(
        std::fs::read(&path).unwrap_or_else(|err| panic!("{err}")),
        png
    );
    assert_eq!(names(dir.path()), vec!["yahoo.png".to_owned()]);

    // A crash after the temporary file is written, and before the rename.
    std::fs::write(dir.path().join(".fastmail.png.part"), &png)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(cached(dir.path(), Provider::Fastmail).is_none());
    let loaded = Loaded::read(dir.path());
    assert!(loaded.uri(Provider::Yahoo).is_some());
    assert!(loaded.uri(Provider::Fastmail).is_none());
    assert!(
        !loaded
            .uri(Provider::Yahoo)
            .unwrap_or_default()
            .contains("file:")
    );
    assert!(
        loaded
            .uri(Provider::Yahoo)
            .unwrap_or_default()
            .starts_with("data:image/png;base64,")
    );

    let dest = dir.path().join("google.png");
    std::fs::create_dir(&dest).unwrap_or_else(|err| panic!("{err}"));
    let err = store(dir.path(), Provider::Google, &png).expect_err("rename onto a directory");
    assert!(matches!(err, IconError::Store(_)), "{err}");
    assert!(dest.is_dir(), "the failed store replaced the destination");
    assert!(cached(dir.path(), Provider::Google).is_none());
    // The fastmail part is the crash above, which nobody cleaned up. The failed
    // store removes its own temporary file; that is the one that must be gone.
    assert!(
        !dir.path().join(".google.png.part").exists(),
        "the failed store left its temporary file"
    );
}

fn names(dir: &Path) -> Vec<String> {
    let mut got: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("{err}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|err| panic!("{err}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    got.sort();
    got
}

#[test]
fn the_refresh_report_names_what_landed() {
    let lines = report(&[
        (Provider::Google, Ok(12)),
        (Provider::Imap, Err(IconError::Unmapped)),
        (Provider::Yahoo, Err(IconError::NotHttps)),
    ]);
    assert_eq!(
        lines,
        "google: wrote 12 bytes\nimap: letters only\nyahoo: not updated\n"
    );
}

#[test]
fn configured_accounts_contribute_their_provider_once() {
    let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
    let store =
        mail_store::SqliteStore::in_memory(dir.path()).unwrap_or_else(|err| panic!("{err}"));
    for (n, (address, host)) in [
        ("a@gmail.com", "imap.gmail.com"),
        ("b@gmail.com", "imap.gmail.com"),
        ("c@fastmail.com", "imap.fastmail.com"),
    ]
    .into_iter()
    .enumerate()
    {
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
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    mail_domain::AccountId::generate().to_string(),
                    address,
                    serde_json::to_string(&plan).unwrap_or_else(|err| panic!("{err}")),
                    format!("2026-01-0{}T00:00:00Z", n + 1),
                ],
            )
            .unwrap_or_else(|err| panic!("{err}"));
    }
    assert_eq!(
        providers_of(&store).unwrap_or_else(|err| panic!("{err}")),
        vec![Provider::Google, Provider::Fastmail]
    );
}

#[test]
fn tests_do_not_count_as_the_user_binary() {
    assert!(
        !user_binary(),
        "a test would fetch into the real cache: {:?}",
        std::env::current_exe()
    );
}

#[test]
fn the_http_client_crate_has_no_cookie_jar() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .unwrap_or_else(|err| panic!("{err}"));
    let reqwest = manifest
        .lines()
        .find(|line| line.contains("reqwest"))
        .unwrap_or_else(|| panic!("no reqwest line:\n{manifest}"));
    assert!(
        !reqwest.contains("cookies"),
        "a cookie jar would be sent with the icon request: {reqwest}"
    );
}

#[tokio::test]
async fn an_http_server_is_not_contacted() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|err| panic!("{err}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("{err}"))
        .port();
    let http = client().unwrap_or_else(|err| panic!("{err}"));
    let address = format!("http://127.0.0.1:{port}/favicon.ico");
    let sent = tokio::time::timeout(Duration::from_secs(1), http.get(&address).send()).await;
    let err = sent
        .unwrap_or_else(|_| panic!("the client waited on a cleartext server"))
        .expect_err("http is not an icon fetch");
    assert!(
        err.to_string().to_ascii_lowercase().contains("https") || err.is_builder(),
        "{err}"
    );
    let hit = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(hit.is_err(), "the client connected to a cleartext server");
}

#[tokio::test]
async fn the_chip_draws_the_cached_icon_or_the_letter() {
    let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
    let png = decode(&solid_png(16, 16, [255, 0, 0, 255])).unwrap_or_else(|err| panic!("{err}"));
    store(dir.path(), Provider::Google, &png).unwrap_or_else(|err| panic!("{err}"));
    let loaded = Loaded::read(dir.path());
    let uri = loaded
        .uri(Provider::Google)
        .unwrap_or_else(|| panic!("no uri"));
    let cases = [
        (Marks::Icons, true, ChipPlace::Inline, true),
        (Marks::Icons, true, ChipPlace::Row, true),
        (Marks::Letters, true, ChipPlace::Inline, false),
        (Marks::Icons, false, ChipPlace::Inline, false),
    ];
    for (marks, with_file, place, image) in cases {
        let html = render_chip(
            marks,
            if with_file {
                loaded.clone()
            } else {
                Loaded::default()
            },
            place,
        );
        let name = format!("{marks:?} file={with_file} {place:?}");
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

fn render_chip(marks: Marks, loaded: Loaded, place: ChipPlace) -> String {
    let mut dom = VirtualDom::new(ChipHarness)
        .with_root_context(ChipCase { marks, place })
        .with_root_context(loaded);
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

#[derive(Clone)]
struct ChipCase {
    marks: Marks,
    place: ChipPlace,
}

#[component]
fn ChipHarness() -> Element {
    let case = consume_context::<ChipCase>();
    rsx! { ProvChip { provider: Provider::Google, marks: case.marks, place: case.place } }
}

#[tokio::test]
#[ignore = "fetches the five provider icons; run with --ignored --nocapture"]
async fn fetch_the_provider_icons() {
    let http = client().unwrap_or_else(|err| panic!("{err}"));
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/provider-icons");
    std::fs::create_dir_all(&out).unwrap_or_else(|err| panic!("{err}"));
    for provider in [
        Provider::Google,
        Provider::Microsoft,
        Provider::Fastmail,
        Provider::Icloud,
        Provider::Yahoo,
    ] {
        let address = url(provider).unwrap_or_else(|| panic!("{provider:?}"));
        match fetch(&http, provider).await {
            Ok(bytes) => {
                println!("{provider:?} {address} bytes={}", bytes.len());
                match decode(&bytes) {
                    Ok(png) => {
                        println!("  decoded png {}", png.len());
                        let path = out.join(format!("{}.png", file_stem(provider)));
                        std::fs::write(&path, &png).unwrap_or_else(|err| panic!("{err}"));
                        println!("  wrote {}", path.display());
                    }
                    Err(err) => println!("  decode failed: {err}"),
                }
            }
            Err(err) => println!("{provider:?} {address} FAILED {err}"),
        }
    }
}
