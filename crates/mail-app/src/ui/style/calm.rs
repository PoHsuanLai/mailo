//! The motion this step added answers to Calm and to reduced motion.
//!
//! Each new animation is named here by its rule. Its duration must go through a `--t-*` token,
//! because a literal is a number `data-motion="calm"` cannot reach; under Calm that token must
//! resolve to Calm's own value, never slower than Standard; and its curve must be a token that
//! Calm flattens, so nothing overshoots.

use super::STYLE;
use std::collections::BTreeMap;

/// `(rule, property that holds the timing)`. Selectors are compared whole. A count's bump is
/// quire's pulse class now (`a-bump`), timed by quire's own recipe, so it is not listed.
const NEW: &[(&str, &str)] = &[
    (".list .row[*|data-presence=leaving]", "animation"),
    (
        ".list .row[*|data-presence=leaving][*|data-exit=curl]",
        "animation",
    ),
    (
        ".list .row[*|data-presence=leaving][*|data-read=unread]",
        "animation-duration",
    ),
    (
        ".list .row[*|data-presence=leaving][*|data-exit=curl][*|data-read=unread]",
        "animation-duration",
    ),
    (".list .row[*|data-presence=healing]", "animation"),
    (".list .row.returning", "animation"),
    (".row .floater", "animation"),
    (".chip.is-landing", "animation"),
    (".item.gulp", "animation"),
    (".toast", "animation"),
    (".hc", "animation"),
    (".linkpill", "animation"),
    (".item[*|aria-current=\"true\"]::before", "animation"),
];

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .split_once("*/")
            .map_or("", |(_, after)| after);
    }
    out.push_str(rest);
    out
}

/// Every `(selector, body)` in `css`, descending into `@media` and skipping `@keyframes`.
fn rules(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = css.as_bytes();
    let (mut index, mut boundary) = (0, 0);
    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                let selector = css[boundary..index].trim().to_owned();
                let mut depth = 0;
                let mut close = index;
                for (at, byte) in bytes.iter().enumerate().skip(index) {
                    match byte {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                close = at;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let body = &css[index + 1..close];
                if selector.starts_with("@media") {
                    out.extend(rules(body));
                } else if !selector.starts_with('@') {
                    out.push((selector, body.to_owned()));
                }
                index = close + 1;
                boundary = index;
            }
            b';' | b'}' => {
                index += 1;
                boundary = index;
            }
            _ => index += 1,
        }
    }
    out
}

