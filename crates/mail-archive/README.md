# mailo-archive

Mail at rest in files: mbox, Maildir naming, single `.eml` messages, and what Apple Mail
(`.emlx`), Thunderbird and Google Takeout leave on disk, read. Bytes in, values out: no
filesystem walk and no clock. An mbox is read from any `BufRead` and written to any `Write`, so a
multi-gigabyte archive streams through.

What a file says about where a message belonged and what had happened to it is reduced to one
`Placement`, whichever format said it.

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
