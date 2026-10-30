#!/usr/bin/env bash
# Every library crate builds on its own with nothing switched on, and mail-crypto with each of its
# halves alone. A headless or third-party user names the features it wants, so none of them
# may be needed by accident.
#
# One invocation per crate, deliberately: with `--workspace` cargo unifies features across the
# members, and `mail-app` switches on everything, so a crate that quietly needs a feature of its
# own would still build. Cargo compiles; this is the `nodef` stage of the gate.
set -euo pipefail
cd "$(dirname "$0")/.."

for crate in mailo-domain mailo-mime mailo-crypto mailo-archive mailo-pim mailo-editor mail-store mailo-proto mail-net mail-secrets mail-runtime mail-core mail-appkit; do
  echo "nodef: $crate"
  cargo check -p "$crate" --no-default-features
done

# Each half of mail-crypto alone, tests included (they are gated by the same features).
for feature in openpgp smime; do
  echo "nodef: mail-crypto --features $feature"
  cargo check -p mailo-crypto --no-default-features --features "$feature" --tests
done
echo "nodef: mail-crypto"
cargo check -p mailo-crypto --no-default-features --tests
echo "nodef: mail-mime"
cargo check -p mailo-mime --no-default-features --tests

# Each crate's test doubles (`testing`) build with nothing else on: the secrets' and runtime's without the
# platform keyring and the bus, and the others' without whatever the workspace turns on for them.
for crate in mailo-domain mail-store mail-net mail-secrets mail-runtime mail-core; do
  echo "nodef: $crate --features testing"
  cargo check -p "$crate" --no-default-features --features testing
done
