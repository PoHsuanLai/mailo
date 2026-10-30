# mailo-crypto

OpenPGP and S/MIME for email, and the Verified Mark Certificate check behind BIMI logos. Bytes
in, values out: nothing here opens a socket, reads a file or asks the clock; the random number
generator and every instant are arguments.

A message is built or parsed with [`mailo-mime`](https://crates.io/crates/mailo-mime); this crate
signs, seals, verifies and opens the bytes it made, as PGP/MIME or as S/MIME.

## Features

- `openpgp`: signing, verifying, encrypting and decrypting with OpenPGP (rPGP), and Web Key
  Directory addresses.
- `smime`: the same with S/MIME (CMS), reading PKCS#12 identities, and the mark certificate
  check (`vmc`). Brings `openpgp` in.
- `testing`: throwaway certificate authorities for tests.

All are off by default.

Part of [mailo](https://github.com/PoHsuanLai/mailo). Licensed under MIT or Apache-2.0, at your
option.