fn declarations(body: &str) -> BTreeMap<String, String> {
    body.split(';')
        .filter_map(|decl| {
            let (name, value) = decl.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// The tokens in one motion level: quire's `.ds` root, with the level's block laid over it.
/// The motion tokens are quire's now; the rules that spend them are still mailo's.
fn tokens(level: Option<&str>) -> BTreeMap<String, String> {
    let all = rules(&strip_comments(ds::stylesheet()));
    let mut out = BTreeMap::new();
    for (selector, body) in &all {
        if selector == ".ds" {
            out.extend(declarations(body));
        }
    }
    if let Some(level) = level {
        let wanted = format!(".ds[*|data-motion={level}]");
        for (selector, body) in &all {
            if *selector == wanted {
                out.extend(declarations(body));
            }
        }
    }
    out
}

/// Split a shorthand into its top-level terms: `calc(a * b)` stays one term.
fn terms(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for ch in value.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if ch.is_whitespace() && depth == 0 {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn var_name(term: &str) -> Option<&str> {
    term.trim()
        .strip_prefix("var(")?
        .strip_suffix(')')
        .map(str::trim)
}

fn millis(literal: &str) -> Option<f64> {
    let literal = literal.trim();
    if let Some(ms) = literal.strip_suffix("ms") {
        return ms.trim().parse().ok();
    }
    literal
        .strip_suffix('s')
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|s| s * 1000.0)
}

/// A duration term's value in milliseconds under `tokens`, and the `--t-*` token it goes
/// through. `Err` when it is a literal or names no motion token.
fn duration(term: &str, tokens: &BTreeMap<String, String>) -> Result<(f64, String), String> {
    let inner = term
        .strip_prefix("calc(")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(term);
    let mut value = 1.0;
    let mut through = None;
    for factor in inner.split('*').map(str::trim) {
        if let Some(name) = var_name(factor) {
            if !name.starts_with("--t-") {
                return Err(format!("{factor} is not a motion duration token"));
            }
            let resolved = tokens
                .get(name)
                .and_then(|raw| millis(raw))
                .ok_or_else(|| format!("{name} does not resolve to a duration"))?;
            value *= resolved;
            through = Some(name.to_owned());
        } else if millis(factor).is_some() {
            return Err(format!("{factor} is a literal duration Calm cannot reach"));
        } else {
            value *= factor
                .parse::<f64>()
                .map_err(|_| format!("{factor} is not a number"))?;
        }
    }
    through
        .map(|name| (value, name))
        .ok_or_else(|| format!("{term} goes through no --t-* token"))
}

/// Whether a curve token, resolved under `tokens`, overshoots.
fn overshoots(term: &str, tokens: &BTreeMap<String, String>) -> Result<bool, String> {
    let mut current = term.to_owned();
    for _ in 0..4 {
        match var_name(&current) {
            Some(name) => {
                current = tokens
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("{name} is not declared"))?;
            }
            None => break,
        }
    }
    let Some(inner) = current
        .strip_prefix("cubic-bezier(")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return Err(format!("{term} resolves to {current}, not a cubic-bezier"));
    };
    let points: Vec<f64> = inner
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    Ok(points.len() == 4 && (points[1] > 1.0 || points[3] > 1.0))
}

#[test]
fn every_new_animation_answers_to_calm() {
    let css = strip_comments(STYLE);
    let all = rules(&css);
    let standard = tokens(None);
    let calm = tokens(Some("calm"));
    let mut failures = Vec::new();
    for (selector, property) in NEW {
        // The last rule with this selector is the one that wins.
        let Some(body) = all
            .iter()
            .rev()
            .find(|(sel, _)| sel == selector)
            .map(|(_, body)| body)
        else {
            failures.push(format!("{selector}: no such rule"));
            continue;
        };
        let decls = declarations(body);
        let Some(value) = decls.get(*property) else {
            failures.push(format!("{selector}: no {property}"));
            continue;
        };
        let parts = terms(value);
        let time = if *property == "animation" {
            parts
                .iter()
                .find(|term| term.contains("--t-") || millis(term).is_some())
                .cloned()
        } else {
            Some(value.clone())
        };
        let Some(time) = time else {
            failures.push(format!("{selector}: {value} has no duration"));
            continue;
        };
        match (duration(&time, &calm), duration(&time, &standard)) {
            (Ok((under_calm, token)), Ok((under_standard, _))) => {
                let calm_token = calm.get(&token).and_then(|raw| millis(raw));
                if calm_token.is_none() {
                    failures.push(format!("{selector}: {token} has no value under calm"));
                }
                if under_calm > under_standard {
                    failures.push(format!(
                        "{selector}: {under_calm}ms under calm, slower than {under_standard}ms"
                    ));
                }
            }
            (Err(why), _) | (_, Err(why)) => failures.push(format!("{selector}: {why}")),
        }
        if *property == "animation" {
            match parts.iter().find(|term| term.starts_with("var(--e-")) {
                Some(curve) => match overshoots(curve, &calm) {
                    Ok(true) => failures.push(format!("{selector}: {curve} overshoots under calm")),
                    Ok(false) => {}
                    Err(why) => failures.push(format!("{selector}: {why}")),
                },
                None => failures.push(format!("{selector}: {value} names no --e-* curve")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_literal_duration_is_named() {
    // The check above is only worth something if it can fail.
    let calm = tokens(Some("calm"));
    assert!(
        duration("560ms", &calm)
            .unwrap_err()
            .contains("literal duration")
    );
    assert!(duration("calc(var(--t-big) * 1.33)", &calm).is_ok());
}

#[test]
fn the_squash_shapes_are_quires_and_calm_still_reaches_them() {
    // Gulp, bump and the seal are quire's keyframes now; mailo declares none (coherence rule 1)
    // and only names them. Calm reaches them through the level's tokens: the spring they play
    // on does not overshoot under it, and `--squish` and `--overshoot` are 1 there.
    let css = strip_comments(STYLE);
    let quire = strip_comments(ds::stylesheet());
    for name in ["gulp", "bump", "seal-pop"] {
        assert!(
            !css.contains(&format!("@keyframes {name}")),
            "mailo still declares @keyframes {name}"
        );
        assert!(
            quire.contains(&format!("@keyframes {name}")),
            "quire has no @keyframes {name}"
        );
    }
    let calm = tokens(Some("calm"));
    assert_eq!(overshoots("var(--e-spring)", &calm), Ok(false));
    assert_eq!(calm.get("--squish").map(String::as_str), Some("1"));
    assert_eq!(calm.get("--overshoot").map(String::as_str), Some("1"));
}

#[test]
fn reduced_motion_still_ends_every_animation() {
    // A leaving row is removed once its exit settles on quire's clock, so reduced motion must
    // shorten animations
    // rather than remove them: `none` would leave the row until the fallback. The desktop's
    // setting is quire's `reduced` level now (`Motion::with_desktop`), not a media query here:
    // under it no duration token is zero, and the ones that time an animation are short.
    let reduced = tokens(Some("reduced"));
    let mut failures = Vec::new();
    for (name, value) in reduced.iter().filter(|(name, _)| name.starts_with("--t-")) {
        match millis(value) {
            Some(ms) if ms > 0.0 => {}
            other => failures.push(format!("{name}: {value} ({other:?})")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(
        reduced.get("--t-big").and_then(|value| millis(value)),
        Some(60.0),
        "the reduced level is not 60ms"
    );
}
