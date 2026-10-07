//! How big the add-account window is for the step it shows: a size per step, computed from the
//! step's content (how many rows the list has, how many fields the form asks, how many parts it
//! is read in), so the sign-in step is not a mostly empty 520x700.
//!
//! The window applies them with quire's `WindowSizer` (`request_size`, in logical pixels), each
//! capped to 85% of the screen (`Extent::fit`), and stops asking once the person has resized the
//! window themselves (`SizeOrigin::Person`). It opens at the size of its first step, the list
//! (`WindowSize::fitting`). The sizes are the window's own estimate of its layout, in logical
//! pixels, measured from the pictures of `ds-shell::accounts`; `Wiring`'s `fit` is the seam a test
//! sees every size asked through.

use ds_blitz::{Extent, WindowSize};

use super::map::Step;

/// The width of a window that draws a list, a short form or a status.
const NARROW: u32 = 480;
/// The width of the server form: a label and an entry side by side, in titled parts.
const WIDE: u32 = 560;

// The heights are measured from the steps as `ds-shell::accounts` draws them (the pictures
// `render_the_typed_forms_and_the_resized_steps` paints): each is what is above the content, the
// content's rows and what is below it, with the room a step keeps around its edges.

/// A list: the title, the search field and the room under them, then the rows and the button.
const LIST: u32 = 157;
/// One provider of the list.
const PROVIDER: u32 = 48;
/// The most rows of the list the window shows before the list scrolls.
const SHOWN: u32 = 8;
/// The short form: the header with its round mark, then the buttons and the room below them.
const SHORT: u32 = 131;
/// One field of the short form: an entry and the space after it.
const FIELD: u32 = 35;
/// The server form: the header, the first part's title, and the buttons under the last part.
const SERVERS: u32 = 207;
/// One row of the server form: a label beside its pop-up or entry.
const ROW: u32 = 48;
/// The title of each part after the first, and the space between two parts.
const PART: u32 = 43;
/// A step that says one thing: a title, a sentence and the buttons.
const SENTENCE: u32 = 135;
/// A browser or code step: the sentence, and the link or code to copy under it.
const COPYABLE: u32 = 165;
/// The review: the title, the account's address and the buttons.
const REVIEW: u32 = 147;
/// One line a review adds for a service a provider lacks.
const SERVICE: u32 = 44;

/// The size of the window while it shows `step`; none while it shows nothing.
pub(super) fn extent(step: Option<&Step>) -> Extent {
    match step {
        None => Extent::new(NARROW, SENTENCE),
        Some(Step::Providers(props)) => {
            // The list's own "Other…" is one more row.
            let rows = (props.providers.len() as u32 + 1).min(SHOWN);
            Extent::new(NARROW, LIST + rows * PROVIDER)
        }
        Some(Step::SignIn(props)) => {
            let fields = props.fields.len() as u32;
            match props.fields.iter().any(|field| field.part.is_some()) {
                true => {
                    let parts = props
                        .fields
                        .iter()
                        .fold((None, 0u32), |(last, count), field| {
                            (field.part, count + u32::from(field.part != last))
                        })
                        .1;
                    servers(fields, parts)
                }
                false => Extent::new(NARROW, SHORT + fields * FIELD),
            }
        }
        Some(Step::Browser { .. } | Step::Code { .. }) => Extent::new(NARROW, COPYABLE),
        Some(Step::Review(props)) => {
            Extent::new(NARROW, REVIEW + props.services.len() as u32 * SERVICE)
        }
        Some(Step::Working { .. } | Step::Failed { .. }) => Extent::new(NARROW, SENTENCE),
    }
}

fn servers(rows: u32, parts: u32) -> Extent {
    Extent::new(WIDE, SERVERS + rows * ROW + parts.saturating_sub(1) * PART)
}

/// The least the window is ever made: the smallest step's size, so a step is never squeezed.
pub(super) fn least() -> Extent {
    Extent::new(NARROW, SENTENCE)
}

/// `wanted` capped to 85% of `screen` when it is known, and never below [`least`].
pub(super) fn fitted(wanted: Extent, screen: Option<Extent>) -> Extent {
    match screen {
        Some(screen) => wanted.fit(screen, Some(least())),
        None => wanted,
    }
}

/// What the window opens at: the list, its first step, fitted to `screen`.
pub(super) fn opening(screen: Option<Extent>) -> WindowSize {
    let list = Extent::new(NARROW, LIST + SHOWN.min(7) * PROVIDER);
    WindowSize::fitting(list, screen, least())
}
