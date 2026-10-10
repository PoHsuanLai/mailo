#!/usr/bin/env bash
# The sans-I/O boundary, mechanically enforced.
#
# `cargo tree -i <dep>` exits 101 when the dependency is absent, which is precisely the state
# we want. Checking the exit status would therefore fail whenever the boundary holds, so we
# check for OUTPUT instead: any line naming the dependency is a leak.
set -uo pipefail

PURE=(mail-domain mail-mime mail-proto mail-pim)
# E6: nor the desktop's D-Bus link (porter's client and bus vocabulary, quire's capability probe):
# the sans-I/O crates know nothing of accountd.
FORBIDDEN=(tokio rusqlite dioxus reqwest keyring-core porter-client porter-dbus ds-desktop zbus)
# `mail-core` is the I/O-capable core: it may use tokio and the network, which the PURE
# crates may not. What it may never reach is anything that draws, the window's toolkit or the
# renderer under it, so that `mail-app`'s two front-ends (`ui`, `cli`) sit over one core that
# knows neither.
CORE=(mail-core)
FORBIDDEN_CORE=(dioxus ds ds-settings ds-blitz blitz-dom dioxus-native-dom)
fail=0

for crate in "${PURE[@]}"; do
  for dep in "${FORBIDDEN[@]}"; do
    if cargo tree -p "$crate" -i "$dep" 2>/dev/null | grep -q .; then
      echo "LEAK: $crate depends on $dep"
      cargo tree -p "$crate" -i "$dep" 2>/dev/null | head -20
      fail=1
    fi
  done
done

if [ "$fail" -eq 0 ]; then
  echo "sans-I/O boundary holds: ${PURE[*]} reach none of ${FORBIDDEN[*]}"
fi

core_fail=0
for crate in "${CORE[@]}"; do
  for dep in "${FORBIDDEN_CORE[@]}"; do
    if cargo tree -p "$crate" -i "$dep" 2>/dev/null | grep -q .; then
      echo "LEAK: $crate depends on $dep"
      cargo tree -p "$crate" -i "$dep" 2>/dev/null | head -20
      core_fail=1
    fi
  done
done
if [ "$core_fail" -eq 0 ]; then
  echo "core boundary holds: ${CORE[*]} reach none of ${FORBIDDEN_CORE[*]}"
fi
fail=$((fail | core_fail))

# The two front-ends sit side by side in `mail-app` and never name each other: what they share
# is `mail-core`'s. A path through `crate::ui` from the terminal, or `crate::cli` from the
# window, is a helper that belongs in `mail-core` (or, if it is truly neither's, nowhere yet).
# `super::` and the crate's own name are caught too, as they reach the other side the same way.
if grep -rnE "(crate|super|mail_app)::ui\b" crates/mail-app/src/cli/; then
  echo "crates/mail-app/src/cli must not name the window (crate::ui): move the shared helper to mail-core"
  fail=1
fi
if grep -rnE "(crate|super|mail_app)::cli\b" crates/mail-app/src/ui/; then
  echo "crates/mail-app/src/ui must not name the command line (crate::cli): move the shared helper to mail-core"
  fail=1
fi

