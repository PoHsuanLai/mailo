# mailo-logic

The decisions of a mail client that need no I/O: how an instant is written for the person
reading it, what an undo reverses, how a typed search is read and ranked, what a filter means.
Values in, values out. Nothing here opens a file or a socket, reads a clock or the environment,
asks the store, or draws a window; a clock or a store is a value or a trait the caller passes in.

What the decisions are made over is `mailo-domain`; what acts on them, the sync, the store and
the network, is `mail-core` and the crates under it, which depend on this one.

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
