//! The one text field.
//!
//! A boxed variant and an inline one. Property rows and the inside of a menu use the inline
//! variant; everything else is boxed. A boxed field is quire's `TextInput`, in a box of mailo's
//! that carries `extra` for its place in the layout (and `onfocus`/`onblur`, so the window knows
//! when typing is going on). An inline field and a secret stay mailo's `input.inp`: an inline
//! field is set in the face and ink of where it sits (the composer's subject in the display face,
//! a folder's name on the frame), which `TextInput` takes from nothing but its own sheet, and a
//! secret never writes its value into the markup, where `TextInput`'s password does.

use dioxus::prelude::*;

/// Which field the mockup draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldKind {
    /// 1px `--line`, radius 10, a 3px `--accent-soft` ring while focused.
    Boxed,
    /// No chrome, for a property row or the inside of a menu.
    Inline,
    /// A boxed field that shows dots. It never draws a value: what is typed goes to `on_input`
    /// and stays in the webview's own field, so the markup never holds a password.
    Secret,
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
    if kind == FieldKind::Boxed {
        let class = match extra {
            None => "field".to_owned(),
            Some(extra) => format!("field {extra}"),
        };
        return rsx! {
            span { class: "{class}",
                ds::TextInput {
                    variant: ds::InputVariant::Boxed,
                    label: placeholder.clone(),
                    value,
                    placeholder,
                    oninput: move |value| on_input.call(value),
                    onfocus: move |()| on_focus.call(()),
                    onblur: move |()| on_blur.call(()),
                }
            }
        };
    }
    let variant = match kind {
        FieldKind::Boxed | FieldKind::Inline => "inp inline",
        FieldKind::Secret => "inp secret",
    };
    let class = match extra {
        None => variant.to_owned(),
        Some(extra) => format!("{variant} {extra}"),
    };
    if kind == FieldKind::Secret {
        return rsx! {
            input {
                class: "{class}",
                r#type: "password",
                autocomplete: "off",
                spellcheck: "false",
                placeholder: "{placeholder}",
                aria_label: "{placeholder}",
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
            // mailo's `input.inp`, or quire's `input.ds-input` a boxed field draws.
            assert!(tag.contains("inp"), "an input is not a field: {tag}");
            found += 1;
            rest = &rest[end..];
        }
        assert!(found > 0, "the frame rendered no input:\n{page}");
    }
}
