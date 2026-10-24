use std::borrow::Cow;
use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageEncoder, ImageFormat, ImageReader, RgbImage};

use super::{PictureSize, resizable, sized, sizes};

/// A picture of soft gradients, which every encoder makes small, as it does a photo.
fn gradient(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x * 255 / width) as u8, (y * 255 / height) as u8, 128])
    })
}

/// A picture of noise, which no encoder makes small.
fn noise(width: u32, height: u32) -> RgbImage {
    let mut seed = 0x2545_f491_u32;
    RgbImage::from_fn(width, height, |_, _| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let [a, b, c, _] = seed.to_le_bytes();
        image::Rgb([a, b, c])
    })
}

fn jpeg_of(picture: &RgbImage, quality: u8, exif: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
    if let Some(exif) = exif {
        encoder
            .set_exif_metadata(exif)
            .unwrap_or_else(|why| panic!("exif: {why}"));
    }
    encoder
        .write_image(
            picture.as_raw(),
            picture.width(),
            picture.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap_or_else(|why| panic!("a jpeg: {why}"));
    out
}

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    jpeg_of(&gradient(width, height), 90, None)
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(gradient(width, height))
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap_or_else(|why| panic!("a png: {why}"));
    out
}

/// What `bytes` is, told from its content, and its size in pixels.
fn read(bytes: &[u8]) -> (ImageFormat, (u32, u32)) {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .unwrap_or_else(|why| panic!("{why}"));
    let format = reader.format().unwrap_or_else(|| panic!("no format"));
    let dimensions = reader
        .into_dimensions()
        .unwrap_or_else(|why| panic!("{why}"));
    (format, dimensions)
}

/// What a case expects of the bytes sent.
#[derive(Debug)]
enum Sent {
    /// The original, untouched.
    Original,
    /// Made smaller: this format, these pixels.
    Smaller(ImageFormat, (u32, u32)),
}

#[test]
fn a_picture_is_fitted_to_its_longest_side_and_never_made_larger() {
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
    let cases: Vec<(&str, Vec<u8>, PictureSize, Sent)> = vec![
        (
            "a wide photo, small",
            jpeg(1600, 1000),
            PictureSize::Small,
            Sent::Smaller(ImageFormat::Jpeg, (640, 400)),
        ),
        (
            "a wide photo, medium",
            jpeg(1600, 1000),
            PictureSize::Medium,
            Sent::Smaller(ImageFormat::Jpeg, (1280, 800)),
        ),
        (
            "a tall photo is fitted by its height",
            jpeg(1000, 1600),
            PictureSize::Small,
            Sent::Smaller(ImageFormat::Jpeg, (400, 640)),
        ),
        (
            "a photo within large is not made larger",
            jpeg(1600, 1000),
            PictureSize::Large,
            Sent::Original,
        ),
        (
            "a photo within small is not made larger",
            jpeg(600, 400),
            PictureSize::Small,
            Sent::Original,
        ),
        (
            "actual size is the original",
            jpeg(1600, 1000),
            PictureSize::Actual,
            Sent::Original,
        ),
        (
            "a png stays a png",
            png(1000, 800),
            PictureSize::Small,
            Sent::Smaller(ImageFormat::Png, (640, 512)),
        ),
        (
            "a smaller copy that comes out larger keeps the original",
            jpeg_of(&noise(1000, 1000), 1, None),
            PictureSize::Small,
            Sent::Original,
        ),
        (
            "a gif is not touched",
            gif,
            PictureSize::Small,
            Sent::Original,
        ),
        (
            "what is not a picture is not touched",
            b"%PDF-1.4 slides".to_vec(),
            PictureSize::Small,
            Sent::Original,
        ),
        (
            "a picture that does not decode is sent as it is",
            [&[0xFF_u8, 0xD8, 0xFF, 0xE0][..], &[0; 64]].concat(),
            PictureSize::Small,
            Sent::Original,
        ),
    ];
    for (case, bytes, size, expected) in cases {
        let sent = sized(&bytes, size);
        match expected {
            Sent::Original => {
                assert!(matches!(sent, Cow::Borrowed(_)), "{case}: re-encoded");
                assert_eq!(&*sent, &bytes[..], "{case}");
            }
            Sent::Smaller(format, dimensions) => {
                assert!(
                    sent.len() < bytes.len(),
                    "{case}: {} is not under {}",
                    sent.len(),
                    bytes.len()
                );
                assert_eq!(read(&sent), (format, dimensions), "{case}");
            }
        }
    }
}

/// A TIFF header for an EXIF block that turns the picture a quarter clockwise (orientation 6)
/// and points at a GPS block, as a phone's photo does.
fn exif_turned_with_location() -> Vec<u8> {
    let mut tiff = b"II*\0".to_vec();
    tiff.extend(8_u32.to_le_bytes());
    // IFD0: two entries, then no next IFD.
    tiff.extend(2_u16.to_le_bytes());
    // Orientation, SHORT, 1, 6.
    tiff.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]);
    // GPSInfo, LONG, 1, at 38.
    tiff.extend([0x25, 0x88, 4, 0, 1, 0, 0, 0, 38, 0, 0, 0]);
    tiff.extend(0_u32.to_le_bytes());
    // The GPS IFD: GPSVersionID, BYTE, 4, 2.2.0.0.
    tiff.extend(1_u16.to_le_bytes());
    tiff.extend([0, 0, 1, 0, 4, 0, 0, 0, 2, 2, 0, 0]);
    tiff.extend(0_u32.to_le_bytes());
    tiff
}

#[test]
fn a_photo_taken_sideways_arrives_upright_and_without_its_exif() {
    let photo = jpeg_of(&gradient(1600, 800), 90, Some(exif_turned_with_location()));
    let has_exif = |bytes: &[u8]| bytes.windows(6).any(|w| w == b"Exif\0\0");
    assert!(has_exif(&photo), "precondition: the photo carries EXIF");

    let sent = sized(&photo, PictureSize::Medium);
    // Stored 1600 × 800 and turned a quarter: 800 × 1600 upright, fitted to 640 × 1280.
    assert_eq!(read(&sent), (ImageFormat::Jpeg, (640, 1280)));
    assert!(
        !has_exif(&sent),
        "the EXIF, and the location in it, went with the copy"
    );
}

#[test]
fn the_sizes_weighed_are_the_sizes_sent() {
    for (case, bytes) in [
        ("a photo", jpeg(2100, 700)),
        ("a png", png(1000, 800)),
        ("noise", jpeg_of(&noise(1000, 1000), 1, None)),
        ("not a picture", b"plain words".to_vec()),
    ] {
        let weighed = sizes(&bytes);
        for (at, size) in PictureSize::ALL.into_iter().enumerate() {
            assert_eq!(
                weighed[at],
                sized(&bytes, size).len() as u64,
                "{case} at {size:?}"
            );
        }
    }
}

#[test]
fn only_a_jpeg_or_a_png_is_resizable() {
    for (case, bytes, expected) in [
        ("jpeg", jpeg(10, 10), true),
        ("png", png(10, 10), true),
        ("gif", b"GIF89a".to_vec(), false),
        ("pdf", b"%PDF-1.4".to_vec(), false),
        ("empty", Vec::new(), false),
    ] {
        assert_eq!(resizable(&bytes), expected, "{case}");
    }
}
