//! The printout on paper, for the `native` frontend: [`crate::print`]'s document made into a PDF
//! by quire (`ds_native::pdf`), with no webview anywhere.
//!
//! Blitz lays the document out and quire cuts it into pages. Neither reads CSS fragmentation
//! (`break-before`, `break-inside`, `@page`), so the page rules `mail_mime::print` writes as CSS
//! would be dropped. [`for_paper`] says the same things in the markers quire reads instead:
//! - a message that starts a page (`Pages::PerMessage`) carries `data-break-before="page"`;
//! - a message's headers, and its list of attachments, carry `data-break-inside="avoid"`, so
//!   neither is cut in two by a page's end;
//! - `@page`'s margins are the [`PageSpec`]'s, which are quire's default (18 mm above and below,
//!   16 mm at the sides: the same as the document asks for).
//!
//! It also names the CJK faces ahead of the generic family, since otherwise fontique's fallback
//! picks one (on some systems a thin or a bitmap-era face), and says once, at the top, that
//! pictures the printout does not hold are named where they were. A printout never fetches: a
//! remote image is left out whether or not the reader was allowed to show it, as it always has
//! been (`mail_mime::print`), and the document's `<p class="missing">` names it.
//!
//! **Why editing the markup is safe.** Every piece of text `mail_mime::print` writes is escaped,
//! `<`, `>` and `"` included, so the tags matched here can only be the ones the builder wrote
//! itself; a subject or a body that spells them out arrives as `&lt;article ...` and is not
//! touched. The markup it keeps is the builder's, the CSP line included.

use crate::print::Printed;
use ds_native::{Margins, PageSize, PageSpec};

/// A message that starts a page, as `mail_mime::print` opens it.
const NEW_PAGE: &str = "<article class=\"message new-page\">";
/// A message's headers.
const HEADERS: &str = "<header class=\"headers\">";
/// A message's list of attachments.
const ATTACHMENTS: &str = "<section class=\"attachments\">";
/// An image the printout names rather than draws.
const MISSING: &str = "<p class=\"missing\">";
/// The "Printed ..." line at the top of the document.
const PRINTED_LINE: &str = "<p class=\"printed\">";

/// What the top of a printout says when it names a picture instead of drawing it.
pub(in crate::ui) const PICTURES_NOTE: &str =
    "Pictures that are not part of the message are not printed; each is named where it appears.";

/// Which regional CJK face leads: the ideographs are shared, their shapes are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Cjk {
    /// Traditional Chinese, as in Taiwan: the default when the locale does not say.
    Tc,
    /// Traditional Chinese, as in Hong Kong and Macau.
    Hk,
    /// Simplified Chinese.
    Sc,
    Jp,
    Kr,
}

impl Cjk {
    /// The suffix Noto's CJK families carry: `Noto Sans CJK TC`.
    fn suffix(self) -> &'static str {
        match self {
            Cjk::Tc => "TC",
            Cjk::Hk => "HK",
            Cjk::Sc => "SC",
            Cjk::Jp => "JP",
            Cjk::Kr => "KR",
        }
    }

    /// The five, `self` first: the first one installed wins.
    fn order(self) -> [Cjk; 5] {
        let mut all = [Cjk::Tc, Cjk::Hk, Cjk::Sc, Cjk::Jp, Cjk::Kr];
        if let Some(at) = all.iter().position(|c| *c == self) {
            all[..=at].rotate_right(1);
        }
        all
    }
}

/// How a printout is put on paper: the sheet, and the CJK face that leads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::ui) struct Paper {
    pub spec: PageSpec,
    pub cjk: Cjk,
}

impl Paper {
    /// A4, and Traditional Chinese first: what a printout gets when the locale says nothing.
    /// The margins are quire's default, 18 mm above and below and 16 mm at the sides: what the
    /// document's own `@page` asks for, which Blitz does not read.
    pub(in crate::ui) fn plain() -> Paper {
        Paper {
            spec: PageSpec {
                size: PageSize::A4,
                margins: Margins::default(),
            },
            cjk: Cjk::Tc,
        }
    }

    /// The paper this process's locale asks for.
    pub(in crate::ui) fn from_env() -> Paper {
        Paper::from_locale(|name| std::env::var(name).ok())
    }

    /// The paper the POSIX locale variables `var` reads ask for: the sheet from `LC_PAPER`, the
    /// CJK face from `LC_CTYPE`, each overridden by `LC_ALL` and defaulting to `LANG`.
    pub(in crate::ui) fn from_locale(var: impl Fn(&str) -> Option<String>) -> Paper {
        let plain = Paper::plain();
        let size = match locale(&var, "LC_PAPER").as_deref().map(territory) {
            Some(Some(place)) if LETTER.contains(&place) => PageSize::Letter,
            _ => PageSize::A4,
        };
        let cjk = locale(&var, "LC_CTYPE")
            .as_deref()
            .and_then(cjk_of)
            .unwrap_or(plain.cjk);
        Paper {
            spec: PageSpec { size, ..plain.spec },
            cjk,
        }
    }
}

