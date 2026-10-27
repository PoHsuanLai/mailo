//! How a sidebar row shows a dragged conversation: outlined while it could land there, lit
//! while the pointer is over it. Places and folders keep their own record of the drag, but both
//! read it through this one rule.

use ds::base::vocab::DropState;

/// Whether the conversation being dragged could be let go on a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Landing {
    Possible,
    Impossible,
}

/// Whether the pointer is over a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Under {
    Yes,
    No,
}

/// The row's drop state: idle unless the drag could land here, then lit if the pointer is over it
/// and only outlined if not.
pub(super) fn lit(landing: Landing, under: Under) -> DropState {
    match (landing, under) {
        (Landing::Impossible, _) => DropState::Idle,
        (Landing::Possible, Under::Yes) => DropState::Target,
        (Landing::Possible, Under::No) => DropState::Accepts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_lit_only_when_the_drag_could_land_on_it() {
        let table = [
            (Landing::Impossible, Under::No, DropState::Idle),
            (Landing::Impossible, Under::Yes, DropState::Idle),
            (Landing::Possible, Under::No, DropState::Accepts),
            (Landing::Possible, Under::Yes, DropState::Target),
        ];
        for (landing, under, want) in table {
            assert_eq!(lit(landing, under), want, "{landing:?} {under:?}");
        }
    }
}
