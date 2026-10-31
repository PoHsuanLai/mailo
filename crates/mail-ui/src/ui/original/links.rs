//! A link inside the Original frame on Blitz: quire's `FrameLinks::intercept(..).with_hover(..)`.
//!
//! The frame never navigates (ds-native gives every frame document a navigation provider that
//! only reports the click), so a stranger's page is never loaded into the reader. The click
//! reaches mailo, which opens the address in the system browser, the way a link in the Reader
//! view opens: `http`, `https` and `mailto` only, through the same check the Reader view's links
//! are built with ([`mail_reader::SafeUrl::link`]: no other scheme, no control or bidi characters,
//! and without the query parameters that only tell the sender who clicked). Anything else
//! (`file:`, `javascript:`, a `data:` document) does nothing.
//!
//! The pointer coming onto a link in a frame, or leaving it, is reported once per crossing with
//! the anchor's own text and where it goes. mailo reads the two as it reads a Reader view link
//! ([`mail_logic::trust::destination`]: the pill is loud when the text names somewhere else), so the
//! reader's link pill shows for a frame's links too ([`FramePill`]).
//!
//! A context menu asked over a link in a frame (a right click) is reported just before the
//! reader's own `oncontextmenu` for the same press, with where the link goes and where the
//! pointer is. It is kept, checked as a click is, until the reader takes it for its Copy Link
//! menu ([`FrameMenus`]).
//!
//! A click on an image in a frame (not inside a link, which is the link's click) is reported with
//! the frame's tag and the `src` the image was drawn from. The reader that drew the frame opens
//! it in Peek when it is one of that message's own parts ([`FrameImages`]).

use ds::prelude::Point;
use ds_blitz::{FrameImageClick, FrameLink, FrameLinkHover, FrameLinkMenu, FrameLinks, HoverPhase};
use mail_reader::SafeUrl;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::watch;

/// Whatever opens a link: the system browser in the window, a recorder in a test.
pub trait Browse: Send + Sync + 'static {
    /// Open `url`, already checked.
    fn open(&self, url: &str);
}

/// The system browser, as the Add account sheet's sign-in opens it.
pub(crate) struct System;

impl Browse for System {
    fn open(&self, url: &str) {
        // Best effort, as a click on a link in a browser is: nothing waits on it.
        let _ = webbrowser::open(url);
    }
}

/// A link in a frame the pointer is on: what its anchor says and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pointed {
    pub(crate) text: String,
    pub(crate) href: String,
}

/// The link under the pointer in any Original frame, as the window's root context. quire reports
/// a crossing on the UI thread, outside any component; the reader's pill hears it here
/// (`ui/hover`). Cloning shares it.
#[derive(Clone)]
pub struct FramePill(watch::Receiver<Option<Pointed>>);

impl std::fmt::Debug for FramePill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FramePill")
    }
}

impl FramePill {
    /// A pill no link is under yet, and the sender quire's hover handler writes to.
    pub(crate) fn new() -> (FramePill, watch::Sender<Option<Pointed>>) {
        let (sender, receiver) = watch::channel(None);
        (FramePill(receiver), sender)
    }

    /// Wait for the next crossing and return the link the pointer is on after it, if any; `None`
    /// once the frames' reporter is gone for good.
    pub(crate) async fn next(&mut self) -> Option<Option<Pointed>> {
        self.0.changed().await.ok()?;
        Some(self.0.borrow_and_update().clone())
    }
}

/// A link a context menu was asked over in a frame: where it goes, as a click on it would open
/// it ([`SafeUrl::link`]), and where the pointer is, in the window's coordinates.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LinkAsked {
    pub(crate) href: String,
    /// What the anchor said (its title when it has no text): what Preview Link's card is titled.
    pub(crate) text: String,
    pub(crate) at: Point,
}

/// The last context menu asked over a link in any Original frame, until the reader's own
/// `oncontextmenu` for the same press takes it, as the window's root context. quire reports it
/// on the UI thread, outside any component. Taken, not read: a right click that lands on no link
/// reports nothing, and must not find the last link's menu still waiting. Cloning shares it.
#[derive(Clone)]
pub struct FrameMenus {
    asked: Arc<Mutex<Option<LinkAsked>>>,
    browse: Arc<dyn Browse>,
}

