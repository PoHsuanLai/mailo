//! The shell's stylesheet.
//!
//! Real `.css` files, stitched at compile time. Splitting by concern rather than by line
//! count is what CONVENTIONS.md section 8 asks for: `list.css` is one concept in a way that
//! "lines 25 to 34 of the stylesheet" never is.
//!
//! The order below is the cascade and is load-bearing. Keep it.
//!
//! It is one `concat!` and not several named pieces because `concat!` takes literals only:
//! `include_str!` expands to one, a `const` does not, and a nested `const` fails to compile
//! with "only literals can be passed to `concat!()`".
//!
//! What the files are not: quire's. The reset, every token (colour, type, radius, spacing,
//! motion, in both schemes and every motion level), the faces and the frame's layers are
//! `ds::stylesheet()` and `ds::font_face_css()`, which the window's `Ds` root and the head carry.
//! These files hold mailo's own layout and the components not yet moved to quire, written
//! against quire's tokens: the scheme is the root's `data-theme`, never a media query, and a
//! Space that lends the card its hue writes `--accent` on the root, never here.
//!
//! Every keyframe is quire's too (coherence rule 1): the rules here only name them, and the
//! motion that has to end is timed by quire's clock, never by the webview's `animationend`
//! (rule 4). The few keyframes quire does not have yet are declared beside the one rule that
//! plays each, and named as exceptions in the lint below.

#[cfg(test)]
mod calm;
#[cfg(test)]
mod exceptions;

pub(super) const STYLE: &str = concat!(
    include_str!("shell.css"),
    include_str!("editor.css"),
    include_str!("list.css"),
    include_str!("menus.css"),
    include_str!("reader.css"),
    include_str!("invite.css"),
    include_str!("hover.css"),
    include_str!("composer.css"),
    include_str!("contacts.css"),
    include_str!("files.css"),
    include_str!("accounts.css"),
    include_str!("rules.css"),
    include_str!("pgp.css"),
    include_str!("controls.css"),
);

