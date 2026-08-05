#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Optional arguments provide a narrow debug command inside the same container.
# The default path is the complete test suite.
if [ "$#" -gt 0 ]; then
	exec "$@"
fi

cargo test --all-features
cargo build --bin amux-rs
bash tests/smoke.sh
bash tests/tpm-bootstrap.sh
