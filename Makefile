SHELL := /usr/bin/env bash

.PHONY: check syntax test shellcheck

check: syntax test shellcheck

syntax:
	bash -n bin/amux lib/*.sh scripts/*.sh tests/*.sh

test:
	tests/smoke.sh

shellcheck:
	@if command -v shellcheck >/dev/null 2>&1; then \
		shellcheck bin/amux lib/*.sh scripts/*.sh tests/*.sh; \
	else \
		printf '%s\n' 'shellcheck not installed; skipping'; \
	fi

