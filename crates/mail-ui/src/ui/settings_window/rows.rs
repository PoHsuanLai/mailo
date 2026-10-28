//! The rows more than one Settings pane draws the same way.

use dioxus::prelude::*;
use ds::base::press::Press;
use ds::components::lists::list::model::ListItem;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;

/// A row that opens a page: `title` with a chevron, and `leading` before it. Its list key is
/// `key`, which is the row's own id for the panes that find it by that.
pub(super) fn link_row(
    key: &str,
    title: &str,
    leading: RowLeading,
    onclick: impl FnMut(Press) + 'static,
    common: Common,
) -> ListItem<String> {
    ListItem::row(
        key.to_owned(),
        title,
        rsx! {
            Row {
                leading,
                title: title.to_owned(),
                size: RowSize::Settings,
                accessory: Accessory::Chevron,
                onclick: Some(EventHandler::new(onclick)),
                common,
            }
        },
    )
}
