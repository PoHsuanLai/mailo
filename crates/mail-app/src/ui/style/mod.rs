//! The shell's stylesheet: layout only.
//!
//! Real `.css` files, stitched at compile time and drawn by `ds::prelude::AppStyle`, which puts
//! them in quire's `app` cascade layer (after `ds`, before the person's own `style.css`). What
//! these files hold is where mailo puts things (the window's grid, the reader's column, the
//! composer's page); how anything looks is quire's: every colour, size, radius, weight, shadow
//! and duration is a token, every control a quire component, every animation quire's. quire's
//! lint holds the line (`lint::assert_clean` at `Profile::Strict`, and the markup lint over the
//! rendered window); a rule that needs a value quire has no token for is a request to quire
//! (FINDINGS "quire requests"), never a literal here.
//!
//! The order below is the cascade and is load-bearing. It is one `concat!` and not several
//! named pieces because `concat!` takes literals only: `include_str!` expands to one, a `const`
//! does not.

#[cfg(test)]
mod exceptions;

pub(super) const STYLE: &str = concat!(
    include_str!("shell.css"),
    include_str!("list.css"),
    include_str!("menus.css"),
    include_str!("reader.css"),
    include_str!("invite.css"),
    include_str!("composer.css"),
    include_str!("contacts.css"),
    include_str!("files.css"),
    include_str!("accounts.css"),
    include_str!("settings.css"),
    include_str!("rules.css"),
    include_str!("pgp.css"),
    include_str!("controls.css"),
);

#[cfg(test)]
pub(in crate::ui) mod tests {
    use super::STYLE;
    use ds_lint::{LintConfig, Offence, Profile, Rule, assert_clean, markup};

    /// Everything the window's markup is styled by: quire's stylesheet, then mailo's.
    pub(in crate::ui) fn full_css() -> String {
        format!("{}\n{STYLE}", ds::stylesheet())
    }

    /// The custom properties mailo's own rules declare on an element, each for its own box.
    const PER_ELEMENT: &[&str] = &[];

    fn config() -> LintConfig {
        LintConfig {
            profile: Profile::Strict,
            own_vars: PER_ELEMENT.iter().map(|name| (*name).to_owned()).collect(),
            exceptions: super::exceptions::STYLE,
            ..LintConfig::new(&ds::kits())
        }
    }

    /// The offences the markup lint finds in `html` drawn with mailo's and quire's sheets: raw
    /// controls, raw vectors and literal colours in a `style`, beside unstyled classes.
    pub(in crate::ui) fn markup_offences(html: &str) -> Vec<Offence> {
        markup(
            html,
            &full_css(),
            &LintConfig {
                exceptions: super::exceptions::MARKUP,
                ..LintConfig::new(&ds::kits())
            },
        )
    }

    /// The window's first frame with a conversation open in the reader, and the command menu's
    /// open menus. The Space's menu and its parts are quire's, styled by quire's sheet.
    async fn frame_markup() -> String {
        use crate::ui::app::App;
        use crate::ui::fixtures::work;
        use dioxus::prelude::*;
        let built = work();
        let store = built.store.clone();
        let mut dom = VirtualDom::new(App)
            .with_root_context(built.store)
            .with_root_context(built.dirs);
        dom.rebuild_in_place();
        let mut menus =
            VirtualDom::new(crate::ui::command::pictures::OpenMenus).with_root_context(store);
        menus.rebuild_in_place();
        // The palette floats in the root's overlay, drawn the render after it asks.
        crate::ui::fixtures::drain(&mut menus);
        dioxus_ssr::render(&dom) + &dioxus_ssr::render(&menus)
    }

    /// Coherence rule 1: mailo's own stylesheet at quire's strictest profile, spacing included.
    /// Every exception is named with its reason, and one that no longer suppresses anything
    /// fails here (`assert_clean` reports a stale exception) rather than hiding a later offence.
    #[test]
    fn our_stylesheet_lints_clean() {
        assert_clean(STYLE, &config());
    }

