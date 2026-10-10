//! Who draws the add-account sheet: the window, or accountd's shell.

use super::*;
use mail_core::link::{Answer, Changes, LinkError};
use mail_core::{Accountd, Link, Transport};
use porter_core::{AccountId, Audience, Candidate, GrantId, IssuedToken, ServiceEndpoint};

/// An accountd that is never asked anything: only its presence is the test's.
#[derive(Debug)]
struct Present;

fn unasked<T>() -> Answer<'static, T> {
    Box::pin(async { Err(LinkError::Other("not asked".to_owned())) })
}

impl Accountd for Present {
    fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
        unasked()
    }
    fn token<'a>(&'a self, _: &'a GrantId, _: &'a Audience) -> Answer<'a, IssuedToken> {
        unasked()
    }
    fn open<'a>(&'a self, _: &'a GrantId, _: &'a ServiceEndpoint) -> Answer<'a, Transport> {
        unasked()
    }
    fn add_account(&self) -> Answer<'_, AccountId> {
        unasked()
    }
    fn reauthenticate<'a>(&'a self, _: &'a AccountId) -> Answer<'a, ()> {
        unasked()
    }
    fn request_grant(&self) -> Answer<'_, Candidate> {
        unasked()
    }
    fn revoke<'a>(&'a self, _: &'a GrantId) -> Answer<'a, ()> {
        unasked()
    }
    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
        Box::pin(async { Ok(None) })
    }
}

fn local() -> Link {
    Link::Local
}

fn linked() -> Link {
    Link::Accountd(Arc::new(Present))
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
