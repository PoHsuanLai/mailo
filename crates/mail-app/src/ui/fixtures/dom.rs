use super::super::app::App;
use super::super::compose::{ComposerPage, use_desk};
use super::super::reading::Reader;
use super::super::style::STYLE;
use super::store::{ACCOUNT, seeded};
use crate::view::{Shell, Theme};
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// The shell's markup, rendered from the real components against a real database.
///
/// `rebuild_in_place` proved the components *run*; it never looked at what they produced.
/// This does, which is the difference between "no panic" and "there is a list on the page".
pub(in crate::ui) fn markup(store: Arc<SqliteStore>) -> String {
    let mut dom = VirtualDom::new(App).with_root_context(store);
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

/// Wrap rendered markup in a self-contained page and write it to `target/`.
///
/// Two files. `<name>.html` leaves `data-theme` unset, which is the
/// desktop-decides path: the stylesheet's `prefers-color-scheme` guard is reachable only
/// while that attribute is absent. `<name>-dark.html` sets `data-theme` as well. Every
/// colour is a token now, in three theme states, so the dark file has to name the state.
/// `color-scheme` on the root used to be enough, when the stylesheet leaned on `Canvas` and
/// `CanvasText`; it no longer answers to that, and a dark file that does not ask renders light.
pub(in crate::ui) fn dump(name: &str, body: &str) {
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let plain = String::new();
    // `Dark` is the variant that names an attribute. `System` is the one that does not.
    let dark = Theme::Dark
        .attribute()
        .expect("Theme::Dark names a data-theme value");
    let forced = format!(" data-theme=\"{dark}\"{plain}");
    for (suffix, root) in [("", plain.as_str()), ("-dark", forced.as_str())] {
        let page = format!(
            "<!doctype html>\n<html lang=\"en\"{root}><head><meta charset=\"utf-8\">\n\
             <title>mailo</title>\n<style>{STYLE}</style>\n</head>\n\
             <body>{body}</body></html>\n"
        );
        let out = target.join(format!("{name}{suffix}.html"));
        std::fs::write(&out, page).unwrap();
        println!("wrote {}", out.display());
    }
}

/// Renders the reader pane on the first thread in the store.
///
/// `App` owns its own `Shell` signal, so nothing outside it can open a conversation; the
/// reader is reached by rendering it directly, which is also the only way to look at the
/// half of the shell an empty selection never shows.
#[component]
fn ReaderHarness(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    rsx! {
        div { class: "app",
            div { class: "places" }
            div { class: "list" }
            div { class: "reader", Reader { thread, shell } }
        }
    }
}

/// The thread whose subject contains `needle`.
pub(in crate::ui) fn thread_like(store: &SqliteStore, needle: &str) -> ThreadId {
    store
        .threads(
            &Query {
                filter: Filter::All,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 50,
                },
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .items
        .into_iter()
        .find(|t| t.subject.contains(needle))
        .unwrap_or_else(|| panic!("no thread matching {needle:?} in the fixture"))
        .id
}

/// The reader pane's markup for one thread.
pub(in crate::ui) fn reader_markup(store: Arc<SqliteStore>, thread: ThreadId) -> String {
    let mut dom = VirtualDom::new_with_props(ReaderHarness, ReaderHarnessProps { thread })
        .with_root_context(store);
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

/// A keyboard event that can be handed to `VirtualDom::handle_event`.
///
/// Written by hand rather than through `SerializedKeyboardData`, which is behind a feature
/// flag: six trivial methods is a smaller dependency than a feature.
#[derive(Debug, Clone)]
pub(in crate::ui) struct FakeKey(pub(in crate::ui) &'static str);

impl dioxus::html::point_interaction::ModifiersInteraction for FakeKey {
    fn modifiers(&self) -> dioxus::html::input_data::keyboard_types::Modifiers {
        dioxus::html::input_data::keyboard_types::Modifiers::empty()
    }
}

impl dioxus::html::HasKeyboardData for FakeKey {
    fn key(&self) -> dioxus::html::input_data::keyboard_types::Key {
        self.0
            .parse()
            .unwrap_or(dioxus::html::input_data::keyboard_types::Key::Character(
                self.0.to_owned(),
            ))
    }
    fn code(&self) -> dioxus::html::input_data::keyboard_types::Code {
        dioxus::html::input_data::keyboard_types::Code::Unidentified
    }
    fn location(&self) -> dioxus::html::input_data::keyboard_types::Location {
        dioxus::html::input_data::keyboard_types::Location::Standard
    }
    fn is_auto_repeating(&self) -> bool {
        false
    }
    fn is_composing(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What a text box contains after someone has typed in it.
///
/// The search box reads `e.value()` and nothing else, so everything below it is the minimum
/// `HasFormData` asks for. Added because the search box is the one control whose behaviour
/// depends on state loaded from the store — `label:` resolves against an index the component
/// fills on mount — and a test that sets `Shell::search` directly would skip exactly that.
#[derive(Debug, Clone)]
pub(in crate::ui) struct Typed(pub(in crate::ui) String);

impl dioxus::html::HasFileData for Typed {
    /// A search box has no files attached to it.
    fn files(&self) -> Vec<dioxus::html::FileData> {
        Vec::new()
    }
}

impl dioxus::html::HasFormData for Typed {
    fn value(&self) -> String {
        self.0.clone()
    }
    fn valid(&self) -> bool {
        true
    }
    fn values(&self) -> Vec<(String, dioxus::html::FormValue)> {
        Vec::new()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A click with nothing but the primary button. The handlers these tests drive ignore the
/// coordinates; the converter still has to produce a [`dioxus::html::MouseData`].
#[derive(Debug, Clone)]
struct FakeClick;

impl dioxus::html::point_interaction::ModifiersInteraction for FakeClick {
    fn modifiers(&self) -> dioxus::html::input_data::keyboard_types::Modifiers {
        dioxus::html::input_data::keyboard_types::Modifiers::empty()
    }
}

impl dioxus::html::point_interaction::InteractionLocation for FakeClick {
    fn client_coordinates(&self) -> dioxus::html::geometry::ClientPoint {
        dioxus::html::geometry::ClientPoint::new(0.0, 0.0)
    }
    fn screen_coordinates(&self) -> dioxus::html::geometry::ScreenPoint {
        dioxus::html::geometry::ScreenPoint::new(0.0, 0.0)
    }
    fn page_coordinates(&self) -> dioxus::html::geometry::PagePoint {
        dioxus::html::geometry::PagePoint::new(0.0, 0.0)
    }
}

impl dioxus::html::point_interaction::InteractionElementOffset for FakeClick {
    fn element_coordinates(&self) -> dioxus::html::geometry::ElementPoint {
        dioxus::html::geometry::ElementPoint::new(0.0, 0.0)
    }
}

impl dioxus::html::point_interaction::PointerInteraction for FakeClick {
    fn trigger_button(&self) -> Option<dioxus::html::input_data::MouseButton> {
        Some(dioxus::html::input_data::MouseButton::Primary)
    }
    fn held_buttons(&self) -> dioxus::html::input_data::MouseButtonSet {
        dioxus::html::input_data::MouseButtonSet::empty()
    }
}

impl dioxus::html::HasMouseData for FakeClick {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The renderer's job, done by the tests instead.
///
/// `handle_event` hands a listener a `PlatformEventData` and a global converter turns it
/// into the typed data the handler expects. A renderer installs one; a test has no renderer,
/// so without this every dispatched event panics on a failed downcast. Keyboard, form and
/// click are converted, because those are the events these tests send.
struct TestEvents;

impl dioxus::html::HtmlEventConverter for TestEvents {
    fn convert_keyboard_data(&self, event: &PlatformEventData) -> dioxus::html::KeyboardData {
        if let Some(chord) = event.downcast::<super::events::FakeChord>() {
            return dioxus::html::KeyboardData::new(chord.clone());
        }
        dioxus::html::KeyboardData::new(
            event
                .downcast::<FakeKey>()
                .cloned()
                .expect("these tests only dispatch FakeKey or FakeChord"),
        )
    }

    fn convert_animation_data(&self, event: &PlatformEventData) -> dioxus::html::AnimationData {
        dioxus::html::AnimationData::new(
            event
                .downcast::<super::events::FakeAnimation>()
                .cloned()
                .expect("these tests only dispatch FakeAnimation"),
        )
    }
    fn convert_cancel_data(&self, _: &PlatformEventData) -> dioxus::html::CancelData {
        unimplemented!("convert_cancel_data is not what these tests dispatch")
    }
    fn convert_clipboard_data(&self, _: &PlatformEventData) -> dioxus::html::ClipboardData {
        unimplemented!("convert_clipboard_data is not what these tests dispatch")
    }
    fn convert_composition_data(&self, _: &PlatformEventData) -> dioxus::html::CompositionData {
        unimplemented!("convert_composition_data is not what these tests dispatch")
    }
    fn convert_drag_data(&self, _: &PlatformEventData) -> dioxus::html::DragData {
        unimplemented!("convert_drag_data is not what these tests dispatch")
    }
    fn convert_focus_data(&self, _: &PlatformEventData) -> dioxus::html::FocusData {
        unimplemented!("convert_focus_data is not what these tests dispatch")
    }
    fn convert_form_data(&self, event: &PlatformEventData) -> dioxus::html::FormData {
        dioxus::html::FormData::new(
            event
                .downcast::<Typed>()
                .cloned()
                .expect("these tests only dispatch Typed"),
        )
    }
    fn convert_image_data(&self, _: &PlatformEventData) -> dioxus::html::ImageData {
        unimplemented!("convert_image_data is not what these tests dispatch")
    }
    fn convert_media_data(&self, _: &PlatformEventData) -> dioxus::html::MediaData {
        unimplemented!("convert_media_data is not what these tests dispatch")
    }
    fn convert_mounted_data(&self, _: &PlatformEventData) -> dioxus::html::MountedData {
        unimplemented!("convert_mounted_data is not what these tests dispatch")
    }
    fn convert_mouse_data(&self, event: &PlatformEventData) -> dioxus::html::MouseData {
        dioxus::html::MouseData::new(
            event
                .downcast::<FakeClick>()
                .cloned()
                .expect("these tests only dispatch FakeClick"),
        )
    }
    fn convert_pointer_data(&self, event: &PlatformEventData) -> dioxus::html::PointerData {
        dioxus::html::PointerData::new(
            event
                .downcast::<super::events::FakePointer>()
                .cloned()
                .expect("these tests only dispatch FakePointer"),
        )
    }
    fn convert_resize_data(&self, _: &PlatformEventData) -> dioxus::html::ResizeData {
        unimplemented!("convert_resize_data is not what these tests dispatch")
    }
    fn convert_scroll_data(&self, _: &PlatformEventData) -> dioxus::html::ScrollData {
        unimplemented!("convert_scroll_data is not what these tests dispatch")
    }
    fn convert_selection_data(&self, _: &PlatformEventData) -> dioxus::html::SelectionData {
        unimplemented!("convert_selection_data is not what these tests dispatch")
    }
    fn convert_toggle_data(&self, _: &PlatformEventData) -> dioxus::html::ToggleData {
        unimplemented!("convert_toggle_data is not what these tests dispatch")
    }
    fn convert_touch_data(&self, _: &PlatformEventData) -> dioxus::html::TouchData {
        unimplemented!("convert_touch_data is not what these tests dispatch")
    }
    fn convert_transition_data(&self, _: &PlatformEventData) -> dioxus::html::TransitionData {
        unimplemented!("convert_transition_data is not what these tests dispatch")
    }
    fn convert_visible_data(&self, _: &PlatformEventData) -> dioxus::html::VisibleData {
        unimplemented!("convert_visible_data is not what these tests dispatch")
    }
    fn convert_wheel_data(&self, _: &PlatformEventData) -> dioxus::html::WheelData {
        unimplemented!("convert_wheel_data is not what these tests dispatch")
    }
}

/// Install the converter once for the whole test binary.
pub(in crate::ui) fn dispatching() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        dioxus::html::set_event_converter(Box::new(TestEvents));
    });
}

/// Dynamic attributes set during one render, and which element each landed on.
///
/// Static attributes live in the template and never appear here, so a control is found by an
/// attribute the component computes — an `aria-label` built from a value, not a literal.
#[derive(Default)]
pub(in crate::ui) struct Seen {
    attrs: Vec<(String, String, dioxus_core::ElementId)>,
}

impl Seen {
    fn ids(&self, name: &str, value: &str) -> Vec<dioxus_core::ElementId> {
        let mut ids = Vec::new();
        for (got_name, got_value, id) in &self.attrs {
            if got_name == name && got_value == value && !ids.contains(id) {
                ids.push(*id);
            }
        }
        ids
    }

    /// The element whose dynamic `name` attribute equals `value`, when this render set it.
    pub(in crate::ui) fn get(&self, name: &str, value: &str) -> Option<dioxus_core::ElementId> {
        let ids = self.ids(name, value);
        (ids.len() == 1).then(|| ids[0])
    }

    /// Every element whose dynamic `name` attribute equals `value`, in paint order.
    pub(in crate::ui) fn all(&self, name: &str, value: &str) -> Vec<dioxus_core::ElementId> {
        self.ids(name, value)
    }

    /// This render's attributes and then `later`'s: what several renders set between them, for
    /// a list that lands a frame or two after the click that asked for it.
    pub(in crate::ui) fn merge(mut self, later: Seen) -> Seen {
        self.attrs.extend(later.attrs);
        self
    }

    /// The one element whose dynamic `name` attribute equals `value`.
    pub(in crate::ui) fn one(&self, name: &str, value: &str) -> dioxus_core::ElementId {
        let ids = self.ids(name, value);
        assert_eq!(
            ids.len(),
            1,
            "attribute {name}={value:?} on {} elements; saw {:?}",
            ids.len(),
            self.attrs
                .iter()
                .filter(|(got_name, _, _)| got_name == name)
                .collect::<Vec<_>>()
        );
        ids[0]
    }
}

impl dioxus_core::WriteMutations for Seen {
    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &dioxus_core::AttributeValue,
        id: dioxus_core::ElementId,
    ) {
        let rendered = match value {
            dioxus_core::AttributeValue::Text(text) => text.clone(),
            dioxus_core::AttributeValue::Float(n) => n.to_string(),
            dioxus_core::AttributeValue::Int(n) => n.to_string(),
            dioxus_core::AttributeValue::Bool(b) => b.to_string(),
            other => format!("{other:?}"),
        };
        self.attrs.push((name.to_owned(), rendered, id));
    }

    fn append_children(&mut self, _: dioxus_core::ElementId, _: usize) {}
    fn assign_node_id(&mut self, _: &'static [u8], _: dioxus_core::ElementId) {}
    fn create_placeholder(&mut self, _: dioxus_core::ElementId) {}
    fn create_text_node(&mut self, _: &str, _: dioxus_core::ElementId) {}
    fn load_template(&mut self, _: dioxus_core::Template, _: usize, _: dioxus_core::ElementId) {}
    fn replace_node_with(&mut self, _: dioxus_core::ElementId, _: usize) {}
    fn replace_placeholder_with_nodes(&mut self, _: &'static [u8], _: usize) {}
    fn insert_nodes_after(&mut self, _: dioxus_core::ElementId, _: usize) {}
    fn insert_nodes_before(&mut self, _: dioxus_core::ElementId, _: usize) {}
    fn set_node_text(&mut self, _: &str, _: dioxus_core::ElementId) {}
    fn create_event_listener(&mut self, _: &'static str, _: dioxus_core::ElementId) {}
    fn remove_event_listener(&mut self, _: &'static str, _: dioxus_core::ElementId) {}
    fn remove_node(&mut self, _: dioxus_core::ElementId) {}
    fn push_root(&mut self, _: dioxus_core::ElementId) {}
}

/// Render `dom` from scratch, recording the dynamic attributes of that first paint.
pub(in crate::ui) fn rebuild_into(dom: &mut VirtualDom) -> Seen {
    let mut seen = Seen::default();
    dom.rebuild(&mut seen);
    seen
}

fn paint(dom: &mut VirtualDom) -> Seen {
    let mut seen = Seen::default();
    dom.render_immediate(&mut seen);
    seen
}

/// Click `element` and return the attributes the resulting render set.
pub(in crate::ui) fn click(dom: &mut VirtualDom, element: dioxus_core::ElementId) -> Seen {
    #[allow(deprecated)]
    dom.handle_event(
        "click",
        std::rc::Rc::new(PlatformEventData::new(Box::new(FakeClick))),
        element,
        true,
    );
    paint(dom)
}

/// Press a key and return the attributes the resulting render set.
pub(in crate::ui) fn key(dom: &mut VirtualDom, key_name: &'static str) -> Seen {
    #[allow(deprecated)]
    dom.handle_event(
        "keydown",
        std::rc::Rc::new(PlatformEventData::new(Box::new(FakeKey(key_name)))),
        dioxus_core::ElementId(INSIDE_THE_SHELL as usize),
        true,
    );
    paint(dom)
}

/// Press a key on the running component tree.
///
/// `bubbling: true`, so it does not matter which element inside the shell receives it — the
/// handler is on the root and the event climbs to it, exactly as it does in a browser.
pub(in crate::ui) fn press(dom: &mut VirtualDom, key: &'static str, element: u32) {
    #[allow(deprecated)]
    dom.handle_event(
        "keydown",
        std::rc::Rc::new(PlatformEventData::new(Box::new(FakeKey(key)))),
        dioxus_core::ElementId(element as usize),
        true,
    );
    dom.render_immediate(&mut NoOpMutations);
}

/// An element inside the shell, to deliver key presses to.
///
/// The exact id does not matter — `bubbling: true` means the event climbs to the handler on
/// the root, as it does in a browser — but it has to be *inside*: ids 1 and 2 are the
/// stylesheet and the root's own placeholder, and an event dispatched there reaches nothing.
/// Found by dispatching to each id in turn and watching which ones moved the database.
pub(in crate::ui) const INSIDE_THE_SHELL: u32 = 3;

/// Whether the harness should have a composer open on this render.
///
/// Shared through the root context rather than a prop, so the test can flip it *between*
/// renders of the same scope — which is the only way the hook-order mistake shows itself.
#[derive(Clone)]
pub(in crate::ui) struct Toggle(pub(in crate::ui) Arc<std::sync::atomic::AtomicBool>);

/// Renders the composer page, opening or closing it according to [`Toggle`] on every render.
#[component]
fn ComposerHarness() -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    let toggle = use_context::<Toggle>();
    let mut shell = use_signal(Shell::default);
    let revision = use_signal(|| 0u64);
    let today = use_signal(crate::today::Today::default);
    let spaces = use_signal(crate::space::Spaces::default);
    let side = use_signal(|| false);
    use_desk(today, spaces, None, side);

    let want_open = toggle.0.load(std::sync::atomic::Ordering::SeqCst);
    let is_open = shell.read().composing.is_some();
    if want_open && !is_open {
        if let Some(draft) = store
            .drafts(ACCOUNT)
            .ok()
            .and_then(|d| d.into_iter().next())
        {
            shell.write().compose(&draft);
        }
    } else if !want_open && is_open {
        shell.write().close_composer();
    }
    let draft = shell.read().composing.as_ref().map(|c| c.draft);
    rsx! {
        if let Some(draft) = draft {
            ComposerPage { key: "{draft}", draft, shell, revision }
        }
    }
}

pub(in crate::ui) fn harness(open: bool) -> (VirtualDom, Toggle, tempfile::TempDir) {
    let (store, dir) = seeded();
    let toggle = Toggle(Arc::new(std::sync::atomic::AtomicBool::new(open)));
    let dom = VirtualDom::new(ComposerHarness)
        .with_root_context(store)
        .with_root_context(toggle.clone());
    (dom, toggle, dir)
}

#[cfg(test)]
mod tests {
    use super::super::store::{empty, realistic, seeded};
    use super::{dump, markup, reader_markup, thread_like};

    #[tokio::test]
    #[ignore = "writes target/first-run.html for a human or a headless browser to look at"]
    async fn render_the_first_run_to_a_file() {
        let (store, _dir) = empty();
        dump("first-run", &markup(store));
    }

    #[tokio::test]
    #[ignore = "writes target/shell.html for a human or a headless browser to look at"]
    async fn render_the_shell_to_a_file() {
        let (store, _dir) = seeded();
        dump("shell", &markup(store));
    }

    #[tokio::test]
    #[ignore = "writes target/reader.html for a human or a headless browser to look at"]
    async fn render_the_reader_to_a_file() {
        let (store, _dir) = realistic();
        let thread = thread_like(&store, "rust-lang");
        dump("reader", &reader_markup(store, thread));
    }

    /// The same, with a mailbox shaped like a real one. See [`render_the_shell_to_a_file`].
    #[tokio::test]
    #[ignore = "writes target/shell-real.html for a human or a headless browser to look at"]
    async fn render_the_shell_with_real_mail() {
        let (store, _dir) = realistic();
        dump("shell-real", &markup(store));
    }

    /// The sidebar with a cached icon on each provider tile.
    ///
    /// Uses the PNGs `fetch_the_provider_icons` wrote, when that probe has been run.
    /// Without them the tiles draw letters, which is the same fallback the window uses.
    #[tokio::test]
    #[ignore = "writes target/shell-icons.html from target/provider-icons, when the real fetch has run"]
    async fn render_the_shell_with_provider_icons() {
        let icons =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/provider-icons");
        let (store, _dir) = empty();
        let rows = [
            ("ada@gmail.com", "imap.gmail.com"),
            ("ada@outlook.com", "imap.outlook.com"),
            ("ada@fastmail.com", "imap.fastmail.com"),
            ("ada@icloud.com", "imap.mail.me.com"),
            ("ada@yahoo.com", "imap.mail.yahoo.com"),
        ];
        for (n, (address, host)) in rows.into_iter().enumerate() {
            let plan = mail_domain::AccountPlan {
                address: address.to_owned(),
                incoming: mail_domain::Incoming::Imap {
                    host: host.to_owned(),
                    port: 993,
                    tls: mail_domain::Tls::Implicit,
                },
                outgoing: mail_domain::Outgoing::Smtp {
                    host: host.to_owned(),
                    port: 465,
                    tls: mail_domain::Tls::Implicit,
                },
                auth: mail_domain::AuthPlan::Password {
                    username: mail_domain::Username::SameAsAddress,
                    sasl: vec![mail_domain::SaslMech::Plain],
                },
                identities: Vec::new(),
            };
            store
                .connection()
                .execute(
                    "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        mail_domain::AccountId::generate().to_string(),
                        address,
                        serde_json::to_string(&plan).unwrap(),
                        format!("2026-01-0{}T00:00:00Z", n + 1),
                    ],
                )
                .unwrap();
        }
        let loaded = crate::provider::icon::Loaded::read(&icons);
        if !icons.join("google.png").is_file() {
            println!(
                "no cached icons in {}; tiles will show letters",
                icons.display()
            );
        }
        // One Space over every account. The first-run layout is one Space per
        // account, which would hide four of the five tiles.
        let spaces = crate::space::Spaces {
            current: 0,
            recall: std::collections::BTreeMap::new(),
            spaces: vec![crate::space::Space {
                name: "Mail".to_owned(),
                scope: crate::space::Scope::All,
                ..crate::space::Space::default()
            }],
        };
        let mut dom = dioxus::prelude::VirtualDom::new(crate::ui::app::App)
            .with_root_context(store)
            .with_root_context(loaded)
            .with_root_context(spaces)
            .with_root_context(crate::view::Appearance {
                marks: crate::view::Marks::Icons,
                ..crate::view::Appearance::default()
            });
        dom.rebuild_in_place();
        dump("shell-icons", &dioxus_ssr::render(&dom));
    }
}
