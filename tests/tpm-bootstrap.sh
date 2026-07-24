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
fake_bin="$WORK_DIR/fake-bin"
mkdir -p "$release_dir" "$plugin_dir/bin" "$plugin_dir/scripts" "$fake_bin"
cp "$ROOT/target/debug/amux-rs" "$release_dir/amux-rs"
tar -C "$WORK_DIR/release" -czf "$WORK_DIR/$archive" "$package"
cp "$ROOT/VERSION" "$ROOT/bin/amux" "$plugin_dir/"
cp "$ROOT/bin/amux" "$plugin_dir/bin/"
cp "$ROOT/scripts/ensure-runtime.sh" "$plugin_dir/scripts/"

cat >"$fake_bin/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
while [ "$#" -gt 0 ]; do
	if [ "$1" = "--dir" ]; then
		cp "$AMUX_TEST_ARCHIVE" "$2/$AMUX_TEST_ARCHIVE_NAME"
		exit 0
	fi
	shift
done
exit 1
SH
chmod +x "$fake_bin/gh"

export AMUX_TEST_ARCHIVE="$WORK_DIR/$archive"
export AMUX_TEST_ARCHIVE_NAME="$archive"
PATH="$fake_bin:$PATH" "$plugin_dir/bin/amux" --version | grep -Fx "amux $version" >/dev/null
test -x "$plugin_dir/bin/amux-rs"
printf 'ok\n'
