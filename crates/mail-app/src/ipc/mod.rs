//! The mail daemon's door.
//!
//! `mailo watch` already holds connections open for as long as the process lives. A daemon is
//! that loop with a door in it, so the command line, the window and anything else become clients
//! of one sync engine rather than three programs opening three sets of sockets to the same
//! servers.
//!
//! # What is here, and what is not
//!
//! Where the door is, who is allowed to be behind it, and how a client starts one are all in
//! [`latchkey`], because none of that is about mail. That separation was made the day the second
//! race was found in it: deciding "is an agent already running" correctly is a problem with
//! exactly one right answer and several plausible wrong ones, and it does not belong in a mail
//! client where it is read by nobody looking for it.
//!
//! What is here is the conversation — [`wire`], and the two ends that speak it. That part *is*
//! about mail, and is the part no library could have chosen for us.
//!
//! # Why not a TCP port on loopback
//!
//! Because anything on the machine could connect to it, and this repository already knows what
//! that costs: `mail-runtime/src/loopback.rs` validates the OAuth `state` parameter precisely
//! because "a loopback listener is reachable by anything on the machine". A socket in a
//! directory only this user can read needs no such argument — the filesystem is the
//! authentication, which is why `latchkey` makes that directory `0700`.

pub mod client;
pub mod daemon;
pub mod wire;

/// This user's mailo daemon, wherever this platform puts such a thing.
///
/// Short name on purpose: on macOS the whole socket path has 104 bytes to fit in, and `TMPDIR`
/// has already spent forty of them before anything of ours is appended.
pub fn agent() -> Result<latchkey::Agent, String> {
    latchkey::Agent::new("mailo").map_err(|e| e.to_string())
}
