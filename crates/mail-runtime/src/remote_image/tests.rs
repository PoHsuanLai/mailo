use super::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

fn image(content_type: Option<&str>, bytes: &[u8]) -> Image {
    Image {
        content_type: content_type.map(str::to_owned),
        bytes: bytes.to_vec(),
    }
}

#[test]
fn a_raster_image_of_the_kind_it_says_is_shown_as_a_data_uri() {
    let cases: [(&str, &[u8], &str); 4] = [
        ("image/png", PNG, "image/png"),
        (
            "image/jpeg",
            &[0xff, 0xd8, 0xff, 0xe0, 0, 0x10],
            "image/jpeg",
        ),
        ("image/gif", b"GIF89a\x01\0\x01\0", "image/gif"),
        ("image/webp", b"RIFF\x04\0\0\0WEBPVP8 ", "image/webp"),
    ];
    for (declared, bytes, kind) in cases {
        let uri = image(Some(declared), bytes).data_uri().expect(declared);
        assert!(uri.starts_with(&format!("data:{kind};base64,")), "{uri}");
    }
}

#[test]
fn the_type_written_is_the_one_the_bytes_are_in_the_allowlists_spelling() {
    // The server's spelling and parameters are not what is written.
    let uri = image(
        Some("IMAGE/JPEG; charset=binary"),
        &[0xff, 0xd8, 0xff, 0xe0],
    )
    .data_uri()
    .unwrap();
    assert!(uri.starts_with("data:image/jpeg;base64,"), "{uri}");
    // A declared type that the bytes are not is written as the bytes' own type.
    let uri = image(Some("image/gif"), PNG).data_uri().unwrap();
    assert!(uri.starts_with("data:image/png;base64,"), "{uri}");
}

#[test]
fn what_is_not_a_raster_image_is_never_shown() {
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>";
    let cases: [(&str, Option<&str>, &[u8]); 5] = [
        ("svg by its declared type", Some("image/svg+xml"), svg),
        ("svg dressed as a png", Some("image/png"), svg),
        ("no declared type", None, PNG),
        ("html", Some("text/html"), b"<html></html>"),
        ("nothing at all", Some("image/png"), b""),
    ];
    for (name, declared, bytes) in cases {
        assert_eq!(image(declared, bytes).data_uri(), None, "{name}");
    }
}

#[test]
fn an_image_past_the_largest_is_not_shown() {
    let mut bytes = PNG.to_vec();
    bytes.resize(MAX_BYTES + 1, 0);
    assert_eq!(image(Some("image/png"), &bytes).data_uri(), None);
    bytes.truncate(MAX_BYTES);
    assert!(image(Some("image/png"), &bytes).data_uri().is_some());
}

#[tokio::test]
async fn an_image_is_fetched_only_from_a_public_https_address() {
    let fetcher = ImageFetcher::new(tokio::runtime::Handle::current()).unwrap();
    for url in [
        "http://127.0.0.1/a.png",
        "ftp://example.com/a.png",
        "https://127.0.0.1/a.png",
        "https://169.254.169.254/latest/meta-data/",
        "https://[::1]/a.png",
        "https://localhost/a.png",
        "https://router.local/a.png",
        "not a url",
    ] {
        assert!(fetcher.get(url).await.is_none(), "{url}");
    }
}

#[test]
fn an_http_image_is_asked_for_over_https() {
    for (url, fetched) in [
        (
            "http://example.com/a.png?w=2",
            Some("https://example.com/a.png?w=2"),
        ),
        (
            "http://example.com:80/a.png",
            Some("https://example.com/a.png"),
        ),
        (
            "http://example.com:8080/a.png",
            Some("https://example.com:8080/a.png"),
        ),
        (
            "https://example.com/a.png",
            Some("https://example.com/a.png"),
        ),
        ("http://192.168.1.1/a.png", None),
        ("data:image/png;base64,AA==", None),
    ] {
        assert_eq!(
            upgraded(url).as_ref().map(reqwest::Url::as_str),
            fetched,
            "{url}"
        );
    }
}
