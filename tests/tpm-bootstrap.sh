#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
WORK_DIR="$(mktemp -d "$TMPDIR/amux-tpm.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT

case "$(uname -s):$(uname -m)" in
Darwin:arm64) target="aarch64-apple-darwin" ;;
Linux:x86_64) target="x86_64-unknown-linux-gnu" ;;
Linux:aarch64 | Linux:arm64) target="aarch64-unknown-linux-gnu" ;;
*) exit 0 ;;
esac

version="$(tr -d '\r\n' <"$ROOT/VERSION")"
package="amux-${version}-${target}"
archive="${package}.tar.gz"
release_dir="$WORK_DIR/release/$package/bin"
plugin_dir="$WORK_DIR/plugin"
failed_plugin_dir="$WORK_DIR/failed-plugin"
fake_bin="$WORK_DIR/fake-bin"
mkdir -p "$release_dir" "$plugin_dir/bin" "$plugin_dir/scripts" \
	"$failed_plugin_dir/bin" "$failed_plugin_dir/scripts" "$fake_bin"
cp "$ROOT/target/debug/amux-rs" "$release_dir/amux-rs"
tar -C "$WORK_DIR/release" -czf "$WORK_DIR/$archive" "$package"
cp "$ROOT/VERSION" "$ROOT/bin/amux" "$plugin_dir/"
cp "$ROOT/bin/amux" "$plugin_dir/bin/"
cp "$ROOT/scripts/ensure-runtime.sh" "$plugin_dir/scripts/"
cp "$ROOT/VERSION" "$failed_plugin_dir/"
cp "$ROOT/bin/amux" "$failed_plugin_dir/bin/"
cp "$ROOT/scripts/ensure-runtime.sh" "$failed_plugin_dir/scripts/"

cat >"$fake_bin/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [ "$1" = "release" ] && [ "$2" = "download" ]; then
	printf '%s\n' download >>"$AMUX_TEST_GH_LOG"
	while [ "$#" -gt 0 ]; do
		if [ "$1" = "--dir" ]; then
			cp "$AMUX_TEST_ARCHIVE" "$2/$AMUX_TEST_ARCHIVE_NAME"
			exit 0
		fi
		shift
	done
	exit 1
fi
if [ "$1" = "attestation" ] && [ "$2" = "verify" ]; then
	printf '%s\n' verify >>"$AMUX_TEST_GH_LOG"
	test "$4" = "--repo"
	test "$5" = "TudorAndrei/amux"
	if [ "${AMUX_TEST_VERIFY_FAIL:-0}" = 1 ]; then
		exit 1
	fi
	exit 0
fi
while [ "$#" -gt 0 ]; do
	shift
done
exit 1
SH
chmod +x "$fake_bin/gh"

export AMUX_TEST_ARCHIVE="$WORK_DIR/$archive"
export AMUX_TEST_ARCHIVE_NAME="$archive"
export AMUX_TEST_GH_LOG="$WORK_DIR/gh.log"
export AMUX_TEST_VERIFY_FAIL=1
if PATH="$fake_bin:$PATH" "$failed_plugin_dir/bin/amux" --version >/dev/null 2>&1; then
	printf '%s\n' 'bootstrap accepted an archive with a failed attestation' >&2
	exit 1
fi
test ! -e "$failed_plugin_dir/bin/amux-rs"
test "$(tail -n 2 "$AMUX_TEST_GH_LOG")" = "$(printf 'download\nverify')"

: >"$AMUX_TEST_GH_LOG"
export AMUX_TEST_VERIFY_FAIL=0
PATH="$fake_bin:$PATH" "$plugin_dir/bin/amux" --version | grep -Fx "amux $version" >/dev/null
test -x "$plugin_dir/bin/amux-rs"
test "$(cat "$AMUX_TEST_GH_LOG")" = "$(printf 'download\nverify')"
printf 'ok\n'
