//! Who draws the add-account sheet: the window, or accountd's shell.

use super::*;
use mail_runtime::link::{Accountd, Answer, Changes, LinkError};
use mail_runtime::{Link, Transport};
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

fn linked() -> Link {
    Link::Accountd(Arc::new(Present))
}

fn window(route: &Route) -> bool {
    matches!(route, Route::OwnWindow)
}

#[test]
fn without_accountd_the_window_draws_every_sheet() {
    assert!(window(&route_of(&Link::Local)));
}

#[test]
fn with_accountd_every_sheet_is_accountds_and_mailo_opens_no_window() {
    // A new account, and one signed in again: the same route, because accountd holds them all.
    assert!(matches!(route_of(&linked()), Route::Accountd(_)));
}
