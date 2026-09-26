#!/usr/bin/env bash
# Build mailo's packages: a .deb, an .rpm and a Flatpak.
#
#   ./scripts/package.sh            every format whose tool is installed
#   ./scripts/package.sh deb        one format: deb, rpm or flatpak
#
# Each format needs an external tool, and none of them is a dependency of the build:
#   deb      cargo install cargo-deb
#   rpm      cargo install cargo-generate-rpm
#   flatpak  flatpak-builder, and flatpak-builder-tools' flatpak-cargo-generator.py on PATH
#            (https://github.com/flatpak/flatpak-builder-tools, cargo/), which needs python3
#            with aiohttp and toml (or tomlkit)
# A missing tool is reported and that format skipped; the script fails only if a build that
# was attempted fails, or if a format named on the command line could not be attempted.
#
# The .deb and .rpm need a release build (`cargo build --release -p mail-app`), which this
# script makes once. The packages land in target/debian/, target/generate-rpm/ and
# target/flatpak/.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-$root/target}"

if [ "$#" -gt 0 ]; then
    wanted=("$@")
    named=yes
else
    wanted=(deb rpm flatpak)
    named=no
fi

missing=0
failed=0
built_release=no

have_cargo_tool() {
    cargo "$1" --version >/dev/null 2>&1 || cargo "$1" --help >/dev/null 2>&1
}

skip() {
    echo "package.sh: skipping $1: $2" >&2
    missing=$((missing + 1))
}

release() {
    if [ "$built_release" = no ]; then
        cargo build --release --locked -p mail-app --bin mailo
        built_release=yes
    fi
}

deb() {
    if ! have_cargo_tool deb; then
        skip deb "cargo-deb is not installed (cargo install cargo-deb)"
        return
    fi
    release
    cargo deb -p mail-app --no-build || failed=$((failed + 1))
}

rpm() {
    if ! have_cargo_tool generate-rpm; then
        skip rpm "cargo-generate-rpm is not installed (cargo install cargo-generate-rpm)"
        return
    fi
    release
    # cargo-generate-rpm does not strip; the release profile's binary is shipped as built.
    cargo generate-rpm -p crates/mail-app --target-dir "$target" || failed=$((failed + 1))
}

flatpak() {
    if ! command -v flatpak-builder >/dev/null 2>&1; then
        skip flatpak "flatpak-builder is not installed"
        return
    fi
    if ! command -v flatpak-cargo-generator.py >/dev/null 2>&1; then
        skip flatpak "flatpak-cargo-generator.py is not on PATH (flatpak-builder-tools, cargo/)"
        return
    fi
    local manifest=packaging/flatpak/io.github.PoHsuanLai.mailo.yml
    flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json ||
        { failed=$((failed + 1)); return; }
    mkdir -p "$target/flatpak"
    flatpak-builder --user --force-clean --install-deps-from=flathub \
        --repo="$target/flatpak/repo" "$target/flatpak/build" "$manifest" ||
        { failed=$((failed + 1)); return; }
    flatpak build-bundle "$target/flatpak/repo" "$target/flatpak/mailo.flatpak" \
        io.github.PoHsuanLai.mailo || failed=$((failed + 1))
}

for format in "${wanted[@]}"; do
    case "$format" in
        deb) deb ;;
        rpm) rpm ;;
        flatpak) flatpak ;;
        *)
            echo "package.sh: unknown format $format (deb, rpm or flatpak)" >&2
            exit 2
            ;;
    esac
done

if [ "$failed" -gt 0 ]; then
    echo "package.sh: $failed build(s) failed" >&2
    exit 1
fi
if [ "$named" = yes ] && [ "$missing" -gt 0 ]; then
    exit 1
fi
