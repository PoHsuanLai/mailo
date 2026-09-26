//! The printout on paper, for the `native` frontend: [`crate::print`]'s document made into a PDF
//! by quire (`ds_native::pdf`), with no webview anywhere.
//!
//! Blitz lays the document out and quire cuts it into pages. Neither reads CSS fragmentation
//! (`break-before`, `break-inside`, `@page`), so the page rules `mail_mime::print` writes as CSS
//! are dropped; the builder writes the same things in the markers quire reads
//! (`data-break-before`, `data-break-inside`: see `mail_mime::print`), and `@page`'s margins
//! are the [`PageSpec`]'s, which are quire's default (18 mm above and below, 16 mm at the sides:
//! the same as the document asks for). Nothing here edits the markup the builder wrote.
//!
//! What this module adds, through `mail_mime::Options`:
//! - **The faces.** Each family names the CJK faces ahead of the generic one, since otherwise
//!   fontique's fallback picks one (on some systems a thin or a bitmap-era face). Which regional
//!   face leads is the message's: the builder marks a message with the script its own headers or
//!   text say (`data-script`, [`mail_mime::Script`]), and that message's families put that
//!   script's face first ([`Cjk::for_script`]). The document's own lines, and a message that
//!   says nothing, lead with the locale's ([`Paper`]).
//! - **The note** at the top that pictures the printout does not hold are named where they were.
//!
//! A remote image prints only for a message whose images the reader consented to, fetched by
//! the caller through the reader's own fetcher (`native_print`); every other remote image is
//! named, as the document's `<p class="missing">`.

use crate::print::{Pictures, Printed};
use ds_native::{Margins, PageSize, PageSpec};
use mail_mime::{Options, Script};

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

    /// The face that leads for text in `script`. Traditional Chinese is Hong Kong's when the
    /// locale's is (`default`), Taiwan's otherwise: the script alone does not say which.
    pub(in crate::ui) fn for_script(script: Script, default: Cjk) -> Cjk {
        match script {
            Script::TraditionalChinese if default == Cjk::Hk => Cjk::Hk,
            Script::TraditionalChinese => Cjk::Tc,
            Script::SimplifiedChinese => Cjk::Sc,
            Script::Japanese => Cjk::Jp,
            Script::Korean => Cjk::Kr,
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

/// The three families a printout sets, with `cjk`'s face leading the CJK ones: serif for the
/// body, sans for the headers and notes, mono for code.
pub(in crate::ui) struct Families {
    pub serif: String,
    pub sans: String,
    pub mono: String,
}

impl Families {
    pub(in crate::ui) fn led_by(cjk: Cjk) -> Families {
        Families {
            serif: stack(
                &["Georgia", "Times New Roman", "Noto Serif"],
                "Noto Serif CJK",
                cjk,
                "serif",
            ),
            sans: stack(&["Noto Sans"], "Noto Sans CJK", cjk, "sans-serif"),
            mono: stack(&["Noto Sans Mono"], "Noto Sans Mono CJK", cjk, "monospace"),
        }
    }
}

/// Every script a message can be marked with.
const SCRIPTS: [Script; 4] = [
    Script::TraditionalChinese,
    Script::SimplifiedChinese,
    Script::Japanese,
    Script::Korean,
];

/// The rules a printout on paper adds after the document's own: faces named, CJK included. Only
/// `font-family` (and the note's look), so every size and weight the document sets stays.
///
/// The document, and a message that says nothing of its script, lead with `default`, the
/// locale's; a message the builder marked with its script leads with that script's face.
pub(in crate::ui) fn paper_css(default: Cjk) -> String {
    let Families { serif, sans, mono } = Families::led_by(default);
    let mut css = format!(
        "body {{ font-family: {serif}; }}\n\
         .printed, .paper-note, h1.thread, .headers, .headers h2, .note, .attachments \
         {{ font-family: {sans}; }}\n\
         .body pre, .body code {{ font-family: {mono}; }}\n\
         .paper-note {{ font-size: 8pt; color: #555; margin: 0 0 1em; }}\n"
    );
    for script in SCRIPTS {
        let cjk = Cjk::for_script(script, default);
        if cjk == default {
            continue;
        }
        let Families { serif, sans, mono } = Families::led_by(cjk);
        let at = format!("article[data-script=\"{}\"]", script.tag());
        css.push_str(&format!(
            "{at} {{ font-family: {serif}; }}\n\
             {at} .headers, {at} .headers h2, {at} .note, {at} .attachments \
             {{ font-family: {sans}; }}\n\
             {at} .body pre, {at} .body code {{ font-family: {mono}; }}\n"
        ));
    }
    css
}

/// `job`'s printout, made for paper: the builder's document with [`paper_css`]'s faces and the
/// note on pictures, and the consented messages' remote images when `pictures` fetches them.
/// Blocking: it reads the stored mail, and may fetch.
pub(in crate::ui) fn printed<Tz>(
    store: &mail_store::SqliteStore,
    job: super::Job,
    paper: &Paper,
    pictures: Option<&Pictures<'_>>,
    zone: &Tz,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Printed, String>
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let style = paper_css(paper.cjk);
    let options = Options {
        pages: job.pages,
        style: &style,
        missing_note: Some(PICTURES_NOTE),
    };
    crate::print::document_with(store, *job.thread.as_uuid(), zone, now, &options, pictures)
}

/// `printed` as a PDF on `paper`. Blocking, and slow next to a click (the layout, and the first
/// time in a process a scan of the system's fonts): never on the render path (F140).
pub(in crate::ui) fn pdf(printed: &Printed, paper: &Paper) -> Result<Vec<u8>, String> {
    ds_native::pdf(&printed.html, paper.spec).map_err(|error| error.to_string())
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
