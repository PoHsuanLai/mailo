#!/usr/bin/env bash
# The sans-I/O boundary, mechanically enforced.
#
# `cargo tree -i <dep>` exits 101 when the dependency is absent, which is precisely the state
# we want. Checking the exit status would therefore fail whenever the boundary holds, so we
# check for OUTPUT instead: any line naming the dependency is a leak.
set -uo pipefail

# Package names, as `cargo tree -p` takes them: the pure crates are published as `mailo-*`.
PURE=(mailo-domain mailo-mime mailo-proto mailo-pim)
# Nor porter's client, its bus vocabulary or quire's capability probe: the sans-I/O crates know
# nothing of the desktop's accountd. Nor anyview (the attachment peek and its card): only
# `mail-app` draws, behind its `anyview` feature.
FORBIDDEN=(tokio rusqlite dioxus reqwest keyring-core porter-client porter-dbus ds-desktop zbus anyview-core anyview-peek)
# `mail-core` is the I/O-capable core: it may use tokio and the network, which the PURE
# crates may not. What it may never reach is anything that draws, the window's toolkit or the
# renderer under it, so that `mail-app`'s two front-ends (`ui`, `cli`) sit over one core that
# knows neither.
CORE=(mail-core mail-client)
FORBIDDEN_CORE=(dioxus ds ds-settings ds-blitz blitz-dom dioxus-native-dom anyview-core anyview-peek)
fail=0

# A name `cargo tree` does not know prints nothing, which would read as a boundary that holds.
for crate in "${PURE[@]}" "${CORE[@]}" mail-store mail-runtime; do
  if ! cargo tree -p "$crate" --depth 0 >/dev/null 2>&1; then
    echo "no package named $crate: update the names in this script"
    exit 1
  fi
done

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

# `mail-store` and `mail-runtime` sit between the pure crates and the core, and draw nothing either:
# anyview, which peeks at a file and draws a card, is `mail-app`'s alone (its `anyview` feature).
lower_fail=0
for crate in mail-store mail-runtime; do
  for dep in anyview-core anyview-peek; do
    if cargo tree -p "$crate" -i "$dep" 2>/dev/null | grep -q .; then
      echo "LEAK: $crate depends on $dep"
      cargo tree -p "$crate" -i "$dep" 2>/dev/null | head -20
      lower_fail=1
    fi
  done
done
if [ "$lower_fail" -eq 0 ]; then
  echo "anyview boundary holds: mail-store and mail-runtime reach neither anyview-core nor anyview-peek"
fi
fail=$((fail | lower_fail))

# Within the window (`mail-ui`), anyview is named by the reader's attachment code only (`ui/reading`), behind
# the `anyview` feature: the strip's thumbnail, Peek's card, and the adapter between them.
if grep -rnE '\banyview_(core|peek)\b' crates --include='*.rs' \
  | grep -vE '^crates/mail-ui/src/ui/reading/(anyview|thumb|peek/part)\.rs:|^crates/mail-app/tests/app/native_anyview\.rs:' \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "anyview_core and anyview_peek belong to ui/reading's anyview.rs, thumb.rs and peek/part.rs: see scripts/check-boundary.sh"
  fail=1
fi

# The two front-ends sit side by side in `mail-app` and never name each other: what they share
# is `mail-core`'s. A path through `crate::ui` from the terminal, or `crate::cli` from the
# window, is a helper that belongs in `mail-core` (or, if it is truly neither's, nowhere yet).
# `super::` and the crate's own name are caught too, as they reach the other side the same way.
if grep -rnE "(crate|super|mail_app)::ui\b" crates/mail-app/src/cli/; then
  echo "crates/mail-app/src/cli must not name the window (crate::ui): move the shared helper to mail-core"
  fail=1
