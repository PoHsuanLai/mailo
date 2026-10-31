//! The files that draw the list, the window and the status ask the store nothing: what a frame
//! shows is a query the client keeps read, and what a press changes is a command. A call to
//! `stored()` on the thread that draws waits behind whatever an ingest holds. (The list's own
//! reading, in `mail-ui-kit`, is held to it there.)

#[test]
fn the_files_that_draw_ask_the_store_nothing() {
    let files = [
        ("list.rs", include_str!("../list.rs")),
        ("window/root.rs", include_str!("../window/root.rs")),
        ("fetching/mod.rs", include_str!("../fetching/mod.rs")),
        ("menus/mod.rs", include_str!("../menus/mod.rs")),
        ("views.rs", include_str!("../views.rs")),
        ("organise.rs", include_str!("../organise.rs")),
        ("reading/mod.rs", include_str!("../reading/mod.rs")),
    ];
    for (name, source) in files {
        let drawn = source.split("#[cfg(test)]").next().unwrap_or(source);
        let calls: Vec<&str> = drawn
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains(".stored()"))
            .collect();
        assert!(calls.is_empty(), "{name} asks the store: {calls:?}");
    }
}
