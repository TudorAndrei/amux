#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runtime="${AMUX_TEST_RUNTIME:-}"

if [ -z "$runtime" ]; then
	if [ "${CI:-}" = "true" ]; then
		runtime="native"
	else
		runtime="container"
	fi
fi

case "$runtime" in
container)
	exec bash "$ROOT/tests/container.sh" "$@"
	;;
native)
	exec bash "$ROOT/tests/run-native.sh" "$@"
	;;
*)
	printf 'amux tests: AMUX_TEST_RUNTIME must be container or native, got %s\n' "$runtime" >&2
	exit 2
	;;
esac
