#!/usr/bin/env bash
# mailo's [patch.crates-io] is quire's fork block, exactly: the vello, anyrender and dioxus forks
# quire builds its renderer with. A [patch] applies only at a workspace root, so every consumer
# that builds `ds-blitz` carries a copy, and a copy that drifts from quire's builds a renderer
# quire never tested.
#
# The block is checked against quire's own `docs/fork-patches.toml` at the rev mailo pins (the
# checkout cargo already fetched, found through `cargo metadata`), with quire's own
# `scripts/fork-patches.sh check`: never against a copy kept here.
set -euo pipefail
cd "$(dirname "$0")/.."

# `--locked`: a block edited without its lock (an entry dropped, a rev moved by hand) fails here.
if ! meta=$(cargo metadata --format-version 1 --locked 2>/dev/null); then
  echo "check-fork-patches: Cargo.lock does not match Cargo.toml; a [patch] entry changed without it?" >&2
  exit 1
fi
ds=$(printf '%s' "$meta" | python3 -c 'import json,sys
d=json.load(sys.stdin)
print(next(p["manifest_path"] for p in d["packages"] if p["name"]=="ds"))')
# crates/ds/Cargo.toml in quire's checkout: the checkout's root is two directories up.
quire=$(dirname "$(dirname "$(dirname "$ds")")")
if [ ! -x "$quire/scripts/fork-patches.sh" ]; then
  echo "check-fork-patches: quire at $quire has no scripts/fork-patches.sh (older than v0.3.0?)" >&2
  exit 1
fi
"$quire/scripts/fork-patches.sh" check "$PWD/Cargo.toml"
