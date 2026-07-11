# Convenience targets for xfin.

.PHONY: build test lint fmt fmt-check check dev install

build:
	cargo build --release

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check: fmt-check lint test

# Debug build re-signed with the stable pk-cli-codesign identity so macOS
# keychain "Always Allow" grants survive rebuilds (see cli-common/scripts).
dev:
	cargo build
	@if [ -x "$$HOME/Dev/cli-common/scripts/dev-sign.sh" ]; then \
		"$$HOME/Dev/cli-common/scripts/dev-sign.sh" target/debug/xfin; \
	else echo "cli-common/scripts/dev-sign.sh not found — binary left ad-hoc signed"; fi
