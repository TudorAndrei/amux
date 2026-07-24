#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '\r\n' <"$ROOT/VERSION")"
expected_version="amux $version"

has_current_binary() {
	local candidate
	for candidate in "$ROOT/bin/amux-rs" "$ROOT/target/release/amux-rs" "$ROOT/target/debug/amux-rs"; do
		if [ -x "$candidate" ] && [ "$("$candidate" --version 2>/dev/null || true)" = "$expected_version" ]; then
			return 0
		fi
	done
	return 1
}

has_current_binary && exit 0

case "$(uname -s):$(uname -m)" in
Darwin:arm64) target="aarch64-apple-darwin" ;;
Linux:x86_64) target="x86_64-unknown-linux-gnu" ;;
Linux:aarch64 | Linux:arm64) target="aarch64-unknown-linux-gnu" ;;
*)
	printf 'amux: no release is available for %s:%s\n' "$(uname -s)" "$(uname -m)" >&2
	exit 1
	;;
esac

if ! command -v gh >/dev/null 2>&1; then
	printf '%s\n' 'amux: GitHub CLI (gh) is required for TPM to download the native release binary' >&2
	exit 1
fi

archive="amux-${version}-${target}.tar.gz"
package="amux-${version}-${target}"
temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/amux-runtime.XXXXXX")"
trap 'rm -rf "$temporary_dir"' EXIT

gh release download "v$version" \
	--repo TudorAndrei/amux \
	--pattern "$archive" \
	--dir "$temporary_dir"

source_binary="$temporary_dir/$package/bin/amux-rs"
tar -xzf "$temporary_dir/$archive" -C "$temporary_dir"
if [ ! -f "$source_binary" ]; then
	printf '%s\n' "amux: release v$version did not contain $package/bin/amux-rs" >&2
	exit 1
fi

temporary_binary="$ROOT/bin/.amux-rs.$$"
install -m 755 "$source_binary" "$temporary_binary"
mv -f "$temporary_binary" "$ROOT/bin/amux-rs"
