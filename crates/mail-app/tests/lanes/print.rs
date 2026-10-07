//! Lane 11: printing. The reader's Print menu offers three rows: the conversation as one
//! flowing printout, each message on pages of its own, and the conversation saved as a PDF.
//! The print dialog is a recorder; the PDF lands where the window saves files.

use ds_harness::Query;

use super::window::{INBOX, Window, deliver, hours_ago};

const PRINT: &str = ".reader-head [*|aria-label=\"Print this conversation\"]";

/// A second message in the review's conversation, so a printout has two to lay out.
fn answered(store: &mail_store::SqliteStore) {
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

/// Pick `row` from the Print menu.
fn print(window: &mut Window, row: &str) {
    window.click(PRINT);
    window.menu_item(row);
}

#[test]
fn each_print_row_hands_its_own_printout_over_and_save_as_pdf_writes_one() {
    let mut window = Window::open(answered);
    let subject = INBOX[3].1;
    window.open_subject(subject);
    window.until("both messages are in the reader", |h| {
        h.count(".reader article.frame") == 2
    });

    print(&mut window, "Print Conversation…");
    window.until_printed("the print dialog is handed the conversation", 1);
    window.until("the toast says how it ended", |h| {
        h.text_of(".ds-toast-body")
            .is_some_and(|toast| toast.contains("Printing cancelled"))
    });
    print(&mut window, "Print Each Message Separately…");
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

    print(&mut window, "Save Conversation as PDF…");
    window.until("the toast says where it saved it", |h| {
        h.text_of(".ds-toast-body")
            .is_some_and(|toast| toast.starts_with("Saved for printing to"))
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
}
