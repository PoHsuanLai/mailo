//! A logo's SVG made into pixels, once, where nothing it says can reach anything.
//!
//! The draft asks for SVG Tiny PS (the "Portable/Secure" profile): no script, no animation, no
//! external reference. Two layers keep that true whatever the file says. First the file is read
//! as XML (no DTD, so no entity expansion) and refused when it is not that profile's shape:
//! its root an SVG `svg` with `baseProfile="tiny-ps"`, and no script, foreign object, image,
//! link or animation anywhere, no reference that is not to itself. Then it is drawn with resvg,
//! whose parser runs no script, over an empty font database, with every `<image>` reference
//! refused, so even a file that slipped past the first layer loads nothing. What comes out is a
//! [`SIZE`]-pixel PNG; the SVG itself is never handed to the window.

use super::NoLogo;
use resvg::{tiny_skia, usvg};

/// The side of the square a logo is drawn in, in pixels: an avatar at twice its size.
pub const SIZE: u32 = 96;

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Elements the profile leaves out that could run, move, load or link.
const REFUSED: &[&str] = &[
    "script",
    "foreignObject",
    "image",
    "a",
    "animate",
    "animateColor",
    "animateMotion",
    "animateTransform",
    "set",
    "handler",
    "listener",
    "video",
    "audio",
    "iframe",
];

/// `svg` checked and drawn as a PNG of [`SIZE`] × [`SIZE`], the logo centred in it.
pub fn draw(svg: &[u8]) -> Result<Vec<u8>, NoLogo> {
    tiny_ps(svg)?;
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(svg, &options).map_err(|e| refused(e.to_string()))?;
    let size = tree.size();
    let side = SIZE as f32;
    let scale = (side / size.width()).min(side / size.height());
    if !scale.is_finite() || scale <= 0.0 {
        return Err(refused("it has no size"));
    }
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (side - size.width() * scale) / 2.0,
        (side - size.height() * scale) / 2.0,
    );
    let mut pixmap = tiny_skia::Pixmap::new(SIZE, SIZE).ok_or_else(|| refused("no canvas"))?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| refused(e.to_string()))
}

fn refused(why: impl Into<String>) -> NoLogo {
    NoLogo::Refused(why.into())
}

/// Whether `svg` has SVG Tiny PS's shape, as far as it matters to drawing it safely.
fn tiny_ps(svg: &[u8]) -> Result<(), NoLogo> {
    let text = std::str::from_utf8(svg).map_err(|_| refused("not UTF-8"))?;
    let doc = roxmltree::Document::parse(text).map_err(|e| refused(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NS) {
        return Err(refused("the root is not an SVG svg element"));
    }
    if root.attribute("baseProfile") != Some("tiny-ps") {
        return Err(refused("not baseProfile=\"tiny-ps\""));
    }
    for node in doc.descendants() {
        if node.is_element() {
            let name = node.tag_name().name();
            if REFUSED
                .iter()
                .any(|refused| refused.eq_ignore_ascii_case(name))
            {
                return Err(refused(format!("it has a <{name}>")));
            }
            for attribute in node.attributes() {
                let value = attribute.value();
                if attribute.name() == "href" && !value.starts_with('#') {
                    return Err(refused(format!("it refers to {value:?}")));
                }
                if attribute.name().starts_with("on") {
                    return Err(refused(format!("it has an {} handler", attribute.name())));
                }
                outward(value)?;
            }
        } else if let Some(text) = node.text() {
            outward(text)?;
        }
    }
    Ok(())
}

/// A `url(…)` that is not to an element of the file itself, or a stylesheet import.
fn outward(value: &str) -> Result<(), NoLogo> {
    let lower = value.to_ascii_lowercase();
    if lower.contains("@import") {
        return Err(refused("it imports a stylesheet"));
    }
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("url(") {
        rest = &rest[at + 4..];
        let target = rest.trim_start().trim_start_matches(['"', '\'']);
        if !target.starts_with('#') {
            return Err(refused("it refers outside itself"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" version="1.2" baseProfile="tiny-ps" viewBox="0 0 10 10">"#;

    fn svg(body: &str) -> Vec<u8> {
        format!("{HEAD}{body}</svg>").into_bytes()
    }

    #[test]
    fn a_tiny_ps_logo_is_drawn_to_a_square_png() {
        let png = draw(&svg(
            r##"<title>Brand</title><rect width="10" height="10" fill="#f00"/>"##,
        ))
        .unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let drawn = tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!((drawn.width(), drawn.height()), (SIZE, SIZE));
        let middle = drawn.pixel(SIZE / 2, SIZE / 2).unwrap();
        assert_eq!(
            (middle.red(), middle.alpha()),
            (255, 255),
            "the red square is drawn"
        );
    }

    #[test]
    fn what_could_run_or_load_is_refused() {
        let cases: &[(&str, Vec<u8>)] = &[
            ("script", svg("<script>alert(1)</script>")),
            ("image", svg(r#"<image xlink:href="https://tracker.example/p.png" width="1" height="1"/>"#)),
            ("data image", svg(r#"<image href="data:image/png;base64,AAAA" width="1" height="1"/>"#)),
            ("link", svg(r#"<a href="https://evil.example"><rect width="1" height="1"/></a>"#)),
            ("animation", svg(r#"<rect width="1" height="1"><set attributeName="x" to="5"/></rect>"#)),
            ("foreign object", svg("<foreignObject><p>hi</p></foreignObject>")),
            ("outside use", svg(r#"<use href="https://evil.example/x.svg#a"/>"#)),
            ("handler", svg(r#"<rect width="1" height="1" onclick="x()"/>"#)),
            ("css import", svg(r#"<style>@import url(https://evil.example/a.css);</style>"#)),
            ("css url", svg(r#"<rect width="1" height="1" style="fill:url(https://evil.example/p)"/>"#)),
            (
                "not tiny-ps",
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="1" height="1"/></svg>"#.to_vec(),
            ),
            ("not svg", b"<html><body/></html>".to_vec()),
            (
                "an entity",
                format!(r#"<!DOCTYPE svg [<!ENTITY a "aaaa">]>{HEAD}<title>&a;</title></svg>"#)
                    .into_bytes(),
            ),
        ];
        for (name, svg) in cases {
            assert!(
                matches!(draw(svg), Err(NoLogo::Refused(_))),
                "case: {name}: {:?}",
                draw(svg).map(|png| png.len())
            );
        }
        let inner = svg(
            r##"<defs><linearGradient id="g"><stop offset="0" stop-color="#000"/></linearGradient></defs><rect width="10" height="10" fill="url(#g)"/><use href="#g"/>"##,
        );
        assert!(draw(&inner).is_ok(), "references to itself are fine");
    }
}
