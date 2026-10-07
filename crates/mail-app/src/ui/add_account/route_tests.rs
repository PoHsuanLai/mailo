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

fn ask(address: Option<&str>) -> Ask {
    Ask {
        address: address.map(str::to_owned),
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
    let local = Link::Local;
    assert!(window(&route_of(&local, &ask(None), |_| true)));
    assert!(window(&route_of(
        &local,
        &ask(Some("ada@example.test")),
        |_| true
    )));
}

#[test]
fn with_accountd_a_new_account_is_added_by_its_sheet_and_mailo_opens_no_window() {
    assert!(matches!(
        route_of(&linked(), &ask(None), |_| false),
        Route::Accountd(_)
    ));
}

#[test]
fn signing_in_again_goes_where_the_accounts_sign_in_is() {
    let accountds = |address: &str| address == "ada@example.test";
    // An account of accountd's is signed in again by accountd's sheet.
    assert!(matches!(
        route_of(&linked(), &ask(Some("ada@example.test")), accountds),
        Route::Accountd(_)
    ));
    // One mailo holds the secrets of itself keeps mailo's window: accountd cannot sign it in.
    assert!(window(&route_of(
        &linked(),
        &ask(Some("bob@example.test")),
        accountds
    )));
}
