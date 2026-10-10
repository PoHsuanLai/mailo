//! Who draws the add-account sheet: the window, or accountd's shell.

use super::*;
use mail_core::Link;
use mail_core::testing::FakeAccountd;

fn local() -> Link {
    Link::Local
}

fn linked() -> Link {
    Link::Accountd(Arc::new(FakeAccountd::default()))
}

/// Without accountd the window draws every sheet. With it, every sheet is accountd's and mailo
/// opens no window: a new account and one signed in again take the same route, because accountd
/// holds them all.
#[test]
fn route_of_each_link() {
    // (link, its maker, whether mailo draws the sheet in its own window)
    type Row = (&'static str, fn() -> Link, bool);
    const CASES: &[Row] = &[("local", local, true), ("accountd", linked, false)];
    for (name, link, own_window) in CASES {
        let route = route_of(&link());
        assert_eq!(matches!(route, Route::OwnWindow), *own_window, "{name}");
        assert_eq!(matches!(route, Route::Accountd(_)), !*own_window, "{name}");
    }
}
