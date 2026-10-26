//! Attachments through anyview: the stored bytes read as a file, and the peek of them.
//!
//! anyview looks at a file through `anyview_core::ReadAt`, bytes read at an offset. mailo's bytes
//! are in the client's blobs, which can be read whole or from the front, so [`BlobBytes`] stands
//! between: the head of a part (what sniffing reads) comes from the client's `head` and costs a few
//! KiB, and a read past it loads the part once, within the peek's byte budget, and serves every
//! later read from memory. Nothing is written to a file.
//!
//! The peek runs on anyview's worker, one thread that looks at the latest file asked for and
//! abandons one that overruns. The strip asks for every stored part of a message at once and the
//! worker keeps the latest request only, so [`look`] takes its turn first: one part is asked for at
//! a time, and none is replaced.

use std::io::{Error, ErrorKind, Result};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use anyview_core::{ByteLen, FileName, FileStamp, Input, ModTime, PeekBudget, PixelArea, ReadAt};
use anyview_peek::{AnyPeeked, PeekWorker, Peeking, WorkerConfig};
use mail_client::Client;
use mail_domain::{BlobId, MessageId};

use super::route::{self, Hand};

/// What a peek may spend: 32 MiB read (a part over it is not peeked whole), a megapixel decoded, and
/// half a second. A card is a few hundred pixels across; a photograph from a phone is the largest
/// thing a message carries.
pub(super) const BUDGET: PeekBudget = PeekBudget::DEFAULT
    .with_bytes(ByteLen(32 << 20))
    .with_pixels(PixelArea(1_000_000))
    .with_time(Duration::from_millis(500));

/// How long [`look`] waits for the worker: past the worker's own four seconds, so only a worker
/// that never started (the system refused it a thread) reaches it.
const WAIT: Duration = Duration::from_secs(8);

/// How much of the front of a part is read without loading the rest: well past the 4 KiB that
/// sniffing reads.
const FRONT: u64 = 8 * 1024;

/// A stored part as anyview reads a file.
pub(super) struct BlobBytes {
    client: Client,
    blob: BlobId,
    len: u64,
    /// The most that is loaded whole.
    limit: u64,
    /// The whole part, once something read past [`FRONT`].
    whole: Mutex<Option<Arc<[u8]>>>,
}

