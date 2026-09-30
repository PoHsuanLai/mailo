//! What mailo puts on a quire component's own element.

use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;

/// A component's own layout class: quire writes it after its own, never in place of one, so a
/// mailo rule can size or place the element without naming a `ds-` class.
pub(in crate::ui) fn classed(class: &str) -> Common {
    Common {
        extra_class: ExtraClass::parse(class).ok(),
        ..Common::default()
    }
}
