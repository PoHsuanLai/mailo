//! Lane 11: printing. Print… in the reader's ⋯ offers three rows: the conversation as one
//! flowing printout, each message on pages of its own, and the conversation saved as a PDF.
//! The print dialog is a recorder; the PDF lands where the window saves files. ⌘P prints the
//! open conversation as the first row does.

use mail_ui::ui::copy;

use super::drive::{Key, PRIMARY};
use super::window::{INBOX, Window, deliver, hours_ago};

/// The rows of Print…, by their place, which take the ⋯'s: a rule stands between printing and
/// saving.
const FLOW: usize = 1;
const APART: usize = 2;
const SAVE: usize = 4;

/// A second message in the review's conversation, so a printout has two to lay out.
fn answered(store: &mail_core::SqliteStore) {
    let subject = INBOX[3].1;
    deliver(
        store,
        "review2",
        &format!(
            "From: Alan Turing <alan@example.test>\r\nTo: Me <me@example.test>\r\n\
             Subject: Re: {subject}\r\nDate: {}\r\nMessage-ID: <review2@example.test>\r\n\
             In-Reply-To: <seed3@example.test>\r\nReferences: <seed3@example.test>\r\n\r\n\
             Agreed on every point.\r\n",
            hours_ago(0)
        ),
    );
}

/// The pages of `pdf`.
fn pages(pdf: &[u8]) -> u32 {
    pdfrum::Document::from_bytes(pdf.to_vec())
        .expect("the printout opens as a PDF")
        .page_count()
}

/// Pick the `row`th row of the Print… menu off the reader's ⋯.
fn print(window: &mut Window, row: usize) {
    window.click(&format!(
        ".reader-head [*|aria-label=\"{}\"]",
        copy::aria::MORE
    ));
    window.menu_item(copy::menus::PRINT);
    let rows = ".ds-menu .ds-menu-list";
    window.until("Print… opens its rows in the menu's place", |h| {
        h.count(&format!("{rows} > *")) == 4
            && h.count(&format!("{rows} > .ds-menu-separator:nth-child(3)")) == 1
    });
    window.click(&format!("{rows} > :nth-child({row})"));
}

/// What the toast says, once it says something.
fn toast(window: &mut Window) -> String {
    window.until("a toast", |h| h.count(".ds-toast-body") == 1);
    window.text(".ds-toast-body")
}

#[test]
fn each_print_row_hands_its_own_printout_over_and_save_as_pdf_writes_one() {
    let mut window = Window::open(answered);
    let subject = INBOX[3].1;
    window.open_subject(subject);
    window.until("both messages are in the reader", |h| {
        h.count(".reader article.frame") == 2
    });

    print(&mut window, FLOW);
    window.until_printed("the print dialog is handed the conversation", 1);
    assert!(
        !toast(&mut window).trim().is_empty(),
        "no word of how it ended"
    );
    print(&mut window, APART);
    window.until_printed("the print dialog is handed each message", 2);
    let printed = window.printed();
    assert_eq!(printed.len(), 2, "not two printouts");
    let (flow, apart) = (&printed[0], &printed[1]);
    assert!(flow.0.starts_with(b"%PDF"), "the first printout is no PDF");
    assert_eq!(flow.1, subject);
    assert_eq!(apart.1, subject);
    assert!(
        pages(&apart.0) >= 2 && pages(&apart.0) > pages(&flow.0),
        "each message did not get pages of its own: {} against {}",
        pages(&apart.0),
        pages(&flow.0)
    );

    print(&mut window, SAVE);
    let saves = window.saves();
    window.until("the toast says where it saved it", |h| {
        h.text_of(".ds-toast-body")
            .is_some_and(|toast| toast.contains(&*saves.to_string_lossy()))
    });
    let saved: Vec<_> = std::fs::read_dir(window.saves())
        .expect("the save directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    let bytes = std::fs::read(&saved[0]).expect("the saved PDF");
    assert!(bytes.starts_with(b"%PDF"), "not a PDF: {:?}", saved[0]);
    assert_eq!(window.printed().len(), 2, "saving opened the print dialog");

    // ⌘P hands the open conversation over as one flowing printout.
    window.press(&[PRIMARY], Key::Char('p'), 1);
    window.until_printed("⌘P hands the conversation over", 3);
    let printed = window.printed();
    assert_eq!(printed[2].1, subject);
    assert_eq!(pages(&printed[2].0), pages(&printed[0].0));
}
