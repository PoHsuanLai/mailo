use super::*;

/// Two pages: 200 × 100 points with a blue band, then 200 × 300 with a red one.
const TWO_PAGES: &[u8] = include_bytes!("../tests/fixtures/two-pages.pdf");

/// `width` × `height` of one colour, written as `format`.
fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let picture = image::RgbImage::from_pixel(width, height, image::Rgb([200, 40, 90]));
    let mut out = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut out), format)
        .unwrap();
    out
}

/// A PNG whose header claims `width` × `height` and whose data is one row: a few dozen bytes
/// that would take gigabytes to decode.
fn claimed_png(width: u32, height: u32) -> Vec<u8> {
    let mut png = encoded(1, 1, ImageFormat::Png);
    // Signature (8), IHDR length (4) and type (4); then width and height, then 5 bytes of
    // depth, colour, compression, filter and interlace, then the CRC over type and data.
    png[16..20].copy_from_slice(&width.to_be_bytes());
    png[20..24].copy_from_slice(&height.to_be_bytes());
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn a_part_is_what_its_first_bytes_say() {
    let cases: &[(&str, &[u8], Option<Kind>)] = &[
        ("png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", Some(Kind::Png)),
        ("jpeg", b"\xFF\xD8\xFF\xE0\0\x10JFIF", Some(Kind::Jpeg)),
        ("gif87", b"GIF87a\x01\0\x01\0", Some(Kind::Gif)),
        ("gif89", b"GIF89a\x01\0\x01\0", Some(Kind::Gif)),
        ("webp", b"RIFF\x24\0\0\0WEBPVP8 ", Some(Kind::WebP)),
        ("riff, not webp", b"RIFF\x24\0\0\0WAVEfmt ", None),
        ("pdf", b"%PDF-1.7\n%\xE2\xE3", Some(Kind::Pdf)),
        ("svg", b"<svg xmlns=\"http:", None),
        ("svg with a prolog", b"<?xml version=\"1", None),
        ("text", b"hello, world", None),
        ("empty", b"", None),
        ("half a png signature", b"\x89PN", None),
    ];
    for (case, head, kind) in cases {
        assert_eq!(sniff(head), *kind, "{case}");
    }
}

#[test]
fn each_image_kind_is_fitted_into_the_room_at_its_own_aspect() {
    let cases = [
        ("png", Kind::Png, ImageFormat::Png),
        ("jpeg", Kind::Jpeg, ImageFormat::Jpeg),
        ("gif", Kind::Gif, ImageFormat::Gif),
        ("webp", Kind::WebP, ImageFormat::WebP),
    ];
    for (case, kind, format) in cases {
        let bytes = encoded(400, 200, format);
        assert_eq!(sniff(&bytes[..16]), Some(kind), "{case}: sniffed");
        let shown = picture(&bytes, kind, THUMB).unwrap_or_else(|why| panic!("{case}: {why:?}"));
        assert_eq!((shown.width, shown.height), (112, 56), "{case}");
        assert!(shown.uri.starts_with("data:image/png;base64,"), "{case}");
    }
}

#[test]
fn a_small_image_is_not_blown_up() {
    let bytes = encoded(30, 20, ImageFormat::Png);
    let shown = picture(&bytes, Kind::Png, THUMB).unwrap();
    assert_eq!((shown.width, shown.height), (30, 20));
}

/// A decompression bomb: the header is read, the claim refused, and nothing decoded. Were it
/// decoded, 100 000 × 100 000 RGB is 30 GB.
#[test]
fn an_image_claiming_too_many_pixels_is_refused_before_it_is_decoded() {
    let cases = [
        ("too wide", 100_000, 10, (100_000, 10)),
        ("too tall", 10, 100_000, (10, 100_000)),
        ("too many pixels", 10_000, 10_000, (10_000, 10_000)),
        ("a bomb", 100_000, 100_000, (100_000, 100_000)),
    ];
    for (case, width, height, expect) in cases {
        let bytes = claimed_png(width, height);
        let started = std::time::Instant::now();
        let refused = picture(&bytes, Kind::Png, VIEW);
        assert_eq!(
            refused,
            Err(Refusal::TooLarge {
                width: expect.0,
                height: expect.1
            }),
            "{case}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{case}: the refusal took {:?}",
            started.elapsed()
        );
    }
    assert!(decodable(MAX_SIDE, 1));
    assert!(!decodable(MAX_SIDE + 1, 1));
    assert!(!decodable(0, 10));
    assert_eq!(
        Refusal::TooLarge {
            width: 100_000,
            height: 100_000
        }
        .sentence(),
        "Too large to preview (100000 × 100000)"
    );
}

#[test]
fn bytes_that_are_not_what_they_began_as_are_unreadable() {
    let mut bytes = encoded(40, 40, ImageFormat::Png);
    bytes.truncate(40);
    assert_eq!(picture(&bytes, Kind::Png, THUMB), Err(Refusal::Unreadable));
    assert_eq!(
        picture(b"%PDF-1.4", Kind::Pdf, THUMB),
        Err(Refusal::Unreadable),
        "a PDF is not an image"
    );
}

#[cfg(feature = "pdf")]
#[test]
fn a_pdf_is_drawn_a_page_at_a_time_and_says_how_many_it_has() {
    let first = pdf_page(TWO_PAGES.to_vec(), 0, PAGE).unwrap();
    assert_eq!((first.number, first.count), (0, 2));
    // 200 × 100 points fitted into 1200 × 1600: 1200 × 600.
    assert_eq!((first.picture.width, first.picture.height), (1200, 600));

    let second = pdf_page(TWO_PAGES.to_vec(), 1, PAGE).unwrap();
    assert_eq!((second.number, second.count), (1, 2));
    // 200 × 300 into 1200 × 1600: the height binds, 1066 × 1600 (or a pixel either way).
    assert_eq!(second.picture.height, 1600);
    assert!((1066..=1067).contains(&second.picture.width));
    assert_ne!(first.picture.uri, second.picture.uri);

    let past = pdf_page(TWO_PAGES.to_vec(), 9, PAGE).unwrap();
    assert_eq!(past.number, 1, "a page past the end is the last one");
}

#[test]
fn a_damaged_pdf_is_unreadable_and_never_panics() {
    let cases: &[(&str, &[u8])] = &[
        ("a signature and nothing", b"%PDF-1.4\n"),
        ("truncated", &TWO_PAGES[..200]),
        ("noise", b"%PDF-1.4\n\xFF\xFE\x00garbage obj stream endobj"),
    ];
    for (case, bytes) in cases {
        let got = pdf_page(bytes.to_vec(), 0, THUMB);
        assert!(
            matches!(got, Err(Refusal::Unreadable | Refusal::Empty)),
            "{case}: {got:?}"
        );
    }
}
