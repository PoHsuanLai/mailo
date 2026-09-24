//! The printed document in a window of its own, and the platform's print dialog over it.
//!
//! The mail is never put in this window's page: that page is the application, with its script and
//! its origin, and a message has no business sharing either. It gets a separate webview whose only
//! content is the printed document. That webview has JavaScript switched off, devtools off, the
//! clipboard off, an ephemeral web context, no navigation away from the document, no new windows
//! and no downloads, and the document keeps its own Content-Security-Policy.
//!
//! **How the window is made.** `dioxus-desktop` 0.7 can only open a window around a `VirtualDom`
//! (`DesktopService::new_window`), which would bring its interpreter script into the printed page.
//! What it does offer is `create_wry_event_handler`, whose handler is given the event loop's
//! window target; with that target a plain `tao` window and a `wry` webview built from an HTML
//! string are ordinary calls. So the whole life of the print window runs in that handler, one
//! step per event, as [`decide`] says.
//!
//! **How it prints.** `wry`'s `WebView::print` is a `webkit2gtk::PrintOperation` whose result it
//! drops, and printing continues after the dialog closes: close the webview then and the pages are
//! lost. On Linux the operation is made here instead, so its `finished` signal says when the
//! window may go. Elsewhere `wry`'s call is used and the window is kept for a minute.
//!
//! The dialog runs a main loop of its own inside the handler. `tao` calls its handlers from its
//! own loop, not from a glib source, so no handler runs again until the dialog closes; the
//! application's page still takes clicks meanwhile, and what they change is drawn afterwards. One
//! print window at a time ([`busy`]): a second would be waiting on the first one's dialog.

use dioxus::desktop::tao::event_loop::EventLoopWindowTarget;
use dioxus::desktop::tao::window::{Window, WindowBuilder};
use dioxus::desktop::{DesktopContext, LogicalSize, wry};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the document may take to load before the window gives up on it.
const LOAD_LIMIT: Duration = Duration::from_secs(30);
/// How long a print may run before its window is closed anyway: a safety net, since the
/// operation says when it has finished.
#[cfg(target_os = "linux")]
const PRINT_LIMIT: Duration = Duration::from_secs(600);
/// Where nothing says a print has finished, how long its window is kept after the dialog.
#[cfg(not(target_os = "linux"))]
const PRINT_LIMIT: Duration = Duration::from_secs(60);
/// How often the waiting task looks at the window, and nudges the event loop.
const TICK: Duration = Duration::from_millis(250);

thread_local! {
    /// Whether a print window is open. One at a time: see the module notes.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// Whether a print window is open now.
pub(super) fn busy() -> bool {
    BUSY.with(Cell::get)
}

/// Where a print window is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// The document is loading into the new webview.
    Loading,
    /// The dialog has been answered with Print; the pages are being made.
    Printing,
}

/// What has been heard from the webview and the print operation.
#[derive(Debug, Default)]
pub(super) struct Heard {
    pub loaded: Cell<bool>,
    pub finished: Cell<bool>,
    pub failed: RefCell<Option<String>>,
}

/// What to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Move {
    Wait,
    /// The document is in: show the dialog.
    Print,
    /// Close the window, and say why when it went wrong.
    Close(Result<(), String>),
}

/// The next move for a window in `phase` for `elapsed`, given what was `heard`.
pub(super) fn decide(phase: Phase, heard: &Heard, elapsed: Duration, limit: Duration) -> Move {
    if let Some(why) = heard.failed.borrow().clone() {
        return Move::Close(Err(format!("Printing failed: {why}")));
    }
    match phase {
        Phase::Loading if heard.loaded.get() => Move::Print,
        Phase::Loading if elapsed >= LOAD_LIMIT => Move::Close(Err(
            "The printout did not load, so nothing was printed.".to_owned(),
        )),
        Phase::Printing if heard.finished.get() || elapsed >= limit => Move::Close(Ok(())),
        Phase::Loading | Phase::Printing => Move::Wait,
    }
}

/// The window and the webview in it.
///
/// Fields drop in order: the operation, then the page, then the window around it.
struct Sheet {
    /// Held until it finishes: dropping it early is what loses the pages.
    #[cfg(target_os = "linux")]
    operation: Option<webkit2gtk::PrintOperation>,
    webview: wry::WebView,
    window: Window,
}

enum Stage {
    Asked {
        html: String,
        title: String,
    },
    Open {
        sheet: Box<Sheet>,
        phase: Phase,
        since: Instant,
    },
    Done(Result<(), String>),
    /// Reported; the handler does nothing more.
    Gone,
}