# `mail-core` returns values for a front-end to word, not the words of a terminal. Two habits
# still stand in the way, and each is held by an allowlist of the files that have not been
# converted yet: a file is listed to be fixed, never to be excused, and a line comes off the
# list when its file is converted. The check fails both ways: an unlisted file with the habit,
# and a listed file without it.
#
#   scripts/core-prose-allowlist.txt          a string naming a `mailo <subcommand>` to run
#   scripts/core-result-string-allowlist.txt  `Result<String, String>`: prose out, prose error
#
# Comment lines are not code and are not counted.
ratchet() { # <list file> <extended regex> <what the pattern finds>
  local list="$1" pattern="$2" what="$3" found listed bad=0
  found=$(grep -rEn "$pattern" crates/mail-core/src --include='*.rs' \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' | cut -d: -f1 | sort -u)
  listed=$(grep -vE '^[[:space:]]*(#|$)' "$list" | sort -u)
  while IFS= read -r file; do
    [ -z "$file" ] && continue
    if ! grep -qxF "$file" <<<"$listed"; then
      echo "$file: $what, and it is not in $list"
      grep -nE "$pattern" "$file" | grep -vE '^[0-9]+:[[:space:]]*//' | head -3
      bad=1
    fi
  done <<<"$found"
  while IFS= read -r file; do
    [ -z "$file" ] && continue
    if ! grep -qxF "$file" <<<"$found"; then
      echo "$file: listed in $list but it no longer has the habit: remove the line"
      bad=1
    fi
  done <<<"$listed"
  return "$bad"
}
ratchet scripts/core-prose-allowlist.txt 'mailo [a-z]' \
  "mail-core code names a terminal command (mailo <subcommand>)" || fail=1
ratchet scripts/core-result-string-allowlist.txt 'Result<String, String>' \
  "mail-core returns Result<String, String>" || fail=1
# And no other `Result<_, String>` either: a failure in mail-core is a `CoreError`, whose words are
# its `Display`, not a string handed about. Comment lines are not code.
if grep -rnE 'Result<.*, *String>' crates/mail-core/src --include='*.rs' \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "mail-core must not return Result<_, String>: use CoreError (crates/mail-core/src/error.rs)"
  fail=1
fi

# The libraries start no runtime and read no environment. The application (`mail-app`'s `edge`)
# owns the one tokio runtime and waits on `async` work there; `mail-core`, `mail-runtime` and
# `mail-store` hand every operation that waits on a network or the keyring back as a future, and
# take what they need (the link to accountd, the environment, the clock) from the `Mail` handle.
# So no library source builds a runtime or blocks on one, and `mail-core` reads no variable and
# asks the operating system for no executable path. Comment lines are not code; code after a
# file's `#[cfg(test)]` line, and files named `tests.rs` or `*_tests.rs`, are tests.
library_code() { # <dir>...: "file:line: text" for every non-comment, non-test line
  local file
  while IFS= read -r file; do
    awk -v f="$file" '/^#\[cfg\(test\)\]/ { exit }
      $0 !~ /^[[:space:]]*\/\// { print f ":" NR ": " $0 }' "$file"
  done < <(find "$@" -name '*.rs' ! -name 'tests.rs' ! -name '*_tests.rs' ! -path '*/tests/*')
}
if library_code crates/mail-core/src crates/mail-runtime/src crates/mail-store/src \
  | grep -E 'new_current_thread|new_multi_thread|Runtime::new\(|runtime::Builder::new|OnceLock<.*Runtime>'; then
  echo "a library builds a tokio runtime: make the function async and let mail-app's edge (crates/mail-app/src/edge.rs) wait on it"
  fail=1
fi
if library_code crates/mail-core/src | grep -E '\bblock_on\('; then
  echo "mail-core blocks on a future: make the function async and let mail-app's edge wait on it"
  fail=1
fi
# The desktop notification adapter (`notify/desktop.rs`, behind the `desktop` feature) asks for the
# program's own path to relaunch it on a click; it is the one place.
if library_code crates/mail-core/src | grep -vE '^crates/mail-core/src/notify/desktop\.rs:' \
  | grep -E 'env::var(_os)?\(|env::current_exe\('; then
  echo "mail-core reads the environment: take it from mail_core::Environment, which mail-app builds in main"
  fail=1
fi
# And no terminal: the command line is `mail-app`'s `cli`. `mail-core` returns typed outcomes
# that the cli words, takes a credential as a parameter or a callback instead of asking a person
# at a tty, and prints nothing (it logs with `log::`). So no `print!`/`println!`/`eprint!`/
# `eprintln!`, no `writeln!`/`write!` to stdout or stderr, no `rpassword`, no `/dev/tty`.
if library_code crates/mail-core/src \
  | grep -E '\b(e?print(ln)?!)\(|std::io::(stdout|stderr)\(|io::(stdout|stderr)\(|rpassword|/dev/tty'; then
  echo "mail-core touches the terminal: return a typed outcome and let mail-app's cli word and print it (log:: is fine)"
  fail=1
fi
if grep -nE '^[[:space:]]*rpassword' crates/mail-core/Cargo.toml; then
  echo "mail-core depends on rpassword: prompting a person is mail-app's cli"
  fail=1
fi
if library_code crates/mail-runtime/src | grep -E 'link::(install|current)\(|static CURRENT: OnceLock<Link>'; then
  echo "the link to accountd is a global again: hand it to mail_core::Mail::new (mail-app's edge::install_link keeps it for the process)"
  fail=1
fi

# `mail-app` reaches the logic through `mail-core`'s `Mail` handle. It still names `mail-runtime`
# and `mail-store` directly in many places (the store is the window's own model, and the signing
# keys, places and engines are used by the window); that is a ratchet, not a clean line: the
# counts below may go down and never up. Lines naming the crate, tests included.
# crates/mail-app/src/edge.rs is the one place that builds the handle.
# scripts/app-lower-crate-uses.txt holds "<crate> <count>".
while read -r lower allowed; do
  [ -z "$lower" ] && continue
  case "$lower" in \#*) continue ;; esac
  now=$(grep -rE "\b${lower}\b" crates/mail-app/src crates/mail-app/tests --include='*.rs' | wc -l)
  if [ "$now" -gt "$allowed" ]; then
    echo "mail-app names $lower on $now lines, over the $allowed in scripts/app-lower-crate-uses.txt: go through mail_core (add the call there) instead"
    fail=1
  elif [ "$now" -lt "$allowed" ]; then
    echo "mail-app names $lower on only $now lines: lower the $allowed in scripts/app-lower-crate-uses.txt to $now"
    fail=1
  fi
done < scripts/app-lower-crate-uses.txt

# Names that moved to porter, or went with the types they named, must not creep back in. mailo
# names accounts, secrets and OAuth issuers with `porter_core` and `porter_provider`'s types
# (`AccountId`, `SecretKey`, `SecretPurpose`, `Credential`, `Issuer`), and keeps its own signing
# keys in `mail_domain::signing`. So there is no `OAuthIssuer`; no `mail_domain::AccountId` (or
# `Credential`, `SecretKey`, `SecretPurpose`) to import; no `AccountId` minted from a UUID by a
# method (`mail_domain::id::new_account_id` and `account_id_from_uuid` do it); and no key held
# as a `SecretPurpose` or a `Credential`.
#
# E2 adds what went with account secrets: mailo's own `Secrets` trait, `KeyringSecrets` and
# `MapSecrets` (an account's secrets are `porter_secrets::Secrets`, held as
# `mail_runtime::AccountSecrets`; signing keys are `SigningStore`), the `secrets` module they
# lived in, and the signing methods the one trait carried (`get_signing`, `put_signing`,
# `forget_signing`). `mail_runtime::KeyringSecrets` is porter's `KeyringSecrets` under another
# crate's name, so it is forbidden by that path and `porter_secrets::KeyringSecrets` is not.
#
# E3 adds what went with account discovery: the preset table's lookup by address
# (`presets::preset_for`, `preset_for_mail_exchanger`, which provider an address belongs to is
# porter's provider files' `matching` now, and what is built-in is `mail_core::discover::known`),
# the personal-Microsoft test (`is_personal_microsoft`; `issuer_named` stays in mailo: porter-discover
# hands over the document's issuer as data, the mapping to a preset is mailo's), and the two modules
# that held discovery: `mail_proto::discover` (what each answer meant) and `mail_runtime::discover` (the search, over reqwest and hickory), which are
# `porter_discover`'s now, over `mail_runtime::lookup`'s two seams.
#
# E4 adds what went with OAuth: the four modules (`mail_runtime::oauth`, `signin`, `renewal`,
# `loopback`, and inside the crate `crate::oauth` and the like) and the types they held
# (`OAuthRegistry`, `Registration`, `Renewal`, `Loopback`, `Pending`, `Freshness`, the
# `oauth2` crate). PKCE, the loopback listener, the exchanges and the client registry are
# `porter_oauth`'s; keeping a token fresh is `mail_runtime::tokens`, behind `TokenSource`.
#
# E5 adds what went with the add-account sheet: its three modules (`add_account::flow`, `sheet` and
# `copy`, the machine, the sheet drawn over it and the words for its refusals) and the sheet itself
# (`AddAccountSheet`), with the `Shell::adding` it was open by. The machine is porter's
# (`porter_core::sheet`), run by `porter_service`, and the window is `add_account::window`, drawn
# with `ds-shell::accounts`.
FORBIDDEN_SYMBOLS='\bOAuthRegistry\b|\bmail_runtime::adopt\b|\bmail_app::adoption\b|\bunadopted_accounts\b|\bmark_secrets_adopted\b|\bmail_runtime::(oauth|signin|renewal|loopback)\b|\bmail_runtime::\{[^}]*\b(oauth|signin|renewal|loopback|Renewal|Registration|Loopback)\b|\bmail_runtime::(Renewal|Registration|Loopback)\b|\bcrate::(oauth|signin|renewal|loopback)\b|\bsignin::(http_client|renew|graph_token|default_path)\b|\boauth2::|\bwith_renewal\b|\bOAuthIssuer\b|\bAccountId::(from_uuid|generate)\b|\bSecretPurpose::(AddressBook|OpenPgp|Smime)\b|\bCredential::(OpenPgp|SmimeKey)\b|\bmail_domain::(AccountId|Credential|SecretKey|SecretPurpose)\b|\bMapSecrets\b|\bmail_runtime::(KeyringSecrets|Secrets|MapSecrets|secrets)\b|\bmail_runtime::\{[^}]*\b(KeyringSecrets|Secrets|MapSecrets)\b|\bdyn Secrets\b|\b(get|put|forget)_signing\b|\bcrate::secrets\b|\bpresets::preset_for\b|\bpresets::\{[^}]*\bpreset_for\b|\bpreset_for\(|\bpreset_for_mail_exchanger\b|\bis_personal_microsoft\b|\bmail_proto::discover\b|\bmail_runtime::discover\b|\bmail_(proto|runtime)::\{[^}]*\bdiscover\b|\badd_account::(flow|sheet|copy)\b|\bAddAccountSheet\b|\bshell\.(read|write)\(\)\.adding\b'
if grep -rnE "$FORBIDDEN_SYMBOLS" crates --include='*.rs' | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "a symbol that moved to porter (or was deleted with its type) is back: see FORBIDDEN_SYMBOLS in scripts/check-boundary.sh"
  fail=1
fi

# The D-Bus link to accountd is desktop-only code behind the `quire-desktop` feature (quire design/36
# rule 3): `porter_dbus` is named in `mail-runtime`'s `link/dbus.rs` and its bus test, and quire's
# probe `ds_desktop` in `mail-app`'s `accountd` and its tests, and nowhere else. Everything else
# asks `mail_runtime::link::Accountd`, which a build for another desktop has too.
if grep -rnE '\b(porter_dbus|ds_desktop)\b' crates --include='*.rs' \
  | grep -vE '^crates/mail-runtime/src/link/dbus\.rs:|^crates/mail-runtime/tests/runtime/link_bus\.rs:|^crates/mail-app/src/accountd(_tests)?\.rs:' \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "porter_dbus and ds_desktop belong to the quire-desktop link only: see scripts/check-boundary.sh"
  fail=1
fi

# `mail-store` is the only crate that speaks SQL. Above it the store is a typed API (accounts,
# identities, labels, blobs, lookups by `Message-ID`, ...), and a test that needs a row no path of
# the program writes asks `mail_store::testing`. So `rusqlite` is not a dependency of `mail-core`,
# `mail-runtime` or `mail-app` (dev-dependencies included), and the store's connection, which is
# private to it, is not reached for by name: `.connection()`, `.reader()`, and the test-support
# `raw_connection` / `raw_accounts` that only `mail-store`'s own tests use. Comment lines are not
# code and are not counted.
for crate in mail-core mail-runtime mail-app; do
  if grep -nE '^[[:space:]]*rusqlite[[:space:]]*=' "crates/$crate/Cargo.toml"; then
    echo "crates/$crate/Cargo.toml: $crate must not depend on rusqlite: ask the store (mail_store), or mail_store::testing in a test"
    fail=1
  fi
done
if grep -rnE '\brusqlite\b|\.connection\(\)|\.reader\(\)|\braw_connection\b|\braw_accounts\b' \
  crates/mail-core crates/mail-runtime crates/mail-app --include='*.rs' \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "SQL above mail-store: add the typed call to mail_store (or a seeding helper to mail_store::testing) instead"
  fail=1
fi

# The reader draws blocks. A raw HTML sink in the UI would put a sender's markup
# in our document, which is what the block types exist to prevent.
if grep -rn "dangerous_inner_html" crates/mail-app/src/ui/; then
  echo "dangerous_inner_html is forbidden under crates/mail-app/src/ui/: the reader draws blocks, not raw markup"
  exit 1
fi

exit "$fail"
