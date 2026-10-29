//! Where there is no session bus: nothing is served and nothing is delivered.

use crate::ui::ask::Ask;
use mail_client::profile::ProfileId;

/// Never constructed.
#[derive(Debug, Clone)]
pub struct Requests;

pub fn serve(_profile: &ProfileId) -> Option<Requests> {
    None
}

pub fn deliver(_profile: &ProfileId, _ask: &dyn Ask, _token: Option<&str>) -> bool {
    false
}