/// Where US Letter is the paper in the shops: glibc's `LC_PAPER` letter territories.
const LETTER: [&str; 14] = [
    "US", "CA", "MX", "PH", "CL", "CO", "VE", "CR", "PR", "GT", "SV", "NI", "PA", "DO",
];

/// The locale `category` is in: `LC_ALL`, else the category's own variable, else `LANG`. The
/// portable `C` and `POSIX` say nothing about paper or script.
fn locale(var: &impl Fn(&str) -> Option<String>, category: &str) -> Option<String> {
    ["LC_ALL", category, "LANG"]
        .into_iter()
        .filter_map(var)
        .find(|value| !value.trim().is_empty())
        .filter(|value| !matches!(value.as_str(), "C" | "POSIX") && !value.starts_with("C."))
}

/// `en_US.UTF-8@euro` → `US`.
fn territory(locale: &str) -> Option<&str> {
    let (_, rest) = locale.split_once('_')?;
    let end = rest.find(['.', '@']).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// The CJK face a locale reads with, when it is a CJK one.
fn cjk_of(locale: &str) -> Option<Cjk> {
    let language = locale.split(['_', '.', '@']).next().unwrap_or("");
    match (language, territory(locale)) {
        ("zh", Some("CN" | "SG")) => Some(Cjk::Sc),
        ("zh", Some("HK" | "MO")) => Some(Cjk::Hk),
        ("zh", _) => Some(Cjk::Tc),
        ("ja", _) => Some(Cjk::Jp),
        ("ko", _) => Some(Cjk::Kr),
        _ => None,
    }
}

/// The families the printout names: Latin first (so Latin text keeps its face), then the CJK
/// faces in `cjk`'s order, then the generic family.
fn stack(latin: &[&str], cjk_family: &str, cjk: Cjk, generic: &str) -> String {
    let mut families: Vec<String> = latin.iter().map(|family| format!("'{family}'")).collect();
    families.extend(
        cjk.order()
            .iter()
            .map(|region| format!("'{cjk_family} {}'", region.suffix())),
    );
    families.push(generic.to_owned());
    families.join(", ")
}

/// The rules [`for_paper`] adds after the document's own: faces named, CJK included. Only
/// `font-family`, so every size and weight the document sets stays.
fn paper_css(cjk: Cjk) -> String {
    let serif = stack(
        &["Georgia", "Times New Roman", "Noto Serif"],
        "Noto Serif CJK",
        cjk,
        "serif",
    );
    let sans = stack(&["Noto Sans"], "Noto Sans CJK", cjk, "sans-serif");
    let mono = stack(&["Noto Sans Mono"], "Noto Sans Mono CJK", cjk, "monospace");
    format!(
        "body {{ font-family: {serif}; }}\n\
         .printed, .paper-note, h1.thread, .headers, .headers h2, .note, .attachments \
         {{ font-family: {sans}; }}\n\
         .body pre, .body code {{ font-family: {mono}; }}\n\
         .paper-note {{ font-size: 8pt; color: #555; margin: 0 0 1em; }}\n"
    )
}

/// `mail_mime::print`'s document, ready for quire's PDF path: the page markers, the named faces,
/// and the note on pictures. See the module notes for what each is and why editing is safe.
pub(in crate::ui) fn for_paper(html: &str, cjk: Cjk) -> String {
    let mut out = html
        .replace(
            NEW_PAGE,
            "<article class=\"message new-page\" data-break-before=\"page\">",
        )
        .replace(
            HEADERS,
            "<header class=\"headers\" data-break-inside=\"avoid\">",
        )
        .replace(
            ATTACHMENTS,
            "<section class=\"attachments\" data-break-inside=\"avoid\">",
        );
    if let Some(at) = out.find("</head>") {
        out.insert_str(at, &format!("<style>\n{}</style>\n", paper_css(cjk)));
    }
    if out.contains(MISSING)
        && let Some(line) = out.find(PRINTED_LINE)
        && let Some(end) = out[line..].find("</p>")
    {
        let after = line + end + "</p>".len();
        out.insert_str(
            after,
            &format!("\n<p class=\"paper-note\">{PICTURES_NOTE}</p>"),
        );
    }
    out
}

/// `printed` as a PDF on `paper`. Blocking, and slow next to a click (the layout, and the first
/// time in a process a scan of the system's fonts): never on the render path (F140).
pub(in crate::ui) fn pdf(printed: &Printed, paper: &Paper) -> Result<Vec<u8>, String> {
    ds_native::pdf(&for_paper(&printed.html, paper.cjk), paper.spec)
        .map_err(|error| error.to_string())
}

/// What a print dialog, and a PDF file, are titled: the subject, or "(no subject)".
pub(in crate::ui) fn title(printed: &Printed) -> String {
    let subject = printed.subject.trim();
    if subject.is_empty() {
        "(no subject)".to_owned()
    } else {
        subject.to_owned()
    }
}