impl std::fmt::Debug for BlobBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobBytes")
            .field("blob", &self.blob)
            .field("len", &self.len)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl BlobBytes {
    /// `blob`, `len` bytes long, loaded whole only if it is no longer than `limit`.
    pub(super) fn new(client: Client, blob: BlobId, len: u64, limit: u64) -> Self {
        BlobBytes {
            client,
            blob,
            len,
            limit,
            whole: Mutex::new(None),
        }
    }

    /// The whole part, loaded on first need.
    fn loaded(&self) -> Result<Arc<[u8]>> {
        let mut whole = self.whole.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(bytes) = whole.as_ref() {
            return Ok(Arc::clone(bytes));
        }
        if self.len > self.limit {
            return Err(Error::new(
                ErrorKind::FileTooLarge,
                crate::ui::copy::status::TOO_LARGE_TO_PEEK,
            ));
        }
        let bytes: Arc<[u8]> = self
            .client
            .blobs()
            .get(self.blob)
            .map_err(Error::other)?
            .into();
        *whole = Some(Arc::clone(&bytes));
        Ok(bytes)
    }

    fn cached(&self) -> Option<Arc<[u8]>> {
        self.whole
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ReadAt for BlobBytes {
    fn len(&self) -> ByteLen {
        ByteLen(self.len)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() || offset >= self.len {
            return Ok(0);
        }
        if let Some(whole) = self.cached() {
            return Ok(copy_from(&whole, offset, buf));
        }
        let end = offset.saturating_add(buf.len() as u64);
        if end <= FRONT {
            // `FRONT` is a few KiB: it fits a usize.
            let front = self
                .client
                .blobs()
                .head(self.blob, FRONT as usize)
                .map_err(Error::other)?;
            return Ok(copy_from(&front, offset, buf));
        }
        Ok(copy_from(&self.loaded()?, offset, buf))
    }
}

/// Copies into `buf` the bytes of `from` at `offset`, as many as there are; none past the end.
fn copy_from(from: &[u8], offset: u64, buf: &mut [u8]) -> usize {
    let Some(rest) = usize::try_from(offset)
        .ok()
        .and_then(|start| from.get(start..))
    else {
        return 0;
    };
    let count = rest.len().min(buf.len());
    buf[..count].copy_from_slice(&rest[..count]);
    count
}

/// The one worker, and the turn that keeps its slot from being replaced.
struct Peeker {
    worker: PeekWorker<AnyPeeked>,
    turn: Mutex<()>,
}

fn peeker() -> &'static Peeker {
    static PEEKER: OnceLock<Peeker> = OnceLock::new();
    PEEKER.get_or_init(|| Peeker {
        worker: PeekWorker::looking(
            WorkerConfig::default().with_name("mailo-peek"),
            Peeking::pane().with_budget(BUDGET),
        ),
        turn: Mutex::new(()),
    })
}

/// The peek of `input` on anyview's worker. Blocking: call it from a blocking thread. `None` when
/// the worker gave no answer in time.
pub(super) fn look(input: Input) -> Option<Arc<AnyPeeked>> {
    let peeker = peeker();
    let _turn = peeker.turn.lock().unwrap_or_else(PoisonError::into_inner);
    let (reply, answer) = mpsc::channel();
    peeker.worker.ask(input, move |peeked| {
        // The asker gave up after `WAIT`: the answer goes nowhere.
        let _ = reply.send(peeked);
    });
    answer.recv_timeout(WAIT).ok().map(Arc::new)
}

/// What anyview made of a stored part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Seen {
    /// The card to draw.
    Ready(Arc<AnyPeeked>),
    /// Still on the server: nothing is fetched to peek at it.
    NotHere,
    /// mailo's to draw, not anyview's (`route::hand`).
    Own,
    /// No card, and why.
    Failed(String),
}

/// Attachment `index` of `message`, peeked at by anyview. Blocking: reads the client.
pub(super) fn see(client: &Client, message: MessageId, index: usize) -> Seen {
    let message = match client.stored().message(message) {
        Ok(message) => message,
        Err(why) => return unreadable(why),
    };
    let Some(attachment) = message.attachments.get(index) else {
        return Seen::Failed(crate::ui::copy::status::no_attachment(index));
    };
    let Some(blob) = attachment.blob() else {
        return Seen::NotHere;
    };
    let blobs = client.blobs();
    let head = match blobs.head(blob, route::HEAD) {
        Ok(head) => head,
        Err(why) => return unreadable(why),
    };
    if route::hand(&attachment.name, &head) == Hand::Mailo {
        return Seen::Own;
    }
    let len = match blobs.size(blob) {
        Ok(len) => len,
        Err(why) => return unreadable(why),
    };
    let input = Input::new(
        file_name(&attachment.name),
        FileStamp {
            len: ByteLen(len),
            // The message's date stands for the part's: it has no clock of its own.
            modified: ModTime(message.date.timestamp_nanos_opt().unwrap_or(0)),
        },
        Arc::new(BlobBytes::new(client.clone(), blob, len, BUDGET.bytes.0)),
    );
    match look(input) {
        Some(peeked) => Seen::Ready(peeked),
        None => Seen::Failed(crate::ui::copy::status::TOOK_TOO_LONG.to_owned()),
    }
}

/// A part the client could not read: the reason goes to the log, and the card says only that.
fn unreadable(why: impl std::fmt::Display) -> Seen {
    log::warn!("an attachment could not be read for anyview: {why}");
    Seen::Failed(crate::ui::copy::status::UNREADABLE.to_owned())
}

