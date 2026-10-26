//! How big the add-account window is: as tall as the step it shows, measured after layout by
//! quire's `use_content_fit` (capped to 85% of the screen, left alone once the person has resized
//! the window), at the width this module names for the step.
//!
//! Only the window's first size is an estimate: it opens at the list, its first step, before
//! anything is laid out (`WindowSize::fitting`), and the measured height follows the first
//! layout.

use ds_blitz::{Extent, WindowSize};

use super::map::Step;

/// The width of a window that draws a list, a short form or a status.
const NARROW: u32 = 480;
/// The width of the server form: a label and an entry side by side, in titled parts.
const WIDE: u32 = 560;

/// The list as `ds-shell::accounts` draws it: the title, the search field and the room under
/// them, then the rows and the button. What the window opens at.
const LIST: u32 = 157;
/// One provider of the list.
const PROVIDER: u32 = 48;
/// The rows of the list the window opens showing.
const OPENING_ROWS: u32 = 7;
/// The shortest a step is: a title, a sentence and the buttons.
const SENTENCE: u32 = 135;

/// How wide the window is while it shows `step`: the server form, read in titled parts, is
/// wider than every other step.
pub(super) fn width(step: Option<&Step>) -> u32 {
    match step {
        Some(Step::SignIn(props)) if props.fields.iter().any(|field| field.part.is_some()) => WIDE,
        _ => NARROW,
    }
}

/// The least the window is ever made: the smallest step's size, so a step is never squeezed.
pub(super) fn least() -> Extent {
    Extent::new(NARROW, SENTENCE)
}

/// What the window opens at: the list, its first step, fitted to `screen`.
pub(super) fn opening(screen: Option<Extent>) -> WindowSize {
    let list = Extent::new(NARROW, LIST + OPENING_ROWS * PROVIDER);
    WindowSize::fitting(list, screen, least())
}
