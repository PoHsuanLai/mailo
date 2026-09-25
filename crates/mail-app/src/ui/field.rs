//! The one text field.
//!
//! Every field is quire's `TextInput`, in a box of mailo's that carries `extra` for its place in
//! the layout (and `onfocus`/`onblur`, so the window knows when typing is going on). A boxed
//! field has quire's box; an inline one is quire's bare face (`FieldFace::Bare`), set in the face
//! and ink of where it sits (the composer's subject in the display face, a folder's name on the
//! frame), which the box gives it; a secret is quire's `TextInputKind::Secret`, which keeps what
//! is typed in the field's own state and never writes a `value` into the markup.

use dioxus::prelude::*;
use ds::{FieldFace, InputVariant, TextInput, TextInputKind};

/// Which field the mockup draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldKind {
    /// 1px `--line`, radius 10, a 3px `--accent-soft` ring while focused.
    Boxed,
    /// No chrome, in the face of what it edits, for a property row or the inside of a menu.
    Inline,
    /// A boxed field that shows dots. It never draws a value: what is typed goes to `on_input`
    /// and stays in the field's own state, so the markup never holds a password. To clear it,
    /// remount it under a new `key`.
    Secret,
}

/// A text field. `extra` is a further class on its box, kept so each place can size it.
#[component]
pub(super) fn Field(
    kind: FieldKind,
    value: String,
    placeholder: String,
    extra: Option<String>,
    on_input: EventHandler<String>,
    on_focus: EventHandler<()>,
    on_blur: EventHandler<()>,
) -> Element {
    let (variant, holds, base) = match kind {
        FieldKind::Boxed => (InputVariant::Boxed, TextInputKind::Text, "field"),
        FieldKind::Inline => (FieldFace::Bare, TextInputKind::Text, "field bare"),
        FieldKind::Secret => (InputVariant::Boxed, TextInputKind::Secret, "field"),
    };
    let class = match extra {
        None => base.to_owned(),
        Some(extra) => format!("{base} {extra}"),
    };
    rsx! {
        span { class: "{class}",
            TextInput {
                variant,
                kind: holds,
                label: placeholder.clone(),
                value,
                placeholder,
                oninput: move |value| on_input.call(value),
                onfocus: move |()| on_focus.call(()),
                onblur: move |()| on_blur.call(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn every_input_on_the_frame_is_a_field() {
        let (store, _dir) = crate::ui::fixtures::seeded();
        let page = crate::ui::fixtures::markup(store);
        let mut rest = page.as_str();
        let mut found = 0usize;
        while let Some(at) = rest.find("<input") {
            rest = &rest[at..];
            let end = rest.find('>').unwrap_or(rest.len());
            let tag = &rest[..end];
            // Every field is quire's `input.ds-input`.
            assert!(tag.contains("ds-input"), "an input is not a field: {tag}");
            found += 1;
            rest = &rest[end..];
        }
        assert!(found > 0, "the frame rendered no input:\n{page}");
    }
}