impl std::fmt::Debug for FrameMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FrameMenus")
    }
}

impl FrameMenus {
    /// Menus whose Open Link `browse` opens, as a click on the link would.
    pub(crate) fn new(browse: Arc<dyn Browse>) -> Self {
        FrameMenus {
            asked: Arc::default(),
            browse,
        }
    }

    /// The link the press that is opening a context menu was on, if any.
    pub(crate) fn take(&self) -> Option<LinkAsked> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// A menu asked over `menu`'s link: kept when a click could follow it, else forgotten (a
    /// `javascript:` or `file:` link has nothing a reader may copy or open).
    fn ask(&self, menu: FrameLinkMenu) {
        let text = if menu.text.trim().is_empty() {
            menu.title.clone().unwrap_or_default()
        } else {
            menu.text.clone()
        };
        let asked = SafeUrl::link(&menu.href).map(|url| LinkAsked {
            href: url.as_str().to_owned(),
            text,
            at: menu.at,
        });
        *self.asked.lock().unwrap_or_else(PoisonError::into_inner) = asked;
    }

    /// Open Link: `href`, already checked when the menu was asked, opened as a click opens it.
    pub(crate) fn open(&self, href: &str) {
        self.browse.open(href);
    }
}

/// An image clicked in a frame: the tag of the frame it is in (`Holder::tag`, written by mailo
/// on the `iframe`) and the `src` it was drawn from, resolved as the frame resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageClicked {
    pub(crate) tag: String,
    pub(crate) src: String,
}

/// The last image clicked in any Original frame, as the window's root context. quire reports the
/// click on the UI thread, outside any component; the reader that drew the frame hears it here
/// and opens the image in Peek (`ui/reading`). Cloning shares it.
#[derive(Clone)]
pub struct FrameImages(watch::Receiver<Option<ImageClicked>>);

impl std::fmt::Debug for FrameImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FrameImages")
    }
}

impl FrameImages {
    /// Nothing clicked yet, and the sender quire's image handler writes to.
    pub(crate) fn new() -> (FrameImages, watch::Sender<Option<ImageClicked>>) {
        let (sender, receiver) = watch::channel(None);
        (FrameImages(receiver), sender)
    }

    /// A listener that hears only the clicks after now: a reader that mounts after a click
    /// must not take it for its own.
    pub(crate) fn listen(&self) -> FrameImages {
        let mut receiver = self.0.clone();
        receiver.mark_unchanged();
        FrameImages(receiver)
    }

    /// Wait for the next click on an image; `None` once the frames' reporter is gone for good.
    pub(crate) async fn next(&mut self) -> Option<ImageClicked> {
        loop {
            self.0.changed().await.ok()?;
            if let Some(clicked) = self.0.borrow_and_update().clone() {
                return Some(clicked);
            }
        }
    }
}

/// What a click in any frame does: `browse` opens it, if it is a link a reader may follow. A
/// crossing goes to `pill`: `Enter` names the link, `Leave` clears it. A context menu over a link
/// goes to `menus`, and a click on an image in a frame that mailo tagged, to `images`.
pub(crate) fn frame_links(
    browse: Arc<dyn Browse>,
    pill: watch::Sender<Option<Pointed>>,
    menus: FrameMenus,
    images: watch::Sender<Option<ImageClicked>>,
) -> FrameLinks {
    FrameLinks::intercept(move |link: FrameLink| {
        if let Some(url) = SafeUrl::link(&link.href) {
            browse.open(url.as_str());
        }
    })
    .with_hover(move |hover: FrameLinkHover| {
        let pointed = match hover.phase {
            HoverPhase::Enter => Some(Pointed {
                text: hover.text,
                // Where a click would go: the pill names the address `browse` would open.
                href: SafeUrl::link(&hover.href).map_or(hover.href, |url| url.as_str().to_owned()),
            }),
            HoverPhase::Leave => None,
        };
        pill.send_replace(pointed);
    })
    .with_context_menu(move |menu: FrameLinkMenu| menus.ask(menu))
    .with_image_click(move |click: FrameImageClick| {
        if let Some(tag) = click.tag {
            images.send_replace(Some(ImageClicked {
                tag: tag.as_str().to_owned(),
                src: click.src,
            }));
        }
    })
}
