SHELL := /bin/bash

.PHONY: check syntax test shellcheck rust-check cog-check package package-check
.NOTPARALLEL:

VERSION ?= $(shell tr -d '\n' < VERSION)
TARGET ?= $(shell rustc -vV | sed -n 's/^host: //p')
PACKAGE := amux-$(VERSION)-$(TARGET)

check:
	bash -n amux.tmux bin/amux tests/*.sh
	bash tests/smoke.sh
	@if command -v shellcheck >/dev/null 2>&1; then \
		shellcheck amux.tmux bin/amux tests/*.sh; \
	else \
		printf '%s\n' 'shellcheck not installed; skipping'; \
	fi
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all-features

syntax:
	bash -n amux.tmux bin/amux tests/*.sh

test:
	bash tests/smoke.sh

shellcheck:
	@if command -v shellcheck >/dev/null 2>&1; then \
		shellcheck amux.tmux bin/amux tests/*.sh; \
	else \
		printf '%s\n' 'shellcheck not installed; skipping'; \
	fi

rust-check:
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all-features

cog-check:
	mise x cocogitto -- cog check --from-latest-tag

package:
	cargo build --release --bin amux-rs
	mkdir -p "dist/$(PACKAGE)/bin" "dist/$(PACKAGE)/docs" "dist/$(PACKAGE)/hooks"
	cp amux.tmux README.md VERSION "dist/$(PACKAGE)/"
	cp bin/amux target/release/amux-rs "dist/$(PACKAGE)/bin/"
	cp -R docs/. "dist/$(PACKAGE)/docs/"
	cp -R hooks/. "dist/$(PACKAGE)/hooks/"
	tar -C dist -czf "dist/$(PACKAGE).tar.gz" "$(PACKAGE)"
	@printf '%s\n' "built dist/$(PACKAGE).tar.gz"

package-check: package
	@package_dir="$$(mktemp -d)"; \
	trap 'rm -rf "$$package_dir"' EXIT; \
	tar -xzf "dist/$(PACKAGE).tar.gz" -C "$$package_dir"; \
	test ! -e "$$package_dir/$(PACKAGE)/Cargo.toml"; \
	"$$package_dir/$(PACKAGE)/bin/amux" --version
