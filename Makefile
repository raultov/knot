.PHONY: all fmt fmt-check clippy test dupes dupes-cleanup check check-all e2e build install help

# Default target: run all mandatory quality gates
all: check

# Auto-fix code formatting
fmt:
	cargo fmt

# Verify code formatting
fmt-check:
	cargo fmt -- --check

# Run linter checks
clippy:
	cargo clippy --all-targets --all-features -- -D warnings

# Run unit tests
test:
	cargo test --lib --all-features

# Check for code duplication (requires cargo-dupes)
dupes:
	@test -x "$$(command -v cargo-dupes)" || (echo "cargo-dupes not found. Installing..." && cargo install cargo-dupes --version 0.2.1 --locked)
	cargo dupes check

# Show stale duplication suppressions
dupes-cleanup:
	@test -x "$$(command -v cargo-dupes)" || (echo "cargo-dupes not found. Installing..." && cargo install cargo-dupes --version 0.2.1 --locked)
	cargo dupes cleanup --dry-run

# Run all local quality gates sequentially (fmt, clippy, unit tests, dupes)
check: fmt-check clippy test dupes

# Build release binaries
build:
	cargo build --release --all-features

# Detect OS binary extension
ifeq ($(OS),Windows_NT)
    EXT := .exe
else
    EXT :=
endif

# Install release binaries to CARGO_HOME/bin (defaults to ~/.cargo/bin)
CARGO_HOME ?= $(HOME)/.cargo
DESTDIR ?= $(CARGO_HOME)/bin

install: build
	@mkdir -p "$(DESTDIR)"
	@cp -f target/release/knot-indexer$(EXT) "$(DESTDIR)/knot-indexer$(EXT)"
	@cp -f target/release/knot$(EXT) "$(DESTDIR)/knot$(EXT)"
	@cp -f target/release/knot-mcp$(EXT) "$(DESTDIR)/knot-mcp$(EXT)"
	@echo "Installed binaries to $(DESTDIR):"
	@echo "  - knot-indexer$(EXT)"
	@echo "  - knot$(EXT)"
	@echo "  - knot-mcp$(EXT)"

# Run fast E2E integration tests (requires Docker + running databases)
e2e:
	./tests/run_all_e2e_fast.sh

# Run all local quality gates AND all E2E integration tests
check-all: check e2e

# Display available make targets
help:
	@echo "Available targets:"
	@echo "  make (or make all) - Default target: alias for 'make check'"
	@echo "  make check         - Run all local quality gates (fmt-check, clippy, test, dupes)"
	@echo "  make check-all     - Run all local quality gates AND E2E integration tests"
	@echo "  make fmt           - Auto-fix code formatting with cargo fmt"
	@echo "  make fmt-check     - Verify code formatting with cargo fmt -- --check"
	@echo "  make clippy        - Run linter checks with cargo clippy"
	@echo "  make test          - Run unit tests"
	@echo "  make dupes         - Run code duplication check with cargo-dupes (auto-installs if missing)"
	@echo "  make dupes-cleanup - Show stale duplication suppressions"
	@echo "  make build         - Build release binaries"
	@echo "  make install       - Build release binaries and copy them to ~/.cargo/bin"
	@echo "  make e2e           - Run all E2E integration tests (requires Docker)"
