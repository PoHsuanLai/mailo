//! The shell's stylesheet.
//!
//! Real `.css` files, stitched at compile time. Splitting by concern rather than by line
//! count is what CONVENTIONS.md section 8 asks for: `list.css` is one concept in a way that
//! "lines 25 to 34 of the stylesheet" never is.
//!
//! The order below is the cascade and is load-bearing. Keep it.

pub(super) const STYLE: &str = concat!(
    include_str!("reset.css"),
    include_str!("tokens.css"),
    include_str!("shell.css"),
    include_str!("list.css"),
    include_str!("menus.css"),
    include_str!("reader.css"),
    include_str!("composer.css"),
    include_str!("controls.css"),
);