/// Open `html` in a window of its own and print it. Resolves when the window is gone.
///
/// Refused while another print window is open.
pub(super) async fn print(
    desktop: DesktopContext,
    html: String,
    title: String,
) -> Result<(), String> {
    if BUSY.with(|busy| busy.replace(true)) {
        return Err("A printout is already open.".to_owned());
    }
    let stage = Rc::new(RefCell::new(Stage::Asked { html, title }));
    let heard = Rc::new(Heard::default());
    let main = desktop.window.clone();
    let handler = desktop.create_wry_event_handler({
        let stage = stage.clone();
        move |_, target| {
            // Borrowed only by this handler and by the waiting task, and never across an await.
            if let Ok(mut stage) = stage.try_borrow_mut() {
                step(&mut stage, target, &heard, &main);
            }
        }
    });
    let done = loop {
        desktop.window.request_redraw();
        tokio::time::sleep(TICK).await;
        let Ok(mut stage) = stage.try_borrow_mut() else {
            continue;
        };
        if let Stage::Done(done) = &*stage {
            let done = done.clone();
            *stage = Stage::Gone;
            break done;
        }
    };
    desktop.remove_wry_event_handler(handler);
    BUSY.with(|busy| busy.set(false));
    done
}

/// One step of the window's life, on an event from the loop.
fn step<T: 'static>(
    stage: &mut Stage,
    target: &EventLoopWindowTarget<T>,
    heard: &Rc<Heard>,
    main: &Arc<Window>,
) {
    let next = match std::mem::replace(stage, Stage::Gone) {
        Stage::Asked { html, title } => match open(target, html, &title, heard, main) {
            Ok(sheet) => Stage::Open {
                sheet: Box::new(sheet),
                phase: Phase::Loading,
                since: Instant::now(),
            },
            Err(why) => Stage::Done(Err(why)),
        },
        Stage::Open {
            mut sheet,
            phase,
            since,
        } => match decide(phase, heard, since.elapsed(), PRINT_LIMIT) {
            Move::Wait => Stage::Open {
                sheet,
                phase,
                since,
            },
            Move::Print => match dialog(&mut sheet, heard, main) {
                Answer::Print => Stage::Open {
                    sheet,
                    phase: Phase::Printing,
                    since: Instant::now(),
                },
                // Dropping the sheet closes the window.
                Answer::Cancel => Stage::Done(Ok(())),
            },
            Move::Close(done) => Stage::Done(done),
        },
        other => other,
    };
    *stage = next;
}

/// The window, with the webview loading `html`.
fn open<T: 'static>(
    target: &EventLoopWindowTarget<T>,
    html: String,
    title: &str,
    heard: &Rc<Heard>,
    main: &Arc<Window>,
) -> Result<Sheet, String> {
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(820.0, 960.0))
        .build(target)
        .map_err(|e| format!("The print window did not open: {e}"))?;
    let loaded = heard.clone();
    let poke = main.clone();
    let builder = wry::WebViewBuilder::new()
        .with_html(html)
        .with_javascript_disabled()
        .with_devtools(false)
        .with_clipboard(false)
        .with_incognito(true)
        // The document itself, and a jump within it. A link in a message goes nowhere from here.
        .with_navigation_handler(|uri| uri.starts_with("about:blank"))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, wry::PageLoadEvent::Finished) {
                loaded.loaded.set(true);
                // A load is not an event of the loop's; this makes one, so the handler runs.
                poke.request_redraw();
            }
        });
    #[cfg(target_os = "linux")]
    let webview = {
        use dioxus::desktop::tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or_else(|| "The print window has nowhere to put the page.".to_owned())?;
        builder.build_gtk(vbox)
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    let webview = webview.map_err(|e| format!("The print window could not show the page: {e}"))?;
    Ok(Sheet {
        window,
        webview,
        #[cfg(target_os = "linux")]
        operation: None,
    })
}

/// How the dialog was answered.
enum Answer {
    Print,
    Cancel,
}

/// The print dialog, over the print window. Returns once it is answered.
#[cfg(target_os = "linux")]
fn dialog(sheet: &mut Sheet, heard: &Rc<Heard>, main: &Arc<Window>) -> Answer {
    use dioxus::desktop::tao::platform::unix::WindowExtUnix;
    use webkit2gtk::PrintOperationExt;
    use wry::WebViewExtUnix;
    let operation = webkit2gtk::PrintOperation::new(&sheet.webview.webview());
    operation.connect_finished({
        let heard = heard.clone();
        let poke = main.clone();
        move |_| {
            heard.finished.set(true);
            poke.request_redraw();
        }
    });
    operation.connect_failed({
        let heard = heard.clone();
        move |_, error| {
            heard.failed.replace(Some(error.to_string()));
        }
    });
    let answer = operation.run_dialog(Some(sheet.window.gtk_window()));
    sheet.operation = Some(operation);
    match answer {
        webkit2gtk::PrintOperationResponse::Print => Answer::Print,
        _ => Answer::Cancel,
    }
}

#[cfg(not(target_os = "linux"))]
fn dialog(sheet: &mut Sheet, _heard: &Rc<Heard>, _main: &Arc<Window>) -> Answer {
    match sheet.webview.print() {
        Ok(()) => Answer::Print,
        Err(_) => Answer::Cancel,
    }
}