    /// Coherence rule 2: no class nothing styles, no raw control, no raw vector, no literal
    /// colour in a `style`, except those `exceptions::MARKUP` names with their reasons.
    #[tokio::test]
    async fn the_frame_draws_no_raw_markup_and_no_unstyled_class() {
        let page = frame_markup().await;
        let offences = markup_offences(&page);
        assert!(offences.is_empty(), "{offences:#?}");
    }

    /// The failure is the class name: a stylesheet that merely exists would pass a test that
    /// only asked "is there CSS".
    #[test]
    fn a_class_with_no_rule_is_named() {
        let offences = markup_offences("<div class=\"zz-missing\"></div>");
        assert!(
            offences
                .iter()
                .any(|offence| offence.rule == Rule::UnstyledClass
                    && offence.text.contains("zz-missing")),
            "{offences:#?}"
        );
    }

    /// mailo's stylesheet with its comments removed.
    fn uncommented() -> String {
        let mut css = String::new();
        let mut rest = STYLE;
        while let Some(start) = rest.find("/*") {
            css.push_str(&rest[..start]);
            rest = rest[start + 2..]
                .split_once("*/")
                .map_or("", |(_, after)| after);
        }
        css.push_str(rest);
        css
    }

    /// The declarations of the rule whose selector is exactly `selector`, comments removed.
    fn rule_body(selector: &str) -> String {
        uncommented()
            .split('}')
            .filter_map(|rule| rule.split_once('{'))
            .find(|(prelude, _)| prelude.trim() == selector)
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default()
    }

    /// Every declaration in the sheet as (selector, property, value). The sheet has no at-rules
    /// and no nesting, so a rule is the text up to its `}`.
    fn declarations() -> Vec<(String, String, String)> {
        let css = uncommented();
        let mut found = Vec::new();
        for rule in css.split('}') {
            let Some((selector, body)) = rule.split_once('{') else {
                continue;
            };
            for declaration in body.split(';') {
                if let Some((property, value)) = declaration.split_once(':') {
                    found.push((
                        selector.trim().to_owned(),
                        property.trim().to_owned(),
                        value.trim().to_owned(),
                    ));
                }
            }
        }
        found
    }

    /// The values `property` takes in the rule for exactly `selector`.
    fn values_of(selector: &str, property: &str) -> Vec<String> {
        declarations()
            .into_iter()
            .filter(|(s, p, _)| s == selector && p == property)
            .map(|(.., value)| value)
            .collect()
    }

