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
//! with "only literals can be passed to `concat!()`". So the dark guards are spelled out
//! here rather than folded into a `TOKENS` of their own.
//!
//! Three rules the files themselves cannot state:
//!
//! - **The dark palette is written once.** `tokens.dark.css` holds bare declarations with no
//!   selector, and it is included twice below: inside the `prefers-color-scheme` guard, and
//!   inside `:root[data-theme="dark"]`. Three theme states, one source of truth. The guard is
//!   `:root:not([data-theme="light"])` so an explicit light choice beats a dark desktop.
//! - **`accents.css` comes after the dark block.** A light accent block and the dark palette
//!   block are both one attribute deep, so source order is what separates them. The six dark
//!   accent blocks carry a second attribute and win from there regardless -- belt as well as
//!   braces, because this is the failure that looks like "the accent doesn't work in dark".
//! - **Contrast-critical tokens are literal `#rrggbb`.** The contrast test parses them; it
//!   cannot read a `color-mix()`, and it must fail rather than skip when it meets one.
//!   `color-mix()` is for decoration -- hovers, washes, the shadow.

#[cfg(test)]
mod contrast;

pub(super) const STYLE: &str = concat!(
    include_str!("reset.css"),
    include_str!("tokens.css"),
    "@media (prefers-color-scheme: dark) {\n  :root:not([data-theme=\"light\"]) {\n",
    include_str!("tokens.dark.css"),
    "  }\n}\n",
    ":root[data-theme=\"dark\"] {\n",
    include_str!("tokens.dark.css"),
    "}\n",
    include_str!("accents.css"),
    include_str!("shell.css"),
    include_str!("list.css"),
    include_str!("menus.css"),
    include_str!("reader.css"),
    include_str!("composer.css"),
    include_str!("controls.css"),
);

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{STYLE, contrast};

    const SLUGS: &[&str] = &[
        "postmark",
        "graphite",
        "pine",
        "indigo",
        "oxblood",
        "vermilion",
    ];

    const ACCENT_KEYS: &[&str] = &["--accent", "--accent-ink", "--accent-soft", "--seal"];

    const PAIRS: &[(&str, &str, f64)] = &[
        ("--ink", "--paper", 4.5),
        ("--ink", "--paper-raised", 4.5),
        ("--ink", "--paper-sunken", 4.5),
        ("--ink-dim", "--paper", 4.5),
        ("--ink-dim", "--paper-raised", 4.5),
        ("--ink-faint", "--paper", 3.0),
        ("--ink-faint", "--paper-raised", 3.0),
        ("--danger", "--paper", 4.5),
        ("--danger-ink", "--danger", 4.5),
        ("--accent-ink", "--accent", 4.5),
        ("--accent", "--paper", 3.0),
        ("--accent", "--paper-raised", 3.0),
    ];

    /// Light, then dark because the desktop prefers it, then dark because the
    /// document asked for it. The overlay is the palette laid over bare `:root`;
    /// light has none.
    const THEMES: &[(&str, Option<&str>)] = &[
        ("light", None),
        ("dark (desktop)", Some(r#":root:not([data-theme="light"])"#)),
        ("dark (attribute)", Some(r#":root[data-theme="dark"]"#)),
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
        if contrast::ratio(resolved, "#000000").is_none() {
            return Err(format!("{name} is not a hex colour ({resolved})"));
        }
        Ok(resolved.clone())
    }

    fn resolved(overlay: Option<&str>, accent: &str) -> BTreeMap<String, String> {
        let mut tokens = declared(STYLE, ":root");
        if let Some(selector) = overlay {
            tokens.extend(declared(STYLE, selector));
        }
        tokens.extend(declared(STYLE, accent));
        tokens
    }

    fn accent_selector(overlay: Option<&str>, slug: &str) -> String {
        match overlay {
            Some(selector) => format!(r#"{selector}[data-accent="{slug}"]"#),
            None => format!(r#":root[data-accent="{slug}"]"#),
        }
    }

    #[test]
    fn a_dark_token_also_has_a_light_value() {
        let light = declared(STYLE, ":root");
        let dark = declared(STYLE, r#":root[data-theme="dark"]"#);
        let dark_tokens: Vec<&str> = dark
            .keys()
            .filter(|name| name.starts_with("--"))
            .map(String::as_str)
            .collect();
        assert!(
            !dark_tokens.is_empty(),
            "no custom properties on :root[data-theme=\"dark\"]",
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
    fn an_accent_declares_exactly_the_four() {
        let expect: BTreeSet<&str> = ACCENT_KEYS.iter().copied().collect();
        let mut failures = Vec::new();
        for slug in SLUGS {
            for selector in [
                format!(r#":root[data-accent="{slug}"]"#),
                format!(r#":root:not([data-theme="light"])[data-accent="{slug}"]"#),
                format!(r#":root[data-theme="dark"][data-accent="{slug}"]"#),
            ] {
                let decls = declared(STYLE, &selector);
                let got: BTreeSet<&str> = decls.keys().map(String::as_str).collect();
                if got != expect {
                    let missing: Vec<_> = expect.difference(&got).collect();
                    let extra: Vec<_> = got.difference(&expect).collect();
                    failures.push(format!("{selector}: missing {missing:?}, extra {extra:?}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn the_decoration_follows_the_dark_palette() {
        // The one-line `color-scheme` rule means `:root[data-theme="dark"]` is the wrong
        // anchor: it occurs before the palette, so the assertion would hold with the files
        // misordered. The desktop guard occurs once, around the palette.
        let Some(palette) = STYLE.rfind(":root:not([data-theme=\"light\"]) {") else {
            panic!("the desktop dark palette selector is missing");
        };
        let Some(decoration) = STYLE.find("[data-accent=") else {
            panic!("no [data-accent= selector");
        };
        assert!(
            decoration > palette,
            "first [data-accent= at {decoration}, last desktop dark palette at {palette}",
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
        assert_eq!(
            row_columns.matches("minmax(0,").count(),
            2,
            ".row grid-template-columns is {row_columns:?}; without both minmax(0, …) tracks the grid overflows the window",
        );
        let position = row.get("position").map(String::as_str);
        assert_eq!(
            position,
            Some("relative"),
            ".row position is {position:?}; .hover is positioned against the row",
        );

        let labels = declared(STYLE, ".labels");
        let span = labels.get("grid-column").map(String::as_str);
        assert_eq!(
            span,
            Some("1 / -1"),
            ".labels grid-column is {span:?}; the menus are grid items of the row",
        );
    }

    #[test]
    fn every_pair_is_legible() {
        let mut failures = Vec::new();
        for &(state, overlay) in THEMES {
            for slug in SLUGS {
                let tokens = resolved(overlay, &accent_selector(overlay, slug));
                for &(fore, back, need) in PAIRS {
                    let fore_hex = literal(&tokens, fore);
                    let back_hex = literal(&tokens, back);
                    match (fore_hex, back_hex) {
                        (Ok(fore_hex), Ok(back_hex)) => match contrast::ratio(&fore_hex, &back_hex)
                        {
                            Some(measured) if measured < need => failures.push(format!(
                                "{fore} on {back}, {state}, {slug}: {measured:.2} < {need:.1}"
                            )),
                            Some(_) => {}
                            None => failures.push(format!(
                                "{fore} on {back}, {state}, {slug}: {fore} is not a hex colour ({fore_hex}) / {back} ({back_hex})"
                            )),
                        },
                        (fore_hex, back_hex) => {
                            if let Err(reason) = fore_hex {
                                failures.push(format!("{fore} on {back}, {state}, {slug}: {reason}"));
                            }
                            if let Err(reason) = back_hex {
                                failures.push(format!("{fore} on {back}, {state}, {slug}: {reason}"));
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
