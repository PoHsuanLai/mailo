use super::*;

#[test]
fn matched_characters_are_runs_not_markup() {
    let parts = pieces("alpha", &[0, 1]);
    assert_eq!(
        parts,
        vec![Piece::Mark("al".to_owned()), Piece::Plain("pha".to_owned())]
    );
}
