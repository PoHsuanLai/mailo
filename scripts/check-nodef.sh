#!/usr/bin/env bash
# Every library crate builds on its own with nothing switched on, and mail-mime with each of its
# crypto halves alone. A headless or third-party user names the features it wants, so none of them
# may be needed by accident.
#
# One invocation per crate, deliberately: with `--workspace` cargo unifies features across the
# members, and `mail-app` switches on everything, so a crate that quietly needs a feature of its
# own would still build. Cargo compiles; this is the `nodef` stage of the gate.
set -euo pipefail
cd "$(dirname "$0")/.."

for crate in mail-domain mail-mime mail-pim mail-store mail-proto mail-runtime mail-core; do
  echo "nodef: $crate"
  cargo check -p "$crate" --no-default-features
done

# Each half of mail-mime alone, tests included (they are gated by the same features).
for feature in openpgp smime; do
  echo "nodef: mail-mime --features $feature"
  cargo check -p mail-mime --no-default-features --features "$feature" --tests
done
echo "nodef: mail-mime"
cargo check -p mail-mime --no-default-features --tests

# The doubles at the runtime's seams build without the platform keyring and the bus.
echo "nodef: mail-runtime --features testing"
cargo check -p mail-runtime --no-default-features --features testing