#[cfg(test)]
pub(in crate::ui) mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::STYLE;
    use ds::lint::{LintConfig, Offence, Profile, Rule, assert_clean, markup, stylesheet};
    use ds::ratio;

    /// Everything the window's markup is styled by: quire's stylesheet, then mailo's.
    pub(in crate::ui) fn full_css() -> String {
        format!("{}\n{STYLE}", ds::stylesheet())
    }

    /// Text on every ground it is drawn on. The Post reference puts faint text on the recessed
    /// panes too (the sidebar's counts, the reader's meta), so `--surface-2` is checked, not
    /// only the paper. `--warn` is a star's fill and a retry's mark, a graphic, so 3.0.
    const PAIRS: &[(&str, &str, f64)] = &[
        ("--ink", "--paper", 4.5),
        ("--ink", "--surface", 4.5),
        ("--ink", "--surface-2", 4.5),
        ("--ink", "--raise", 4.5),
        ("--ink-soft", "--paper", 4.5),
        ("--ink-soft", "--surface", 4.5),
        ("--ink-soft", "--surface-2", 4.5),
        ("--ink-faint", "--paper", 3.0),
        ("--ink-faint", "--surface", 3.0),
        ("--ink-faint", "--surface-2", 3.0),
        ("--danger", "--paper", 4.5),
        ("--danger-ink", "--danger", 4.5),
        ("--accent-ink", "--accent", 4.5),
        ("--accent", "--paper", 3.0),
        ("--accent", "--surface", 3.0),
        ("--ink", "--accent-soft", 4.5),
        ("--warn", "--surface", 3.0),
        ("--ok", "--surface", 3.0),
    ];

    /// The palette mailo draws with, which is quire's now: light, then dark. Each state is
    /// the `.ds` root's blocks laid over one another in cascade order, with the Postmark accent
    /// the window's root resolves to (`appearance.toml`'s default, and what the import of
    /// `appearance.json` gives every migrated user). quire always writes `data-theme`, so
    /// there is no desktop-decides state left to check.
    const THEMES: &[(&str, &[&str])] = &[
        ("light", &[".ds", ".ds[*|data-accent=postmark]"]),
        (
            "dark",
            &[
                ".ds",
                ".ds[*|data-accent=postmark]",
                ".ds[*|data-theme=dark]",
                ".ds[*|data-theme=dark][*|data-accent=postmark]",
            ],
        ),
    ];

    /// Every declaration of every rule whose selector is exactly `selector`, merged in source
    /// order so a later rule wins. Exact comparison, never `contains`: `:root[data-theme="dark"]`
    /// is a prefix of `:root[data-theme="dark"][data-accent="oxblood"]`.
    fn declared(css: &str, selector: &str) -> BTreeMap<String, String> {
        let css = strip_comments(css);
        let mut out = BTreeMap::new();
        declarations_in(&css, selector, &mut out);
        out
    }

    fn declarations_in(css: &str, selector: &str, out: &mut BTreeMap<String, String>) {
        let bytes = css.as_bytes();
        let mut index = 0;
        let mut boundary = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'{' => {
                    let sel = css[boundary..index].trim();
                    let Some(close) = matching_close(css, index) else {
                        return;
                    };
                    // An at-rule is a wrapper. Descend so the rules inside it are visible;
                    // do not record the at-rule itself.
                    if sel.starts_with('@') {
                        declarations_in(&css[index + 1..close], selector, out);
                    } else if sel == selector {
                        record(&css[index + 1..close], out);
                    }
                    index = close + 1;
                    boundary = index;
                }
                b';' | b'}' => {
                    boundary = index + 1;
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    fn matching_close(css: &str, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (index, byte) in css.as_bytes().iter().enumerate().skip(open) {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn record(body: &str, out: &mut BTreeMap<String, String>) {
        for decl in body.split(';') {
            let Some((name, value)) = decl.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if kept(name) {
                out.insert(name.to_string(), value.trim().to_string());
            }
        }
    }

    fn kept(name: &str) -> bool {
        name.starts_with("--")
            || name == "grid-template-columns"
            || name == "position"
            || name == "grid-column"
    }

    fn strip_comments(css: &str) -> String {
        let mut out = String::with_capacity(css.len());
        let mut chars = css.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                while let Some(d) = chars.next() {
                    if d == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
                out.push(' ');
            } else {
                out.push(c);
            }
        }
        out
    }

    /// `Some("--x")` when the whole value is exactly `var(--x)`.
    fn exactly_var(value: &str) -> Option<&str> {
        let inner = value.trim().strip_prefix("var(")?.strip_suffix(')')?.trim();
        if inner.starts_with("--") && !inner.contains(|c: char| c == ',' || c.is_whitespace()) {
            Some(inner)
        } else {
            None
        }
    }

    /// The literal hex a token resolves to, following one `var(--x)` and no further.
    fn literal(tokens: &BTreeMap<String, String>, name: &str) -> Result<String, String> {
        let Some(raw) = tokens.get(name) else {
            return Err(format!("{name} is missing"));
        };
        let resolved = if let Some(target) = exactly_var(raw) {
            let Some(value) = tokens.get(target) else {
                return Err(format!("{name} is var({target}) but {target} is missing"));
            };
            value
        } else {
            raw
        };
        if ratio(resolved, "#000000").is_none() {
            return Err(format!("{name} is not a hex colour ({resolved})"));
        }
        Ok(resolved.clone())
    }

    /// The palette in one theme state: each block in `layers`, later over earlier.
    fn resolved(layers: &[&str]) -> BTreeMap<String, String> {
        let css = ds::stylesheet();
        let mut tokens = BTreeMap::new();
        for selector in layers {
            tokens.extend(declared(css, selector));
        }
        tokens
    }

    #[test]
    fn a_dark_token_also_has_a_light_value() {
        let light = declared(ds::stylesheet(), ".ds");
        let dark = declared(ds::stylesheet(), ".ds[*|data-theme=dark]");
        let dark_tokens: Vec<&str> = dark
            .keys()
            .filter(|name| name.starts_with("--"))
            .map(String::as_str)
            .collect();
        assert!(
            !dark_tokens.is_empty(),
            "no custom properties on .ds[*|data-theme=dark]",
        );
        let missing: Vec<&str> = dark_tokens
            .into_iter()
            .filter(|name| !light.contains_key(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "dark palette tokens with no light value: {}",
            missing.join(", "),
        );
    }

    #[test]
    fn the_card_the_readout_measures_is_the_stylesheets() {
        // `ds::readout`, which the Space editor shows, measures against constants because it
        // cannot read CSS. These are the constants, checked against the stylesheet the window
        // is drawn with in every theme state, so the readout and the window cannot drift apart.
        use ds::{Card, POST_DARK, POST_LIGHT};
        let mut failures = Vec::new();
        for &(state, layers) in THEMES {
            let tokens = resolved(layers);
            let card: Card = if state == "dark" {
                POST_DARK
            } else {
                POST_LIGHT
            };
            for (token, want) in [
                ("--surface", card.surface),
                ("--ink", card.ink),
                ("--accent", card.accent),
                ("--accent-soft", card.accent_soft),
                ("--accent-ink", card.accent_ink),
            ] {
                match literal(&tokens, token) {
                    Ok(got) if got.eq_ignore_ascii_case(want) => {}
                    Ok(got) => failures.push(format!("{state} {token}: css {got}, palette {want}")),
                    Err(reason) => failures.push(format!("{state}: {reason}")),
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn nothing_is_keyed_on_the_retired_accent() {
        // Six hues used to be `[data-accent=...]` blocks. The attribute is no longer written,
        // so a rule still keyed on it would never apply, silently.
        for retired in ["data-accent", "--swatch-", ".accent-choice", ".swatch"] {
            assert!(
                !STYLE.contains(retired),
                "{retired} is still in the stylesheet"
            );
        }
    }

    #[test]
    fn the_paper_sets_its_own_ink() {
        // `.app` is the frame and inks its text with the Space's `--f-ink`, which is chosen to
        // read on the frame's colour — dark on a light Space, light on a dark one. The card is
        // paper, not frame: anything on it that inherited the frame's ink would read in one
        // theme and vanish in the other (the reader's subject did, dark on dark).
        let color = property(&rule_body(STYLE, ".card"), "color");
        assert_eq!(
            color, "var(--ink)",
            ".card must reset the text colour to the paper's ink"
        );
    }

    #[test]
    fn the_load_bearing_layout_survives() {
        let app = declared(STYLE, ".app");
        let app_columns = app.get("grid-template-columns").map(String::as_str);
        assert!(
            app_columns.is_some_and(|value| value.contains("minmax(0, 1fr)")),
            ".app grid-template-columns is {app_columns:?}; without minmax(0, 1fr) the grid overflows the window",
        );

        let row = declared(STYLE, ".row");
        let row_columns = row
            .get("grid-template-columns")
            .map(String::as_str)
            .unwrap_or("");
        // The mockup's row is the unread dot, one flexible text track, and the time.
        // Sender and subject stack inside `.row-main`, so the old second `minmax` track is gone.
        assert_eq!(
            row_columns.matches("minmax(0,").count(),
            1,
            ".row grid-template-columns is {row_columns:?}; the text track has to be minmax(0, …) or the grid overflows the window",
        );
        assert!(
            row_columns.contains("minmax(0, 1fr)"),
            ".row grid-template-columns is {row_columns:?}"
        );
        let position = row.get("position").map(String::as_str);
        assert_eq!(
            position,
            Some("relative"),
            ".row position is {position:?}; .hover is positioned against the row",
        );

        let menu = declared(STYLE, ".row .fmenu, .row .row-menu");
        assert_eq!(
            menu.get("position").map(String::as_str),
            Some("absolute"),
            "a row menu is positioned against the row, under the strip"
        );
    }

    /// `.cmdk` used to open with `peek-in`, which animates opacity from 0. Any frame caught
    /// before it ends (a headless screenshot, a slow first paint) shows the panel translucent:
    /// the scrim and the rows behind it bleed through and the whole menu reads washed out.
    /// The open rise may move the panel, never fade it, and the overlay sits above the scrim.
    #[test]
    fn the_command_panel_is_opaque_on_its_first_frame() {
        let css = strip_comments(STYLE);
        let body = rule_body(&css, ".cmdk");
        let animation = property(&body, "animation");
        assert!(
            !animation.contains("peek-in"),
            ".cmdk animates with {animation:?}; peek-in starts transparent"
        );
        let name = animation
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(',');
        let frames = keyframes(&css, name).replace(' ', "");
        assert!(
            !frames.contains("opacity:0"),
            "@keyframes {name} fades the panel: {frames}"
        );
        // Both layers are quire's `--z-*` tokens, resolved on the `.ds` root.
        let layers = declared(ds::stylesheet(), ".ds");
        let layer = |selector: &str| -> i32 {
            let value = property(&rule_body(&css, selector), "z-index");
            let value = exactly_var(&value)
                .and_then(|name| layers.get(name))
                .cloned()
                .unwrap_or(value);
            value.parse().unwrap_or(0)
        };
        let (wrap_z, scrim_z) = (layer(".cmdk-wrap"), layer(".scrim"));
        assert!(
            wrap_z > scrim_z,
            "command menu z-index {wrap_z} is not above the scrim {scrim_z}"
        );
    }

    fn rule_body(css: &str, selector: &str) -> String {
        let bytes = css.as_bytes();
        let mut index = 0;
        let mut boundary = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'{' => {
                    let sel = css[boundary..index].trim();
                    let Some(close) = matching_close(css, index) else {
                        return String::new();
                    };
                    if sel == selector {
                        return css[index + 1..close].to_owned();
                    }
                    index = close + 1;
                    boundary = index;
                }
                b';' | b'}' => {
                    boundary = index + 1;
                    index += 1;
                }
                _ => index += 1,
            }
        }
        String::new()
    }

    fn property(body: &str, name: &str) -> String {
        body.split(';')
            .find_map(|decl| {
                let (prop, value) = decl.split_once(':')?;
                (prop.trim() == name).then(|| value.trim().to_owned())
            })
            .unwrap_or_default()
    }

    fn keyframes(css: &str, name: &str) -> String {
        let needle = format!("@keyframes {name}");
        let Some(at) = css.find(&needle) else {
            panic!("{needle} is missing");
        };
        let open = css[at..].find('{').expect("keyframes body") + at;
        let close = matching_close(css, open).expect("keyframes close");
        css[open + 1..close].to_owned()
    }

    #[test]
    fn every_pair_is_legible() {
        // Postmark, the root's default accent, in both schemes. A Space's hint is written
        // inline from `ds::derive`, and quire's own palette tests sweep every hue and chroma of
        // it against the same card.
        let mut failures = Vec::new();
        for &(state, layers) in THEMES {
            let tokens = resolved(layers);
            for &(fore, back, need) in PAIRS {
                let fore_hex = literal(&tokens, fore);
                let back_hex = literal(&tokens, back);
                match (fore_hex, back_hex) {
                    (Ok(fore_hex), Ok(back_hex)) => match ratio(&fore_hex, &back_hex) {
                        Some(measured) if measured < need => failures.push(format!(
                            "{fore} on {back}, {state}, postmark: {measured:.2} < {need:.1}"
                        )),
                        Some(_) => {}
                        None => failures.push(format!(
                            "{fore} on {back}, {state}: {fore} is not a hex colour ({fore_hex}) / {back} ({back_hex})"
                        )),
                    },
                    (fore_hex, back_hex) => {
                        if let Err(reason) = fore_hex {
                            failures.push(format!("{fore} on {back}, {state}: {reason}"));
                        }
                        if let Err(reason) = back_hex {
                            failures.push(format!("{fore} on {back}, {state}: {reason}"));
                        }
                    }
                }
            }
        }
        // A known shortfall of quire's is reported to quire, not repainted here, and each must
        // still be one: once quire fixes it, its line goes, or this test says it is stale.
        let stale: Vec<&str> = QUIRE_GAPS
            .iter()
            .filter(|(gap, _)| !failures.iter().any(|failure| failure.starts_with(gap)))
            .map(|(gap, _)| *gap)
            .collect();
        assert!(
            stale.is_empty(),
            "quire fixed these; drop them from QUIRE_GAPS: {stale:?}"
        );
        failures.retain(|failure| !QUIRE_GAPS.iter().any(|(gap, _)| failure.starts_with(gap)));
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Pairs quire's tokens fall short on, each reported to quire with its reason.
    /// Empty since quire v0.1.2 gave dark `--danger-ink` its own value (#1A0B08).
    const QUIRE_GAPS: &[(&str, &str)] = &[];

    /// Rules that are not the palette. `:root` holds the hex the contrast test reads,
    /// including when it is nested in `@media`. An at-rule is a wrapper, as in
    /// [`declarations_in`].
    fn component_rules(css: &str, out: &mut Vec<(String, String)>) {
        let bytes = css.as_bytes();
        let mut index = 0;
        let mut boundary = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'{' => {
                    let selector = css[boundary..index].trim();
                    let Some(close) = matching_close(css, index) else {
                        return;
                    };
                    let body = &css[index + 1..close];
                    if selector.starts_with('@') {
                        component_rules(body, out);
                    } else if !selector.is_empty() && !selector.starts_with(":root") {
                        out.push((selector.to_string(), body.to_string()));
                    }
                    index = close + 1;
                    boundary = index;
                }
                b';' | b'}' => {
                    boundary = index + 1;
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    /// Letter, digit, or `-`. `_` is a boundary, and so is either end of the text.
    /// Comparison stays case-sensitive: `Canvas` is not `CanvasText`, and `Highlight`
    /// is not `--highlight`.
    fn word_continues(c: char) -> bool {
        c.is_alphanumeric() || c == '-'
    }

    /// What may follow a hex colour without making it part of a longer identifier.
    /// End of input does not continue one.
    fn colour_ident_continues(c: char) -> bool {
        c.is_alphanumeric() || c == '-' || c == '_'
    }

    fn starts_with_at(chars: &[char], index: usize, needle: &str) -> bool {
        let mut rest = chars[index..].iter().copied();
        for expected in needle.chars() {
            if rest.next() != Some(expected) {
                return false;
            }
        }
        true
    }

    /// Length in chars of a `#` plus 3, 4, 6, or 8 hex digits, longest first, when the
    /// next character does not continue an identifier. `None` when `chars[index]` is not
    /// such a colour.
    fn hex_colour_at(chars: &[char], index: usize) -> Option<usize> {
        if chars[index] != '#' {
            return None;
        }
        let mut run = 0usize;
        while chars
            .get(index + 1 + run)
            .is_some_and(|c| c.is_ascii_hexdigit())
        {
            run += 1;
        }
        for len in [8usize, 6, 4, 3] {
            if run < len {
                continue;
            }
            let terminated = chars
                .get(index + 1 + len)
                .is_none_or(|c| !colour_ident_continues(*c));
            if terminated {
                return Some(len + 1);
            }
        }
        None
    }

    fn offence_at(chars: &[char], index: usize) -> Option<String> {
        if let Some(len) = hex_colour_at(chars, index) {
            return Some(chars[index..index + len].iter().collect());
        }
        for name in ["rgba(", "rgb(", "hsl(", "oklch(", "var(--edge)"] {
            if starts_with_at(chars, index, name) {
                return Some(name.to_string());
            }
        }
        if index > 0 && word_continues(chars[index - 1]) {
            return None;
        }
        let mut matched: Option<&str> = None;
        for word in [
            "CanvasText",
            "Canvas",
            "ButtonFace",
            "Highlight",
            "currentColor",
        ] {
            if !starts_with_at(chars, index, word) {
                continue;
            }
            let after = index + word.chars().count();
            let boundary = chars.get(after).is_none_or(|c| !word_continues(*c));
            if boundary && matched.is_none_or(|got| word.len() > got.len()) {
                matched = Some(word);
            }
        }
        match matched {
            // An icon's stroke follows the text it sits in. That is not a palette
            // choice, so `currentColor` is allowed only as the whole value of
            // `stroke` or `fill`. Anywhere else, including `color`, it stays forbidden.
            Some("currentColor") if current_color_is_stroke_or_fill(chars, index) => None,
            Some(word) => Some(word.to_string()),
            None => None,
        }
    }

    /// `currentColor` at `start` is the entire value of a `stroke` or `fill` declaration.
    ///
    /// `stroke-width` is a different property, and a value with another token
    /// (`stroke: currentColor extra`) is not the keyword on its own.
    fn current_color_is_stroke_or_fill(chars: &[char], start: usize) -> bool {
        let Some(property) = property_before(chars, start) else {
            return false;
        };
        if property != "stroke" && property != "fill" {
            return false;
        }
        let mut index = start + "currentColor".chars().count();
        while chars.get(index).is_some_and(|c| c.is_whitespace()) {
            index += 1;
        }
        chars.get(index).is_none_or(|c| *c == ';')
    }

    /// The property whose value starts at `value`, when only whitespace separates them.
    fn property_before(chars: &[char], value: usize) -> Option<String> {
        let mut index = value;
        while index > 0 && chars[index - 1].is_whitespace() {
            index -= 1;
        }
        if index == 0 || chars[index - 1] != ':' {
            return None;
        }
        index -= 1;
        while index > 0 && chars[index - 1].is_whitespace() {
            index -= 1;
        }
        let end = index;
        while index > 0 && word_continues(chars[index - 1]) {
            index -= 1;
        }
        (end > index).then(|| chars[index..end].iter().collect())
    }

    fn colour_offences(selector: &str, body: &str) -> Vec<String> {
        let chars: Vec<char> = body.chars().collect();
        let mut offences = Vec::new();
        let mut index = 0;
        while index < chars.len() {
            if let Some(found) = offence_at(&chars, index) {
                offences.push(format!("{selector}: {found}"));
                index += found.chars().count();
            } else {
                index += 1;
            }
        }
        offences
    }

    /// Every `@font-face` in `css`, as (family, `src`).
    fn font_faces(css: &str) -> Vec<(String, String)> {
        css.split("@font-face")
            .skip(1)
            .filter_map(|rest| {
                let body = &rest[rest.find('{')? + 1..rest.find('}')?];
                // `src` is found by name, not by splitting on `;`: a data URI contains one
                // (`data:font/woff2;base64,`), so a declaration split cuts it in half.
                let family = body.split(';').find_map(|decl| {
                    let (name, value) = decl.split_once(':')?;
                    (name.trim() == "font-family")
                        .then(|| value.trim().trim_matches('"').to_string())
                })?;
                let src = body[body.find("src:")? + "src:".len()..].trim_start();
                Some((family, src.to_string()))
            })
            .collect()
    }

    #[test]
    fn each_font_token_leads_with_a_face_we_ship() {
        // The faces are quire's `webview-fonts` rules, which the window's head carries; the
        // `--font-*` stacks are quire's tokens on `.ds`.
        let faces = font_faces(ds::font_face_css());
        let shipped: BTreeSet<&str> = faces.iter().map(|(family, _)| family.as_str()).collect();
        let tokens = declared(ds::stylesheet(), ".ds");
        for token in ["--font-display", "--font-ui", "--font-data"] {
            let Some(stack) = tokens.get(token) else {
                panic!("{token} is missing");
            };
            let first = stack
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"');
            assert!(
                shipped.contains(first),
                "{token} leads with {first:?}, which no @font-face declares; shipped: {shipped:?}"
            );
        }
        // Three families, each in both subsets; Karla and Space Mono in two styles or weights.
        assert_eq!(faces.len(), 10, "{shipped:?}");
    }

    #[test]
    fn each_face_is_an_embedded_woff2() {
        // "wOF2" is the WOFF2 signature; in base64 it begins "d09GMg". A truncated or
        // mis-encoded file, or a TTF renamed to .woff2, fails here rather than as a silent
        // fallback to the system face that only a screenshot would show.
        for (family, src) in font_faces(ds::font_face_css()) {
            assert!(
                src.starts_with("url(data:font/woff2;base64,d09GMg"),
                "{family}: {}",
                &src[..src.len().min(48)]
            );
        }
    }

    /// Custom properties a component sets on one element (from markup, per row or per spark),
    /// so they are never on `:root`. Each rule that reads one gives it a fallback or is only
    /// reached with it set.
    const PER_ELEMENT: &[&str] = &[
        "--i",
        "--a",
        "--d",
        "--dy",
        "--j",
        "--pc",
        "--g",
        "--fly-delay",
    ];

    /// The classes in `html` that no rule in `css` styles, by quire's markup lint
    /// (`ds::lint::markup`, coherence rule 2): each named once, in order.
    pub(in crate::ui) fn unstyled_classes(html: &str, css: &str) -> Vec<String> {
        let found: BTreeSet<String> = markup(html, css, &LintConfig::default())
            .into_iter()
            .filter(|offence| offence.rule == Rule::UnstyledClass)
            .filter_map(|offence| {
                let (_, class) = offence.text.rsplit_once(": ")?;
                Some(class.to_owned())
            })
            .collect();
        found.into_iter().collect()
    }

    /// What quire's markup lint still finds in `html` once mailo's named exceptions are
    /// applied: raw controls, raw vectors and literal colours in a `style`, beside unstyled
    /// classes.
    pub(in crate::ui) fn markup_offences(html: &str) -> Vec<Offence> {
        markup(
            html,
            &full_css(),
            &LintConfig {
                exceptions: super::exceptions::MARKUP,
                ..LintConfig::default()
            },
        )
    }

    /// The window's first frame with a conversation open in the reader, the command menu's
    /// open menus, and the Space editor, which the first frame never shows: opened here
    /// through the Space's name as a person would.
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
            VirtualDom::new(crate::ui::command::tests::OpenMenus).with_root_context(store);
        menus.rebuild_in_place();
        let editor = crate::ui::space_editor::tests::editor_open_markup();
        assert!(
            editor.contains("aria-label=\"Space editor\""),
            "the editor did not open: {editor}"
        );
        dioxus_ssr::render(&dom) + &dioxus_ssr::render(&menus) + &editor
    }

    #[tokio::test]
    async fn every_class_on_the_frame_is_styled() {
        let page = frame_markup().await;
        let missing = unstyled_classes(&page, &full_css());
        assert!(missing.is_empty(), "unstyled classes: {missing:?}");
    }

    /// Coherence rule 2 beyond classes: no raw control, raw vector or literal colour in a
    /// `style` on the frame, except those `exceptions::MARKUP` names with their reasons.
    #[tokio::test]
    async fn the_frame_draws_no_raw_markup_beyond_its_exceptions() {
        let page = frame_markup().await;
        let offences = markup_offences(&page);
        assert!(offences.is_empty(), "{offences:#?}");
    }

    /// Coherence rule 1: mailo's own stylesheet, at quire's strictest profile, spacing
    /// included. Every exception is named with its reason, and one that no longer suppresses
    /// anything fails here rather than hiding a later offence.
    #[test]
    fn our_stylesheet_lints_clean() {
        let config = LintConfig {
            profile: Profile::Strict,
            own_vars: PER_ELEMENT.iter().map(|name| (*name).to_owned()).collect(),
            exceptions: super::exceptions::STYLE,
        };
        assert_clean(STYLE, &config);
        let every = stylesheet(
            STYLE,
            &LintConfig {
                exceptions: &[],
                ..config.clone()
            },
        );
        let stale: Vec<_> = config
            .exceptions
            .iter()
            .filter(|exception| !every.iter().any(|offence| exception.covers(offence)))
            .collect();
        assert!(
            stale.is_empty(),
            "exceptions that suppress nothing: {stale:#?}"
        );
    }

    #[test]
    fn a_class_with_no_rule_is_named() {
        // The failure is the class name. A stylesheet that merely exists would pass a
        // test that only asked "is there CSS".
        let missing = unstyled_classes(
            "<div class=\"zz-missing\"></div>",
            &crate::ui::style::tests::full_css(),
        );
        assert_eq!(
            missing,
            vec!["zz-missing".to_owned()],
            "unstyled classes: {missing:?}"
        );
    }

    #[test]
    fn every_var_is_declared() {
        // A misspelt token is not an error anywhere: `var(--line-sfot)` resolves to nothing,
        // the declaration is dropped, and the element quietly takes its inherited value. With
        // hundreds of rules ported from the reference, this is the typo that survives review.
        // Declared means declared by quire: on the `.ds` root, in a scheme, level or accent
        // block, or written inline by the root itself (the frame's `--f-*`, `ds::FrameVars`).
        let css = strip_comments(STYLE);
        let quire = strip_comments(ds::stylesheet());
        let mut root = BTreeSet::new();
        for (at, _) in quire.match_indices("--") {
            let name: String = quire[at..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if quire[at + name.len()..].trim_start().starts_with(':') {
                root.insert(name);
            }
        }
        let look = crate::space::Space::default().look;
        for pair in ds::FrameVars::of(&look, ds::Scheme::Light)
            .style_attr()
            .split(';')
        {
            if let Some((name, _)) = pair.split_once(':') {
                root.insert(name.to_owned());
            }
        }
        let mut missing = BTreeSet::new();
        for (at, _) in css.match_indices("var(--") {
            let name: String = css[at + "var(".len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if !root.contains(&name) && !PER_ELEMENT.contains(&name.as_str()) {
                missing.insert(name);
            }
        }
        assert!(missing.is_empty(), "used but never declared: {missing:?}");
    }

    #[test]
    fn no_colour_outside_the_palette() {
        let css = strip_comments(STYLE);
        let mut rules = Vec::new();
        component_rules(&css, &mut rules);
        let mut offences = Vec::new();
        for (selector, body) in &rules {
            offences.extend(colour_offences(selector, body));
        }
        assert!(offences.is_empty(), "{}", offences.join("\n"));
    }

    #[test]
    fn current_color_is_only_a_stroke_or_fill_value() {
        // The keyword names the surrounding text. It is the whole value of
        // `stroke` or `fill`, or it is a colour outside the palette.
        const CASES: &[(&str, &str, &[&str])] = &[
            (".label", "color: currentColor", &[".label: currentColor"]),
            (".ic", "stroke: currentColor", &[]),
            (".ic", "fill: currentColor", &[]),
            (
                ".presets button",
                "background: currentColor",
                &[".presets button: currentColor"],
            ),
            (".ic", "stroke-width: currentColor", &[".ic: currentColor"]),
            (".ic", "stroke: currentColor extra", &[".ic: currentColor"]),
        ];
        let mut failures = Vec::new();
        for &(selector, body, expect) in CASES {
            let got = colour_offences(selector, body);
            let expect: Vec<String> = expect.iter().map(|offence| (*offence).to_owned()).collect();
            if got != expect {
                failures.push(format!(
                    "{selector} {{ {body} }}: got {got:?}, want {expect:?}"
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