    /// The words of a value, calls kept whole: `var(--s-6) calc(a * b)` is two.
    fn words(value: &str) -> Vec<String> {
        let (mut words, mut word, mut depth) = (Vec::new(), String::new(), 0usize);
        for c in value.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ => {}
            }
            if c.is_whitespace() && depth == 0 {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            } else {
                word.push(c);
            }
        }
        if !word.is_empty() {
            words.push(word);
        }
        words
    }

    /// Whether `word` is a length written as a number of pixels or rems, `0` excepted.
    fn is_literal_length(word: &str) -> bool {
        let number = word.trim_start_matches('-');
        number.starts_with(|c: char| c.is_ascii_digit() || c == '.')
            && (number.ends_with("px") || number.ends_with("rem"))
    }

    /// Spacing rule: margin, padding, gap and offset are the spacing tokens (`--s-*`), `0`, or a
    /// proportion (`em`, `%`); never a pixel count. Typed in a number it is a size nobody chose.
    #[test]
    fn spacing_is_a_token_never_a_number() {
        // (selector, property, why) for the one place that is not a step of the scale.
        const EXCEPT: &[(&str, &str, &str)] = &[(
            ".send-at",
            "left",
            "the sidebar's width, until the pill is placed by its pane",
        )];
        const SPACING: &[&str] = &[
            "margin",
            "margin-top",
            "margin-right",
            "margin-bottom",
            "margin-left",
            "padding",
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
            "padding-inline",
            "gap",
            "row-gap",
            "column-gap",
            "top",
            "right",
            "bottom",
            "left",
            "inset",
        ];
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(selector, property, value)| {
                SPACING.contains(&property.as_str())
                    && !EXCEPT
                        .iter()
                        .any(|(s, p, _)| s == selector && p == property)
                    && words(value).iter().any(|word| is_literal_length(word))
            })
            .collect();
        assert!(off.is_empty(), "spacing written as a number: {off:#?}");
    }

    /// Spacing rule: a line is `--hair`, and any other border width is a spacing token. A bare
    /// `1px` blurs across two device pixels at a fractional scale.
    #[test]
    fn borders_are_hairs_or_steps() {
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, _)| {
                property.starts_with("border") && !property.contains("radius")
            })
            .filter(|(.., value)| words(value).iter().any(|word| is_literal_length(word)))
            .collect();
        assert!(off.is_empty(), "border widths written as numbers: {off:#?}");
    }

    /// Shape rule: a box in the content is `--r-media`, a raised panel `--r-panel`; the only
    /// others are a pill, and the small marks inside text and controls.
    #[test]
    fn radii_are_media_panel_or_a_mark() {
        const SHAPES: &[&str] = &[
            "var(--r-media)",
            "var(--r-panel)",
            "var(--r-pill)",
            "var(--r-tiny)",
            "var(--r-micro)",
        ];
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, value)| {
                property == "border-radius" && !SHAPES.contains(&value.as_str())
            })
            .collect();
        assert!(off.is_empty(), "radii off the rule sheet: {off:#?}");
    }

    /// Shape rule: an in-content card has `--shadow-1`, a sheet `--shadow-sheet`, a popover
    /// `--shadow-pop`; the window and its card have none (it is flush). The one other shadow is
    /// an inset ring, a drop target's outline, drawn in spacing steps.
    #[test]
    fn shadows_are_the_three_and_the_window_has_none() {
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, value)| {
                property == "box-shadow"
                    && ![
                        "var(--shadow-1)",
                        "var(--shadow-sheet)",
                        "var(--shadow-pop)",
                    ]
                    .contains(&value.as_str())
                    && !value.starts_with("inset 0 0 0 var(--s-2) var(--")
            })
            .collect();
        assert!(off.is_empty(), "shadows off the rule sheet: {off:#?}");
        for selector in [".app", ".card"] {
            for property in ["box-shadow", "border-radius", "padding"] {
                assert_eq!(
                    values_of(selector, property),
                    Vec::<String>::new(),
                    "the window is flush: {selector} sets {property}"
                );
            }
        }
    }

    /// Spacing rule: a sheet's form is 16 padding and 12 gap; a popover is padded 8 and is one of
    /// three widths (narrow, medium and wide: a time to pick, a list, a grid of emoji).
    #[test]
    fn sheets_and_popovers_keep_their_measures() {
        let sheets = ".sheet-form, .book, .rules";
        assert_eq!(values_of(sheets, "padding"), ["var(--s-16)"], "{sheets}");
        assert_eq!(values_of(sheets, "gap"), ["var(--s-12)"], "{sheets}");
        assert_eq!(values_of(".doctor", "padding"), ["var(--s-16)"]);
        assert_eq!(values_of(".doctor", "gap"), ["var(--s-12)"]);
        const NARROW: &str = "260px";
        const MEDIUM: &str = "320px";
        const WIDE: &str = "516px";
        for (selector, width) in [
            (".pick-time, .tpl-save", NARROW),
            (".leave-ask", MEDIUM),
            (".downloads", MEDIUM),
            (".em-picker", WIDE),
        ] {
            assert_eq!(values_of(selector, "width"), [width], "{selector}");
        }
        for selector in [".downloads", ".em-picker", ".checklist"] {
            assert_eq!(values_of(selector, "padding"), ["var(--s-8)"], "{selector}");
        }
    }

    /// The left and right of a `padding` or `margin` shorthand.
    fn sides(value: &str) -> (String, String) {
        let w = words(value);
        match w.len() {
            1 => (w[0].clone(), w[0].clone()),
            2 | 3 => (w[1].clone(), w[1].clone()),
            4 => (w[3].clone(), w[1].clone()),
            _ => (String::new(), String::new()),
        }
    }

    /// Spacing rule: the reader and the composer share one content inset, `--s-18`, on both
    /// edges of every band, so a subject, a header line, a body and a bar start at one x; and a
    /// bar (the composer's, the composer's foot, the viewer's) is `--s-10` above and below.
    #[test]
    fn the_reader_and_the_composer_share_one_inset() {
        const INSET: &str = "var(--s-18)";
        // (selector, property): each band's shorthand.
        const BANDS: &[(&str, &str)] = &[
            (".reader-head", "padding"),
            (".c-top", "padding"),
            (".c-head", "padding"),
            (".c-props", "padding"),
            (".c-main", "padding"),
            (".c-foot", "padding"),
            (".viewer-head", "padding"),
            (".banners", "margin"),
        ];
        for (selector, property) in BANDS {
            let value = values_of(selector, property);
            assert_eq!(value.len(), 1, "{selector} {property}");
            let (left, right) = sides(&value[0]);
            assert_eq!(
                (left.as_str(), right.as_str()),
                (INSET, INSET),
                "{selector}"
            );
        }
        for (selector, property) in [
            ("article.frame > :not(.html)", "margin-left"),
            ("article.frame > :not(.html)", "margin-right"),
        ] {
            assert_eq!(values_of(selector, property), [INSET], "{selector}");
        }
        for selector in [".c-top", ".c-foot", ".viewer-head"] {
            let value = values_of(selector, "padding");
            assert_eq!(words(&value[0])[0], "var(--s-10)", "{selector}: a bar");
        }
    }

    /// Colour rule: the stylesheet names no colour. A hex, a colour function or a named colour in
    /// a property that paints is a value nobody can retheme; quire's tokens are the palette.
    #[test]
    fn the_stylesheet_names_no_colour() {
        const PAINTS: &[&str] = &[
            "color",
            "background",
            "background-color",
            "border",
            "border-top",
            "border-right",
            "border-bottom",
            "border-left",
            "border-color",
            "outline",
            "caret-color",
            "box-shadow",
            "text-decoration-color",
            "fill",
            "stroke",
        ];
        const NAMED: &[&str] = &[
            "white", "black", "red", "green", "blue", "gray", "grey", "orange", "yellow",
        ];
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, value)| {
                PAINTS.contains(&property.as_str())
                    && (value.contains('#')
                        || ["rgb", "hsl", "oklch", "color-mix"]
                            .iter()
                            .any(|function| value.contains(function))
                        || words(value)
                            .iter()
                            .any(|word| NAMED.contains(&word.as_str())))
            })
            .collect();
        assert!(off.is_empty(), "colours written as literals: {off:#?}");
    }

    /// Colour rule: the reader and the composer are one ground, `--surface-2`, from the head to
    /// the body and the inline reply under a thread.
    #[test]
    fn the_reader_and_the_composer_are_on_one_ground() {
        for selector in [".reader", ".cpage", ".c-main", ".inline-reply"] {
            assert_eq!(
                values_of(selector, "background"),
                ["var(--surface-2)"],
                "{selector}"
            );
        }
    }

    /// Colour rule: the ink ladder. Text is `--ink` (content), `--ink-soft` (labels) or
    /// `--ink-faint` (metadata); a status says a fact in `--danger` or `--warn`; the frame's own
    /// ink is the sidebar's. And state is quire's: nothing here dims by opacity but to fade a part
    /// in or out (0 and 1), so hover, pressed, disabled and selected are drawn by quire alone.
    #[test]
    fn ink_is_a_ladder_and_state_is_not_an_opacity() {
        const INKS: &[&str] = &[
            "var(--ink)",
            "var(--ink-soft)",
            "var(--ink-faint)",
            "var(--danger)",
            "var(--warn)",
            "var(--f-ink)",
            "var(--accent-text)",
        ];
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, value)| property == "color" && !INKS.contains(&value.as_str()))
            .collect();
        assert!(off.is_empty(), "text in an ink off the ladder: {off:#?}");
        let dimmed: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, value)| {
                property == "opacity" && !["0", "1"].contains(&value.as_str())
            })
            .collect();
        assert!(dimmed.is_empty(), "state drawn by opacity: {dimmed:#?}");
    }

    /// Colour rule: the accent is for the primary action, which is quire's button. In mailo's own
    /// rules it appears only where the person's text takes it: the caret, a selection, a link, a
    /// ticked box, and the ring round a drop target.
    #[test]
    fn the_accent_is_in_seven_places_and_no_more() {
        const TAKEN: &[&str] = &[
            ".c-body",
            ".c-body ul.todo > li.done .box",
            ".c-body .m-a",
            ".c-sel",
            ".c-caret",
            ".cpage[*|data-drop=\"accepts\"], .inline-reply[*|data-drop=\"accepts\"]",
            ".cpage[*|data-drop=\"target\"], .inline-reply[*|data-drop=\"target\"]",
        ];
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(selector, _, value)| {
                value.contains("var(--accent") && !TAKEN.contains(&selector.as_str())
            })
            .collect();
        assert!(off.is_empty(), "the accent where it is not taken: {off:#?}");
    }

    /// The `.rs` files under `src`, as (path, text) pairs, tests and fixtures left out: a file
    /// whose path says test or fixtures, and a file's text from its `#[cfg(test)]` on.
    fn shipped_sources() -> Vec<(std::path::PathBuf, String)> {
        fn walk(dir: &std::path::Path, found: &mut Vec<(std::path::PathBuf, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.to_string_lossy().to_lowercase();
                if path.is_dir() {
                    if !name.contains("fixtures") {
                        walk(&path, found);
                    }
                } else if name.ends_with(".rs")
                    && !name.contains("test")
                    && let Ok(text) = std::fs::read_to_string(&path)
                {
                    let shipped = text.split("#[cfg(test)]").next().unwrap_or("").to_owned();
                    found.push((path, shipped));
                }
            }
        }
        let mut found = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut found,
        );
        found
    }

    /// Whether `line` writes a colour: `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`, or a colour
    /// function, inside a string.
    fn writes_a_colour(line: &str) -> bool {
        if ["rgb(", "rgba(", "hsl(", "hsla("]
            .iter()
            .any(|f| line.contains(f))
        {
            return true;
        }
        let bytes = line.as_bytes();
        let mut at = 0;
        while let Some(found) = line[at..].find('#') {
            let start = at + found + 1;
            let digits = bytes[start..]
                .iter()
                .take_while(|byte| byte.is_ascii_hexdigit())
                .count();
            let after = bytes.get(start + digits).copied();
            if [3, 4, 6, 8].contains(&digits)
                && !after.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return true;
            }
            at = start;
        }
        false
    }

    /// Colour rule: a colour literal is allowed in a frame sheet, in print and in a test, and
    /// there it is a named `const`. Nowhere in the window's code is a colour written in a string
    /// beside the code that uses it.
    #[test]
    fn colour_literals_are_named_consts_in_frame_and_print_sheets() {
        let mut off = Vec::new();
        for (path, text) in shipped_sources() {
            for (number, line) in text.lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") || code.contains("const ") {
                    continue;
                }
                if writes_a_colour(line) {
                    off.push(format!(
                        "{}:{}: {}",
                        path.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            off.is_empty(),
            "colour literals not in a const:\n{}",
            off.join("\n")
        );
    }

    /// The scan finds what it is named for, and passes what it is not.
    #[test]
    fn the_colour_scan_reads_a_hex_and_not_a_fragment() {
        assert!(writes_a_colour(r##"let c = "#1d1d1f";"##));
        assert!(writes_a_colour(r##"let c = "#fff";"##));
        assert!(writes_a_colour(r#"let c = "rgba(0, 0, 0, .5)";"#));
        assert!(!writes_a_colour(r##"let h = "#top";"##));
        assert!(!writes_a_colour(r##"let n = "#53667";"##));
        assert!(!writes_a_colour("#[derive(Debug)]"));
    }

    /// Type rule: sizes are the scale's tokens, and the scale in use is a closed set. A new size
    /// is a step added here on purpose, not a token reached for because it was near. The sizes
    /// mailo's rules once took from beside the scale (`--fs-small`, `--fs-note`, `--fs-eyebrow`,
    /// `--fs-body`, `--fs-caption`) are metadata, and metadata is `--fs-help`.
    #[test]
    fn every_font_size_is_a_step_of_the_scale() {
        const SCALE: &[&str] = &[
            "var(--fs-display)",   // a composer's subject, 26
            "var(--fs-subject)",   // a reader's subject, 20
            "var(--fs-heading)",   // a heading written in a message
            "var(--fs-heading-3)", // a sub-heading written in a message
            "var(--fs-title)",     // a pane's title, 16
            "var(--fs-base)",      // a small heading written in a message
            "var(--fs-reading)",   // message text, 14
            "var(--fs-control)",   // a control's text, 13
            "var(--fs-meta)",      // code, 12.5
            "var(--fs-help)",      // metadata (Footnote)
        ];
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, value)| {
                property == "font-size" && !SCALE.contains(&value.as_str())
            })
            .collect();
        assert!(off.is_empty(), "font sizes off the scale: {off:#?}");
    }

    /// Type rule: message text, read or written, is 14 on 1.65 in Inter in `--ink`; the line
    /// heights in the sheet are that, the reader's subject (1.15) and a written heading (1.25).
    #[test]
    fn message_text_is_set_one_way_and_line_heights_are_three() {
        for selector in [".reader-body", ".c-body"] {
            for (property, want) in [
                ("font-size", "var(--fs-reading)"),
                ("line-height", "1.65"),
                ("font-family", "var(--font-ui)"),
                ("color", "var(--ink)"),
            ] {
                assert_eq!(
                    values_of(selector, property),
                    [want],
                    "{selector} {property}"
                );
            }
        }
        let off: Vec<_> = declarations()
            .into_iter()
            .filter(|(_, property, value)| {
                property == "line-height" && !["1.15", "1.25", "1.65"].contains(&value.as_str())
            })
            .collect();
        assert!(off.is_empty(), "line heights off the sheet: {off:#?}");
    }

    /// Type rule: titles. A pane's title is Title (16/700), a reader's subject 20/700 on 1.15 in
    /// the display face, a composer's subject 26/700.
    #[test]
    fn the_titles_are_set_by_the_rule_sheet() {
        assert_eq!(values_of(".list-head h2", "font-size"), ["var(--fs-title)"]);
        assert_eq!(
            values_of(".reader-subject", "font-size"),
            ["var(--fs-subject)"]
        );
        assert_eq!(values_of(".reader-subject", "line-height"), ["1.15"]);
        assert_eq!(values_of(".c-title", "font-size"), ["var(--fs-display)"]);
        assert_eq!(values_of(".c-title", "font-weight"), ["700"]);
        assert_eq!(
            values_of(".c-title", "font-family"),
            ["var(--font-display)"]
        );
    }

    /// Type rule: code is `--font-code` at `--fs-meta`, wherever it is drawn.
    #[test]
    fn code_is_the_code_face_at_one_size() {
        for (selector, property, value) in declarations() {
            if property == "font-family" && value == "var(--font-code)" {
                assert_eq!(
                    values_of(&selector, "font-size"),
                    ["var(--fs-meta)"],
                    "{selector} draws code"
                );
            }
        }
    }

    /// Type rule: a quote's bar is two steps wide (`--s-2`) in `--ink-soft`, in the reader, the
    /// composer and the quoted text of a reply alike.
    #[test]
    fn a_quote_bar_is_one_bar() {
        for selector in [
            ".reader-body .quote",
            ".c-body blockquote",
            ".o-rq .rq-body",
        ] {
            assert_eq!(
                values_of(selector, "border-left"),
                ["var(--s-2) solid var(--ink-soft)"],
                "{selector}"
            );
        }
    }

    /// The `.card` rule's declarations that matter. The window is quire's `SplitView`, whose last
    /// pane takes what the others leave; without `min-width: 0` on the card a long subject widens
    /// the window past the screen. The card is paper, not frame: it resets the text colour to the
    /// paper's ink, or anything on it would inherit the frame's ink, which reads in one scheme and
    /// vanishes in the other.
    #[test]
    fn the_card_rule_shrinks_and_sets_its_ink() {
        // (what the rule must do, the declaration that does it)
        const CASES: &[(&str, &str)] = &[
            ("shrink", "min-width: 0"),
            (
                "reset the text colour to the paper's ink",
                "color: var(--ink)",
            ),
        ];
        let card = rule_body(".card");
        for (does, declaration) in CASES {
            assert!(
                card.contains(declaration),
                ".card must {does} ({declaration}): {card}"
            );
        }
    }
}
