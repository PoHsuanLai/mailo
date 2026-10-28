//! Where there is no session bus: nothing is served and nothing is delivered.

use mail_client::profile::ProfileId;
use mail_domain::ThreadId;
use porter_core::AccountId;

/// Never constructed.
#[derive(Debug, Clone)]
pub struct Requests;

pub fn serve(_profile: &ProfileId) -> Option<Requests> {
    None
}

pub fn deliver(_profile: &ProfileId, _thread: ThreadId, _token: Option<&str>) -> bool {
    false
}

pub fn deliver_mailto(_profile: &ProfileId, _uri: &str, _token: Option<&str>) -> bool {
    false
}

pub fn deliver_inbox(_profile: &ProfileId, _account: &AccountId, _token: Option<&str>) -> bool {
    false
}

pub fn deliver_activate(_profile: &ProfileId, _token: Option<&str>) -> bool {
    false
}

pub fn deliver_reply(_profile: &ProfileId, _thread: ThreadId, _token: Option<&str>) -> bool {
    false
}

pub fn deliver_archive(_profile: &ProfileId, _thread: ThreadId, _token: Option<&str>) -> bool {
    false
}
