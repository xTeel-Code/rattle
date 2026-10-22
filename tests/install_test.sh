#!/bin/sh
# Tests install.sh against a fake release served from 127.0.0.1 (no network).
#
#   sh tests/install_test.sh [path/to/rattle]   (default: target/debug/rattle)

set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${1:-$ROOT/target/debug/rattle}
[ -x "$BIN" ] || { echo "build rattle first ($BIN missing)" >&2; exit 1; }

WORK=$(mktemp -d)
SERVER=""
cleanup() {
    if [ -n "$SERVER" ]; then
        kill "$SERVER" 2>/dev/null || true
        wait "$SERVER" 2>/dev/null || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok - $*"; }

case "$(uname -m)" in arm64 | aarch64) arch=aarch64 ;; *) arch=x86_64 ;; esac
case "$(uname -s)" in Darwin) target="$arch-apple-darwin" ;; *) target="$arch-unknown-linux-musl" ;; esac
asset="rattle-$target.tar.gz"

sha256() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# A fake release: the binary, packed like the release workflow does.
mkdir -p "$WORK/release" "$WORK/pack"
cp "$BIN" "$WORK/pack/rattle"
tar -czf "$WORK/release/$asset" -C "$WORK/pack" rattle
echo "$(sha256 "$WORK/release/$asset")  $asset" >"$WORK/release/$asset.sha256"

PORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$WORK/release" >/dev/null 2>&1 &
SERVER=$!
i=0
until curl -fs "http://127.0.0.1:$PORT/" >/dev/null 2>&1; do
    i=$((i + 1))
    [ $i -lt 100 ] || fail "test server did not start"
    sleep 0.1
done

install() { # $1 = install dir; extra env comes from the caller
    RATTLE_DOWNLOAD_BASE="http://127.0.0.1:$PORT" RATTLE_INSTALL_DIR="$1" sh "$ROOT/install.sh"
}

# 1. Installs the verified binary, and it runs.
out=$(install "$WORK/a/bin" 2>&1) || fail "install failed: $out"
echo "$out" | grep -q "checksum ok" || fail "no checksum check: $out"
"$WORK/a/bin/rattle" help | grep -q "Usage: rattle" || fail "installed binary does not run"
echo "$out" | grep -q "is not on your PATH" || fail "no PATH hint: $out"
pass "installs a checksum-verified binary"

# 2. Installing again replaces it (an update).
install "$WORK/a/bin" >/dev/null 2>&1 || fail "reinstall failed"
pass "reinstalls over an existing binary"

# 3. A tampered download is refused and nothing is installed.
echo "0000000000000000000000000000000000000000000000000000000000000000  $asset" >"$WORK/release/$asset.sha256"
if out=$(install "$WORK/b/bin" 2>&1); then fail "installed despite a bad checksum"; fi
echo "$out" | grep -q "checksum mismatch" || fail "unexpected error: $out"
[ ! -e "$WORK/b/bin/rattle" ] || fail "binary installed despite a bad checksum"
pass "refuses a download whose checksum does not match"

# 4. No checksum published: refused too.
rm "$WORK/release/$asset.sha256"
if out=$(install "$WORK/c/bin" 2>&1); then fail "installed an unverified binary"; fi
echo "$out" | grep -q "refusing to install an unverified binary" || fail "unexpected error: $out"
pass "refuses a download without a checksum"

# 5. No release asset and no cargo: explains how to get Rust.
rm "$WORK/release/$asset"
if out=$(PATH=/usr/bin:/bin install "$WORK/d/bin" 2>&1); then fail "succeeded without a binary or cargo"; fi
echo "$out" | grep -q "no release download found" || fail "unexpected output: $out"
echo "$out" | grep -q "rustup.rs" || fail "no Rust hint: $out"
pass "without a release or cargo, points to rustup"

echo "all install.sh tests passed"
