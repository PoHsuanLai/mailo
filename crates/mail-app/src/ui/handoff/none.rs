//! Where there is no session bus: nothing is served and nothing is delivered.

use super::Request;
use mail_domain::ThreadId;

/// Never constructed.
#[derive(Debug, Clone)]
pub struct Requests;

pub fn serve() -> Option<Requests> {
    None
}

pub fn deliver(_thread: ThreadId, _token: Option<&str>) -> bool {
    false
}

pub(in crate::ui) fn use_handoff() {
    let _: Option<Request> = None;
}
