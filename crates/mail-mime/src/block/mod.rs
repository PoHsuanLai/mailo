//! A message body as blocks a reader can draw.
//!
//! HTML is tokenized with the same html5ever ammonia used to serialize it.
//! There is no DOM and no `Block::Raw`: the only markup that leaves this
//! module is what these types can say.

mod heaviness;
mod html;
mod image;
mod kind;
mod limits;
mod machine;
mod parse;
mod quote;
mod text;
mod tracking;
mod url;

pub use html::{is_mapped, mapped_tags};
pub use kind::{Action, Block, Dir, Document, Flowed, ImgSrc, Inlined, Shape, Span};
pub use limits::{Limits, Reached};
pub use parse::{from_html, from_text};
pub(crate) use parse::{from_html_describing, from_text_keeping_lines};
pub use url::{LINK_REL, LINK_TARGET, SafeUrl};
