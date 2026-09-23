//! How laid-out a body is.
//!
//! A score, not a rule. Every signal is something a real message trips on its
//! own — a receipt has a table, a letter has paragraphs — and a wrong rule
//! would still be a readable message, so nobody would notice. Each positive
//! signal is capped below [`Limits::HEAVY`], so two of them have to agree.
//! Plain text scores zero by construction: there is no markup to weigh.
//!
//! The number is what a corpus gets printed beside. Tests compare numbers and
//! read the verdict. They do not pin the number.

use super::limits::Limits;

/// What the HTML walk counted. Plain text leaves this at [`Signals::plain`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Signals {
    plain: bool,
    pub tables_in_tables: u32,
    pub presentational: u32,
    pub elements: u32,
    pub spacer_images: u32,
    pub table_cells: u32,
    pub max_depth: u32,
    pub text_chars: u32,
    pub links: u32,
    pub long_paragraphs: u32,
}

impl Signals {
    /// `text/plain`. The score is zero whatever else a caller might have set.
    pub(crate) fn plain() -> Self {
        Self {
            plain: true,
            ..Self::default()
        }
    }
}

/// The heaviness score.
pub(crate) fn score(signals: &Signals) -> i32 {
    if signals.plain {
        return 0;
    }
    let nest = if signals.tables_in_tables > 0 { 80 } else { 0 };
    let present = share(signals.presentational, signals.elements, 60);
    let spacer = (signals.spacer_images as i32 * 15).min(45);
    let area = (signals.table_cells as i32).min(40);
    let depth = ((signals.max_depth as i32).saturating_sub(4) * 5).min(40);
    let count = (signals.elements as i32 / 4).min(30);
    let density = text_density(signals);
    let links = (signals.links as i32 * 5).min(20);
    let prose = if signals.long_paragraphs >= 3 { -40 } else { 0 };
    nest + present + spacer + area + depth + count + density + links + prose
}

/// `true` when the score says the body is laid out rather than written.
pub(crate) fn is_heavy(signals: &Signals) -> bool {
    score(signals) >= Limits::HEAVY
}

fn share(part: u32, whole: u32, cap: i32) -> i32 {
    if whole == 0 {
        0
    } else {
        ((part as i32) * cap / whole as i32).min(cap)
    }
}

/// Sparse text among many tags is layout. A short letter is not judged.
fn text_density(signals: &Signals) -> i32 {
    if signals.elements < 25 {
        return 0;
    }
    let per = signals.text_chars / signals.elements.max(1);
    if per >= 40 {
        return 0;
    }
    ((40 - per as i32) * 30 / 40).min(30)
}
