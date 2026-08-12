# Convenience targets for xfin.

BIN := xfin

.PHONY: build test lint fmt fmt-check check smoke verify dev install

build: SIGN_TARGET = target/release/$(BIN)
build:
	cargo build --release
	@$(SIGN)

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check: fmt-check lint test

# Offline smoke: drive the real binary over the surfaces that must never need
# a network call or the keychain. Catches runtime clap panics a unit test can't
# see (a subcommand flag colliding with a global, a bad value parser).
smoke:
	cargo run --quiet -- --version >/dev/null
	cargo run --quiet -- --help >/dev/null
	cargo run --quiet -- billing --help >/dev/null
	cargo run --quiet -- billing download --help >/dev/null
	@# Missing id with no --all is a usage error, and it must be decided before
	@# anything touches the keychain or the network.
	@if cargo run --quiet -- billing download >/dev/null 2>&1; then \
		echo "smoke: expected 'billing download' with no id to fail"; exit 1; \
	fi
	@echo "smoke: ok"

# The family CI gate (piekstra-cli/1).
verify: fmt-check lint test smoke

# Debug build re-signed with the stable pk-cli-codesign identity so macOS
# keychain "Always Allow" grants survive rebuilds (see cli-common/scripts).
dev: SIGN_TARGET = target/debug/$(BIN)
dev:
	cargo build
	@$(SIGN)

# `cargo install` ad-hoc signs, giving the binary a new code identity every
# time. macOS scopes keychain "Always Allow" grants to that identity, so an
# unsigned reinstall silently revokes them. Re-signing with the stable shared
# identity keeps one grant valid across every future install.
install: SIGN_TARGET = $${CARGO_INSTALL_ROOT:-$$HOME/.cargo}/bin/$(BIN)
install:
	cargo install --path . --force
	@$(SIGN)

# Shared re-signing step. No-ops with a note when the helper or identity is
# absent (CI, Linux, a machine that hasn't run setup-dev-signing.sh).
define SIGN
if [ -x "$$HOME/Dev/cli-common/scripts/dev-sign.sh" ]; then \
	"$$HOME/Dev/cli-common/scripts/dev-sign.sh" "$(SIGN_TARGET)"; \
else echo "cli-common/scripts/dev-sign.sh not found — $(SIGN_TARGET) left ad-hoc signed"; fi
endef