fi
if grep -rnE "(crate|super|mail_app)::cli\b" crates/mail-ui/src/ui/; then
  echo "crates/mail-ui/src/ui must not name the command line (crate::cli): move the shared helper to mail-core"
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
# take what they need (the secrets, the environment, the clock) from the `Mail` handle.
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
# Then what went with account secrets: mailo's own `Secrets` trait, `KeyringSecrets` and
# `MapSecrets` (an account's secrets are `porter_secrets::Secrets`, held as
# `mail_runtime::AccountSecrets`; signing keys are `SigningStore`), the `secrets` module they
# lived in, and the signing methods the one trait carried (`get_signing`, `put_signing`,
# `forget_signing`). `mail_runtime::KeyringSecrets` is porter's `KeyringSecrets` under another
# crate's name, so it is forbidden by that path and `porter_secrets::KeyringSecrets` is not.
#
# Then what went with account discovery: the preset table's lookup by address
# (`presets::preset_for`, `preset_for_mail_exchanger`, which provider an address belongs to is
# porter's provider files' `matching` now, and what is built-in is `mail_core::discover::known`),
# the personal-Microsoft test (`is_personal_microsoft`; `issuer_named` stays in mailo: porter-discover
# hands over the document's issuer as data, the mapping to a preset is mailo's), and the two modules
# that held discovery: `mail_proto::discover` (what each answer meant) and `mail_runtime::discover` (the search, over reqwest and hickory), which are
# `porter_discover`'s now, over `mail_runtime::lookup`'s two seams.
#
# Then what went with OAuth: the four modules (`mail_runtime::oauth`, `signin`, `renewal`,
# `loopback`, and inside the crate `crate::oauth` and the like) and the types they held
# (`OAuthRegistry`, `Registration`, `Renewal`, `Loopback`, `Pending`, `Freshness`, the
# `oauth2` crate). PKCE, the loopback listener, the exchanges and the client registry are
# `porter_oauth`'s; keeping a token fresh is `mail_runtime::tokens`, behind `TokenSource`.
#
# Then what went with the add-account sheet: its three modules (`add_account::flow`, `sheet` and
# `copy`, the machine, the sheet drawn over it and the words for its refusals) and the sheet itself
# (`AddAccountSheet`), with the `Shell::adding` it was open by. The machine is porter's
# (`porter_core::sheet`), run by `porter_service`, and the window is `add_account::window`, drawn
# with `ds-shell::accounts`.
FORBIDDEN_SYMBOLS='\bOAuthRegistry\b|\bmail_runtime::adopt\b|\bmail_app::adoption\b|\bunadopted_accounts\b|\bmark_secrets_adopted\b|\bmail_runtime::(oauth|signin|renewal|loopback)\b|\bmail_runtime::\{[^}]*\b(oauth|signin|renewal|loopback|Renewal|Registration|Loopback)\b|\bmail_runtime::(Renewal|Registration|Loopback)\b|\bcrate::(oauth|signin|renewal|loopback)\b|\bsignin::(http_client|renew|graph_token|default_path)\b|\boauth2::|\bwith_renewal\b|\bOAuthIssuer\b|\bAccountId::(from_uuid|generate)\b|\bSecretPurpose::(AddressBook|OpenPgp|Smime)\b|\bCredential::(OpenPgp|SmimeKey)\b|\bmail_domain::(AccountId|Credential|SecretKey|SecretPurpose)\b|\bMapSecrets\b|\bmail_runtime::(KeyringSecrets|Secrets|MapSecrets|secrets)\b|\bmail_runtime::\{[^}]*\b(KeyringSecrets|Secrets|MapSecrets)\b|\bdyn Secrets\b|\b(get|put|forget)_signing\b|\bcrate::secrets\b|\bpresets::preset_for\b|\bpresets::\{[^}]*\bpreset_for\b|\bpreset_for\(|\bpreset_for_mail_exchanger\b|\bis_personal_microsoft\b|\bmail_proto::discover\b|\bmail_runtime::discover\b|\bmail_(proto|runtime)::\{[^}]*\bdiscover\b|\badd_account::(flow|sheet|copy)\b|\bAddAccountSheet\b|\bshell\.(read|write)\(\)\.adding\b'
if grep -rnE "$FORBIDDEN_SYMBOLS" crates --include='*.rs' | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "a symbol that moved to porter (or was deleted with its type) is back: see FORBIDDEN_SYMBOLS in scripts/check-boundary.sh"
  fail=1
fi

# Linked mode is gone: mailo keeps its own accounts and credentials, and no account is the desktop
# accountd's. The link (`mail_runtime::link`, its secrets and tokens, `Link`, `Accountd`), the
# `granted` plan, the held accounts it set aside and the app's `accountd` module are not to come
# back by name; accountd's sign-in help, when it comes, is a sign-in like any other.
LINKED_SYMBOLS='\b(porter_dbus|ds_desktop)\b|\bmail_(runtime|core)::link\b|\bLinkedSecrets\b|\bLinkedTokens\b|\bAuthPlan::Granted\b|\bset_granted_only\b|\bheld_accounts\b|\bHELD_LINE\b|\bcrate::accountd\b|\bmail_app::accountd\b|\binstall_link\b'
if grep -rnE "$LINKED_SYMBOLS" crates --include='*.rs' | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "linked mode is gone: see LINKED_SYMBOLS in scripts/check-boundary.sh"
  fail=1
fi

# `mail-store` is the only crate that speaks SQL. Above it the store is a typed API (accounts,
# identities, labels, blobs, lookups by `Message-ID`, ...), and a test that needs a row no path of
# the program writes asks `mail_store::testing`. So `rusqlite` is not a dependency of `mail-core`,
# `mail-runtime` or `mail-app` (dev-dependencies included), and the store's connection, which is
# private to it, is not reached for by name: `.connection()`, `.reader()`, and the testing
# `raw_connection` that only `mail-store`'s own tests use. Comment lines are not
# code and are not counted.
for crate in mail-core mail-runtime mail-app; do
  if grep -nE '^[[:space:]]*rusqlite[[:space:]]*=' "crates/$crate/Cargo.toml"; then
    echo "crates/$crate/Cargo.toml: $crate must not depend on rusqlite: ask the store (mail_store), or mail_store::testing in a test"
    fail=1
  fi
done
if grep -rnE '\brusqlite\b|\.connection\(\)|\.reader\(\)|\braw_connection\b' \
  crates/mail-core crates/mail-runtime crates/mail-app --include='*.rs' \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//'; then
  echo "SQL above mail-store: add the typed call to mail_store (or a seeding helper to mail_store::testing) instead"
  fail=1
fi

# A library crate pays for nothing it was not asked for: its default features carry none of the
# platform keyring, the desktop's extras, the Secret Service or the test doubles. `mail-mime`'s crypto
# halves and `mail-core`'s `desktop` and `pdf` are the defaults a plain user expects; the rest the
# program names. (`mail-app` is the program: its own `quire-desktop` default is its build's choice.)
for crate in mail-domain mail-mime mail-pim mail-store mail-proto mail-runtime mail-core; do
  if grep -E '^default[[:space:]]*=' "crates/$crate/Cargo.toml" | grep -E 'quire-desktop|keyring|oo7|testing'; then
    echo "crates/$crate/Cargo.toml: a library's default features must not include quire-desktop, keyring, oo7 or testing"
    fail=1
  fi
done

# The reader draws blocks. A raw HTML sink in the UI would put a sender's markup
# in our document, which is what the block types exist to prevent.
if grep -rn "dangerous_inner_html" crates/mail-ui/src/ui/; then
  echo "dangerous_inner_html is forbidden under crates/mail-ui/src/ui/: the reader draws blocks, not raw markup"
  exit 1
fi

exit "$fail"