/// The name anyview sniffs by: the sender's claim made safe to write, or "file" when nothing is left.
fn file_name(claimed: &str) -> FileName {
    FileName::new(&mail_core::attach::safe_name(claimed)).unwrap_or_else(|_| FileName::unnamed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyview_core::FormatKind;
    use mail_core::SqliteStore;

    fn store(dir: &std::path::Path) -> Arc<SqliteStore> {
        std::fs::create_dir_all(dir.join("blobs")).unwrap();
        Arc::new(SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap())
    }

    /// Bytes that differ from one position to the next.
    fn counting(len: usize) -> Vec<u8> {
        (0..len).map(|n| (n % 251) as u8).collect()
    }

    fn bytes_of(store: &Arc<SqliteStore>, data: &[u8], limit: u64) -> BlobBytes {
        let blob = store.blobs().put(data).unwrap();
        BlobBytes::new(
            crate::ui::fixtures::client(store),
            blob,
            data.len() as u64,
            limit,
        )
    }

    #[test]
    fn a_part_reads_as_the_bytes_that_were_stored_at_any_offset() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        // One in the database, one in a file (the store keeps 32 KiB and under inline), and one
        // shorter than the front that is read without loading the rest.
        for len in [100, 20_000, 70_000] {
            let data = counting(len);
            let part = bytes_of(&store, &data, 1 << 20);
            assert_eq!(part.len(), ByteLen(len as u64), "{len}");
            assert_eq!(
                part.read_range(0..len as u64).unwrap(),
                data,
                "{len}: whole"
            );
            assert_eq!(part.read_range(0..16).unwrap(), data[..16], "{len}: head");
            let middle = len / 2;
            let end = (middle + 50).min(len);
            assert_eq!(
                part.read_range(middle as u64..end as u64).unwrap(),
                data[middle..end],
                "{len}: middle"
            );
        }
    }

    #[test]
    fn reads_at_and_past_the_end_give_what_is_left_and_then_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let data = counting(70_000);
        let part = bytes_of(&store, &data, 1 << 20);
        let mut buf = [0u8; 64];
        assert_eq!(part.read_at(69_990, &mut buf).unwrap(), 10);
        assert_eq!(buf[..10], data[69_990..]);
        assert_eq!(part.read_at(70_000, &mut buf).unwrap(), 0);
        assert_eq!(part.read_at(u64::MAX, &mut buf).unwrap(), 0);
        assert_eq!(part.read_at(0, &mut []).unwrap(), 0);
    }

    #[test]
    fn the_front_is_read_without_loading_the_part_and_the_rest_only_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let data = counting(70_000);
        let part = bytes_of(&store, &data, 50_000);
        // What sniffing reads comes from the front, whatever the limit.
        assert_eq!(part.read_range(0..4096).unwrap(), data[..4096]);
        assert!(part.cached().is_none(), "a front read loaded the part");
        // Past it needs the whole, and the whole is over the limit.
        let refused = part.read_range(60_000..60_010).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::FileTooLarge);
        assert!(part.cached().is_none());
        // Within the limit it is loaded once and then served from memory.
        let part = bytes_of(&store, &data, 80_000);
        assert_eq!(
            part.read_range(60_000..60_010).unwrap(),
            data[60_000..60_010]
        );
        assert!(part.cached().is_some());
    }

    #[test]
    fn a_part_that_is_not_in_the_store_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let part = BlobBytes::new(
            crate::ui::fixtures::client(&store),
            BlobId::generate(),
            70_000,
            1 << 20,
        );
        assert!(part.read_range(0..16).is_err());
        assert!(part.read_range(60_000..60_010).is_err());
    }

    #[test]
    fn anyview_peeks_at_a_stored_picture_through_the_adapter() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let picture = image::RgbImage::from_pixel(64, 32, image::Rgb([30, 120, 200]));
        let mut png = Vec::new();
        picture
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let part = bytes_of(&store, &png, BUDGET.bytes.0);
        let input = Input::new(
            file_name("beach.png"),
            FileStamp {
                len: ByteLen(png.len() as u64),
                modified: ModTime(0),
            },
            Arc::new(part),
        );
        let peeked = look(input).expect("the worker answered");
        assert_eq!(peeked.kind, FormatKind::Raster);
        assert_eq!(peeked.name, "beach.png");
        assert!(peeked.still().is_some(), "{:?}", peeked.body);
    }
}
