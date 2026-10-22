#!/bin/sh
# Installs rattle: https://github.com/xTeel-Code/rattle
#
#   curl -fsSL https://raw.githubusercontent.com/xTeel-Code/rattle/main/install.sh | sh
#
# Downloads a prebuilt binary from the latest GitHub release (checksum
# verified), or builds from source with cargo when there is none for this
# platform. Settings, as environment variables:
#
#   RATTLE_INSTALL_DIR   where to put the binary      (default: ~/.local/bin)
#   RATTLE_VERSION       a release tag, e.g. v0.3.0   (default: the latest)
#   RATTLE_FROM_SOURCE   1 = always build with cargo
#
# For example:
#   curl -fsSL https://raw.githubusercontent.com/xTeel-Code/rattle/main/install.sh | RATTLE_INSTALL_DIR=/usr/local/bin sh

set -eu

REPO="xTeel-Code/rattle"
BIN_DIR="${RATTLE_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${RATTLE_VERSION:-latest}"
# Where release assets are downloaded from (overridable for testing).
if [ "$VERSION" = latest ]; then
    BASE_DEFAULT="https://github.com/$REPO/releases/latest/download"
else
    BASE_DEFAULT="https://github.com/$REPO/releases/download/$VERSION"
fi
BASE="${RATTLE_DOWNLOAD_BASE:-$BASE_DEFAULT}"

say() { printf 'rattle-install: %s\n' "$*"; }
die() { printf 'rattle-install: error: %s\n' "$*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        arm64 | aarch64) arch=aarch64 ;;
        x86_64 | amd64) arch=x86_64 ;;
        *) return 1 ;;
    esac
    case "$os" in
        Darwin) echo "$arch-apple-darwin" ;;
        Linux) echo "$arch-unknown-linux-musl" ;;
        *) return 1 ;;
    esac
}

sha256() {
    if has sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif has shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        return 1
    fi
}

# Downloads and verifies the release binary; returns 1 if there is none to use.
install_binary() {
    target=$(target) || { say "no prebuilt binary for $(uname -s) $(uname -m)"; return 1; }
    has curl || { say "curl not found"; return 1; }
    has tar || { say "tar not found"; return 1; }
    asset="rattle-$target.tar.gz"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    say "downloading $asset ($VERSION)"
    if ! curl -fsSL -o "$tmp/$asset" "$BASE/$asset"; then
        say "no release download found at $BASE/$asset"
        return 1
    fi
    if curl -fsSL -o "$tmp/$asset.sha256" "$BASE/$asset.sha256"; then
        expected=$(cut -d' ' -f1 <"$tmp/$asset.sha256")
        actual=$(sha256 "$tmp/$asset") || die "no sha256sum or shasum to verify the download"
        [ "$expected" = "$actual" ] || die "checksum mismatch for $asset (expected $expected, got $actual)"
        say "checksum ok"
    else
        die "no checksum published for $asset; refusing to install an unverified binary"
    fi

    tar -xzf "$tmp/$asset" -C "$tmp"
    [ -f "$tmp/rattle" ] || die "$asset does not contain a rattle binary"
    mkdir -p "$BIN_DIR"
    # Replace atomically, so a running rattle is never left half-written.
    cp "$tmp/rattle" "$BIN_DIR/.rattle.new"
    chmod 755 "$BIN_DIR/.rattle.new"
    mv "$BIN_DIR/.rattle.new" "$BIN_DIR/rattle"
}

install_from_source() {
    if ! has cargo; then
        die "no prebuilt binary to use and Rust is not installed.
  Install Rust (https://rustup.rs):
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  then run this installer again."
    fi
    # --root puts the binary in $BIN_DIR, same as the prebuilt one.
    root=$(dirname "$BIN_DIR")
    [ "$(basename "$BIN_DIR")" = bin ] || die "RATTLE_INSTALL_DIR must end in /bin when building from source"
    # Build a release, like the prebuilt path; main only if none exists yet.
    tag="$VERSION"
    if [ "$tag" = latest ]; then
        tag=$(latest_tag) || tag=""
    fi
    if [ -n "$tag" ]; then
        say "building $tag from source with cargo (a minute or two)"
        cargo install --locked --force --root "$root" --git "https://github.com/$REPO" --tag "$tag"
    else
        say "no release yet; building the latest main from source with cargo"
        cargo install --locked --force --root "$root" --git "https://github.com/$REPO"
    fi
}

# The latest release's tag: GitHub redirects /releases/latest to
# /releases/tag/<tag> (to /releases when there is none).
latest_tag() {
    url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") || return 1
    case "$url" in
        */releases/tag/*) echo "${url##*/}" ;;
        *) return 1 ;;
    esac
}

if [ "${RATTLE_FROM_SOURCE:-0}" = 1 ] || ! install_binary; then
    install_from_source
fi

"$BIN_DIR/rattle" help >/dev/null 2>&1 || die "installed $BIN_DIR/rattle does not run"
say "installed $BIN_DIR/rattle"

# Is it the rattle a shell will find?
found=$(command -v rattle 2>/dev/null || true)
case ":$PATH:" in
    *":$BIN_DIR:"*)
        if [ -n "$found" ] && [ "$found" != "$BIN_DIR/rattle" ]; then
            say "note: another rattle comes first on your PATH: $found"
        fi
        ;;
    *)
        say "$BIN_DIR is not on your PATH yet; add it to your shell profile:"
        # shellcheck disable=SC2016 # $PATH is meant literally: it's a line to paste.
        printf '    echo '\''export PATH="%s:$PATH"'\'' >> ~/.zshrc   # or ~/.bashrc\n' "$BIN_DIR"
        ;;
esac
say "run 'rattle' to open it, or 'rattle help' for all commands"
