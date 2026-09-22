# latchkey

Find this user's background agent, or start one. One logic, three platforms.

A *latchkey* is the key to your own front door: it lets you in, and it tells you whether anyone
is already home.

```rust
let agent = latchkey::Agent::new("mailo")?;

// In the agent process:
let door = agent.listen()?;              // or Err(Error::AlreadyRunning)
for client in door.incoming() {
    let mut client = client?;            // Read + Write, and that is all you need to know
}

// In the client process:
let mut agent = agent.connect_or_start(
    || latchkey::spawn(&["daemon"]),     // how to launch one; yours to decide
    Duration::from_secs(5),
)?;
```

## What it does

The transport half of this problem is solved. [`interprocess`] already unifies Unix domain
sockets and Windows named pipes behind one `Read + Write` stream, and this crate is built on it
rather than beside it.

What is *not* solved anywhere, and what everyone who needs it writes again, is the lifecycle
around that stream:

- **Where does the socket go**, per platform, without exceeding `sun_path`?
- **Is one already running**, and can that be answered without a race?
- **Who starts it**, and how long does a client wait?
- **What is left behind** when it is killed rather than asked to stop?

Those four are what this crate decides, and nothing else. There is no message framing, no
protocol version, no request type: those differ per application, and a crate that chose them for
you would be one you fought.

## Guarantees

- **One agent per user**, enforced by an advisory file lock rather than by looking at the socket.
- **A killed agent locks nobody out.** The kernel releases the lock with the process — under
  `SIGKILL`, under a power cut — so there is no stale state to reap and no timeout after which a
  lock is assumed dead.
- **A client never deletes anything.** Knocking and tidying are different jobs.
- **Every platform rule is a pure function**, so the macOS and Windows answers are tested from
  whatever machine runs `cargo test`.

## What it cannot hide

One platform difference survives the abstraction, and pretending otherwise would be worse than
naming it: **a busy agent refuses on Windows and queues on Unix.**

A Unix domain socket holds pending connections in a backlog, so a client that knocks while the
agent is mid-conversation simply waits its turn and notices nothing. A Windows named pipe serves
one client per instance and turns the rest away with `ERROR_PIPE_BUSY`. `connect` therefore has a
third answer besides "reached" and "nobody home": [`Error::Busy`], which means somebody *is* home
and cannot come to the door. `connect_or_start` treats it as proof of life and keeps knocking,
because starting an agent then would be starting a rival to one that is demonstrably alive.

The knock is bounded for the same reason. `interprocess` defaults to unbounded waiting, which on
Windows is `WaitNamedPipeW(NMPWAIT_WAIT_FOREVER)` — so the default "is anyone home?" can block
for ever against a busy agent, with nothing to cancel it. This crate always passes a timeout.

And one limitation that is not papered over: **on Windows an agent started by `spawn` inherits
the starting client's handles.** `CreateProcess` is called with `bInheritHandles: TRUE`, because
the standard library offers no stable way to say otherwise, and as std's own source puts it,
"once an inheritable handle is created, *any* spawned child will inherit that handle". Setting
the agent's stdio to null does not help — the handles at issue belong to the client.

So if the client's stdout is a pipe someone reads to end-of-file — `$(mytool status)` in a
shell, a CI step capturing output — that read waits for the *agent*, which is designed to live
for hours. Have the client write to a file or the terminal instead, or supply your own `start`
closure: `connect_or_start` takes one precisely so `spawn` is a convenience and not a
constraint. Fixing it inside the crate needs one `unsafe` call to `SetHandleInformation`, and
this crate compiles under `unsafe_code = "forbid"`.

## Why a lock and not a look

The obvious way to find out whether an agent is running is to look at its socket: is the file
there, and does anything answer on it? That reads correctly and is wrong under concurrency, in
two ways that are easy to write and hard to see.

*A client that tidies races every other client.* Between a failed connect and the `remove_file`
it does on the strength of it, another client's agent can bind — and the tidying client then
deletes a working agent's door. Two people running the same command at the same moment on a cold
machine is all it takes.

*An agent that clears races every other agent.* Connect, fail, remove, bind: four steps, and two
agents can interleave at every one of them. Both clear, both bind, and the second unlinks the
first's socket out from under it. The first is then listening on an inode nothing can reach —
invisible, holding no clients, and never exiting.

Both come from the same mistake: inferring liveness from a file. A socket that refuses
connections only *suggests* nobody is home. An advisory lock *is* the answer, because the kernel
does the arbitration.

## Where it looks

| | endpoint | lock |
|---|---|---|
| Linux | `$XDG_RUNTIME_DIR/<name>/agent.sock`, else `$HOME/.cache/…` | beside it |
| macOS | `$TMPDIR/<name>/agent.sock`, else `~/Library/Caches/…` | beside it |
| Windows | a named pipe, `<name>-<user>` | `%LOCALAPPDATA%\<name>\agent.lock` |

macOS gets `TMPDIR` because it has no `XDG_RUNTIME_DIR` and its `TMPDIR` is per-user and private;
`/tmp` would put the socket where every other account can see it. Windows gets the user in the
pipe name because that namespace is machine-wide, and without it two people signed in to one box
would be one agent.

`sun_path` is 108 bytes on Linux and 104 on macOS. The limit is not advisory — a longer path is
silently truncated by some libcs and refused by others — so exceeding it is an error with all
three numbers in it rather than a socket bound somewhere nobody asked for.

## Why not just use systemd

You should, where you can. systemd socket activation and launchd's `Sockets` key are both better
than this: the init system binds the socket at login and starts your process on the first
connection, so there is no stale socket, no spawn race and no timeout to pick.

The catch is that they are three different answers and the third does not exist. A tool installed
by `cargo install`, by `brew`, or by a shell script has no installation step in which to write a
unit file, and has to work anyway — in a container, over ssh, on a colleague's laptop, on the Mac
someone tries it on next. This crate is that case: demand-start, nothing enabled, and a per-user
agent that a unit file can later be layered *on top of*, to start at login rather than to work at
all.

## Licence

MIT OR Apache-2.0.

[`interprocess`]: https://crates.io/crates/interprocess
