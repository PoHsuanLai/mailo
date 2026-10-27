//! Raw DEFLATE both ways over one connection, as IMAP `COMPRESS DEFLATE` (RFC 4978) asks.
//!
//! Each write is compressed and flushed to a byte boundary (`Z_SYNC_FLUSH`) so the server can
//! read the command at once; what the server sends is inflated as it arrives, however it was cut
//! into reads. The two directions are two independent streams that keep their state for the life
//! of the connection.

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use std::io;

/// The two streams of one compressed connection.
pub(crate) struct Deflate {
    out: Compress,
    inn: Decompress,
}

impl std::fmt::Debug for Deflate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Deflate")
    }
}

impl Deflate {
    /// Raw streams, with no zlib header: RFC 4978 §3 names DEFLATE (RFC 1951) itself.
    pub(crate) fn new() -> Self {
        Self {
            out: Compress::new(Compression::default(), false),
            inn: Decompress::new(false),
        }
    }

    /// `data` as the bytes to send, flushed so the receiver can decode all of it.
    pub(crate) fn compress(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(data.len() / 2 + 64);
        let mut taken = 0;
        loop {
            let before = self.out.total_in();
            self.out
                .compress_vec(&data[taken..], &mut out, FlushCompress::Sync)
                .map_err(io::Error::other)?;
            taken += usize::try_from(self.out.total_in() - before).unwrap_or(0);
            // Room left over means the flush finished; none means it may have more to say.
            if taken >= data.len() && out.len() < out.capacity() {
                return Ok(out);
            }
            out.reserve(out.capacity().max(256));
        }
    }

    /// What `raw`, just read from the connection, decompresses to. Empty while a block is still
    /// partly in flight.
    pub(crate) fn inflate(&mut self, raw: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(raw.len() * 3 + 64);
        let mut taken = 0;
        loop {
            let before = self.inn.total_in();
            let status = self
                .inn
                .decompress_vec(&raw[taken..], &mut out, FlushDecompress::Sync)
                .map_err(io::Error::other)?;
            taken += usize::try_from(self.inn.total_in() - before).unwrap_or(0);
            if matches!(status, Status::StreamEnd) {
                return Err(io::Error::other("the compressed stream ended"));
            }
            if taken >= raw.len() && out.len() < out.capacity() {
                return Ok(out);
            }
            out.reserve(out.capacity().max(256));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client and the server it talks to, each with its own pair of streams.
    #[test]
    fn what_one_side_writes_the_other_reads_across_any_cut() {
        let mut client = Deflate::new();
        let mut server = Deflate::new();
        let lines: Vec<Vec<u8>> = vec![
            b"a001 NOOP\r\n".to_vec(),
            b"a002 UID FETCH 1:* (UID FLAGS)\r\n".to_vec(),
            // Large and repetitive, so it spans many output buffers.
            "* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n"
                .repeat(5_000)
                .into_bytes(),
            Vec::new(),
            b"a003 LOGOUT\r\n".to_vec(),
        ];
        for line in lines {
            let wire = client.compress(&line).unwrap();
            // Cut where a socket might: a byte at a time for a short one, in thirds for the rest.
            let step = if wire.len() < 64 {
                1
            } else {
                wire.len() / 3 + 1
            };
            let mut heard = Vec::new();
            for piece in wire.chunks(step) {
                heard.extend(server.inflate(piece).unwrap());
            }
            assert_eq!(heard, line);
        }
    }

    #[test]
    fn bytes_that_are_not_deflate_are_an_error() {
        let mut server = Deflate::new();
        assert!(server.inflate(&[0xff; 32]).is_err());
    }
}
