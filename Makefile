SHELL := /bin/bash
# =============================================================================
# Variables
# =============================================================================

.DEFAULT_GOAL := help

.PHONY: help
help: ## Display this help text for Makefile
	@awk 'BEGIN {FS = ":.*##"; printf "\nUsage:\n  make \033[36m<target>\033[0m\n"} /^[a-zA-Z0-9_-]+:.*?##/ { printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

# =============================================================================
# Setup & Installation
# =============================================================================

##@ Setup & Installation

.PHONY: install
install: ## Install rustup components + cargo-deny
	@echo "=> Installing rustup components"
	@rustup component add rustfmt clippy
	@echo "=> Installing cargo-deny"
	@cargo install --locked cargo-deny

# =============================================================================
# Code Quality
# =============================================================================

##@ Code Quality

.PHONY: fmt fmt-check lint test deny check-all ci
fmt: ## Format the workspace
	@cargo fmt --all

fmt-check: ## Check formatting without changing files
	@cargo fmt --all --check

lint: ## Run clippy, denying warnings
	@cargo clippy --all-targets -- -D warnings

test: ## Run the test suite
	@cargo test

deny: ## Check licenses, advisories, bans and sources (cargo-deny); unmaintained advisories warn instead of failing
	@cargo deny check -W unmaintained

check-all: fmt-check lint test deny ## Run every check CI runs

ci: check-all ## Run all checks for CI

# =============================================================================
# Run
# =============================================================================

##@ Run

.PHONY: run demo
run: ## Run the companion
	@cargo run

demo: ## Run the companion against fixture data, no real WoW install or network needed
	@cargo run -- --demo

# =============================================================================
# Packaging
# =============================================================================

##@ Packaging

.PHONY: bundle-macos
bundle-macos: ## Build a release binary and assemble a local SideQuest Companion.app (ad-hoc signed, current arch only)
	@cargo build --release
	@./packaging/macos/bundle.sh "$$(cargo metadata --no-deps --format-version=1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')" target/release/sidequest-companion target/release/sidequest-companion target/bundle

# =============================================================================
# Main
# =============================================================================

##@ Main

.PHONY: clean
clean: ## Remove build output
	@cargo clean
	@rm -rf target/bundle .release
