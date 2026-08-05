#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v docker >/dev/null 2>&1; then
	printf 'amux tests: Docker is required; use mise run test-native only for explicit host debugging\n' >&2
	exit 1
fi
if ! docker info >/dev/null 2>&1; then
	printf 'amux tests: Docker is not available; start Docker or use mise run test-native explicitly\n' >&2
	exit 1
fi

# The image contains tools only. The repository is mounted read-only so a test
# cannot modify the checkout. Linux build data stays in project-specific Docker
# volumes and never mixes with the host target directory.
image="$(docker build --quiet --file "$ROOT/Dockerfile.test" "$ROOT")"
project_id="$(printf '%s' "$ROOT" | cksum | awk '{print $1}')"

exec docker run --rm --init \
	--label dev.amux.test-suite=true \
	--workdir /workspace \
	--mount "type=bind,source=$ROOT,target=/workspace,readonly" \
	--mount "type=volume,source=amux-test-target-$project_id,target=/workspace/target" \
	--mount "type=volume,source=amux-test-registry-$project_id,target=/usr/local/cargo/registry" \
	--tmpfs /tmp:exec,mode=1777 \
	--env CARGO_TARGET_DIR=/workspace/target \
	--env RUST_BACKTRACE="${RUST_BACKTRACE:-1}" \
	"$image" "$@"
