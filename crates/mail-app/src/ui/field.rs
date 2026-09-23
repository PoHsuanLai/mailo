//! The one text field.
//!
//! A boxed variant and an inline one. Property rows and the inside of a menu use the inline
//! variant; everything else is boxed. The class is always `inp`, so a stray `<input>` is visible
//! in the frame dump.

use dioxus::prelude::*;

/// Which field the mockup draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldKind {
    /// 1px `--line`, radius 10, a 3px `--accent-soft` ring while focused.
    Boxed,
    /// No chrome, for a property row or the inside of a menu.
    Inline,
    /// A slider from `min` to `max`, for a number with no need of digits: the Space's grain.
    /// The value it hands back is the slider's position, as text like every other field.
    Range { min: u8, max: u8 },
}

/// A text field. `extra` is a further class, kept so the list's search box stays `input.search`.
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
    let variant = match kind {
        FieldKind::Boxed => "inp",
        FieldKind::Inline => "inp inline",
        FieldKind::Range { .. } => "inp range",
    };
    let class = match extra {
        None => variant.to_owned(),
        Some(extra) => format!("{variant} {extra}"),
    };
    if let FieldKind::Range { min, max } = kind {
        return rsx! {
            input {
                class: "{class}",
                r#type: "range",
                min: "{min}",
                max: "{max}",
                aria_label: "{placeholder}",
                value: "{value}",
                oninput: move |event| on_input.call(event.value()),
                onfocusin: move |_| on_focus.call(()),
                onfocusout: move |_| on_blur.call(()),
            }
        };
    }
    rsx! {
        input {
            class: "{class}",
            r#type: "text",
            autocomplete: "off",
            placeholder: "{placeholder}",
            value: "{value}",
            oninput: move |event| on_input.call(event.value()),
            onfocusin: move |_| on_focus.call(()),
            onfocusout: move |_| on_blur.call(()),
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
            assert!(tag.contains("inp"), "an input is not a field: {tag}");
            found += 1;
            rest = &rest[end..];
        }
        assert!(found > 0, "the frame rendered no input:\n{page}");
    }
}
