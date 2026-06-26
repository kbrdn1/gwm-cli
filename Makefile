.PHONY: help fetch build build-release run tui install fmt fmt-check lint clippy test test-verbose test-release test-watch test-minimal-path ci doctor doctor-release audit msrv changelog-rc-check clean

SHELL := /bin/bash
CARGO ?= cargo
GWM_ENV := GWM_NO_GLOBAL_CONFIG=1
TAG ?=

.DEFAULT_GOAL := help

# Colors
GREEN = \033[0;32m
YELLOW = \033[0;33m
RED = \033[0;31m
BLUE = \033[0;34m
BOLD = \033[1m
NC = \033[0m # No Color

# Help command
help: ## Show this help
	@printf "${YELLOW}${BOLD}Available commands:${NC}\n"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' Makefile | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "${GREEN}make %-20s${NC} %s\n", $$1, $$2}'

# =============================================================================
# Setup
# =============================================================================

fetch: ## Fetch Cargo dependencies
	@printf "${YELLOW}Fetching Cargo dependencies...${NC}\n"
	$(CARGO) fetch
	@printf "${GREEN}Dependencies fetched.${NC}\n"

install: ## Install local gwm binary
	@printf "${YELLOW}Installing local gwm binary...${NC}\n"
	$(CARGO) install --path . --force
	@printf "${GREEN}Local gwm binary installed.${NC}\n"

# =============================================================================
# Development
# =============================================================================

run: ## Show CLI help via cargo
	@printf "${YELLOW}Running gwm CLI help...${NC}\n"
	$(GWM_ENV) $(CARGO) run -- --help
	@printf "${GREEN}CLI help completed.${NC}\n"

tui: ## Open the gwm TUI via cargo
	@printf "${YELLOW}Opening gwm TUI...${NC}\n"
	$(GWM_ENV) $(CARGO) run --

doctor: ## Run gwm doctor via cargo
	@printf "${YELLOW}Running gwm doctor...${NC}\n"
	$(GWM_ENV) $(CARGO) run -- doctor
	@printf "${GREEN}gwm doctor completed.${NC}\n"

doctor-release: ## Build release binary and run gwm doctor
	@printf "${YELLOW}Building release binary for doctor...${NC}\n"
	$(CARGO) build --release --quiet
	@printf "${YELLOW}Running release gwm doctor...${NC}\n"
	$(GWM_ENV) ./target/release/gwm doctor
	@printf "${GREEN}Release gwm doctor completed.${NC}\n"

# =============================================================================
# Build
# =============================================================================

build: ## Build debug binary and library
	@printf "${YELLOW}Building debug binary and library...${NC}\n"
	$(CARGO) build
	@printf "${GREEN}Build completed.${NC}\n"

build-release: ## Build release binary
	@printf "${YELLOW}Building release binary...${NC}\n"
	$(CARGO) build --release
	@printf "${GREEN}Release build completed.${NC}\n"

# =============================================================================
# Code Quality
# =============================================================================

fmt: ## Format Rust sources
	@printf "${YELLOW}Formatting Rust sources...${NC}\n"
	$(CARGO) fmt --all
	@printf "${GREEN}Rust sources formatted.${NC}\n"

fmt-check: ## Check Rust formatting
	@printf "${YELLOW}Checking Rust formatting...${NC}\n"
	$(CARGO) fmt --all -- --check
	@printf "${GREEN}Rust formatting is clean.${NC}\n"

lint: clippy ## Alias for clippy

clippy: ## Run clippy with warnings denied
	@printf "${YELLOW}Running clippy...${NC}\n"
	$(CARGO) clippy --all-targets --all-features -- -D warnings
	@printf "${GREEN}Clippy completed.${NC}\n"

msrv: ## Run clippy incompatible_msrv lint
	@printf "${YELLOW}Checking MSRV compatibility...${NC}\n"
	$(CARGO) clippy --all-targets --all-features -- -W clippy::incompatible_msrv
	@printf "${GREEN}MSRV compatibility check completed.${NC}\n"

# =============================================================================
# Testing
# =============================================================================

test: ## Run the full test suite
	@printf "${YELLOW}Running full test suite...${NC}\n"
	$(GWM_ENV) $(CARGO) test
	@printf "${GREEN}Tests completed.${NC}\n"

test-verbose: ## Run the full test suite with verbose Cargo output
	@printf "${YELLOW}Running full test suite with verbose output...${NC}\n"
	$(GWM_ENV) $(CARGO) test --verbose
	@printf "${GREEN}Verbose tests completed.${NC}\n"

test-release: ## Run the full test suite in release mode
	@printf "${YELLOW}Running release-mode test suite...${NC}\n"
	$(GWM_ENV) $(CARGO) test --release
	@printf "${GREEN}Release-mode tests completed.${NC}\n"

test-watch: ## Run cargo-watch over the test suite
	@printf "${YELLOW}Watching tests with cargo-watch...${NC}\n"
	$(GWM_ENV) $(CARGO) watch -x test

test-minimal-path: ## Run tests under a stripped CI-like PATH
	@printf "${YELLOW}Running tests under stripped PATH...${NC}\n"
	@cargo_path="$$(command -v $(CARGO))"; \
	if [ -z "$$cargo_path" ]; then \
		printf "${RED}cargo not found in PATH${NC}\n" >&2; \
		exit 1; \
	fi; \
	PATH="$$(dirname "$$cargo_path"):/usr/bin:/bin" $(GWM_ENV) $(CARGO) test
	@printf "${GREEN}Stripped-PATH tests completed.${NC}\n"

# =============================================================================
# CI
# =============================================================================

ci: fmt-check clippy build test ## Run local CI checks
	@printf "${GREEN}${BOLD}Local CI checks completed.${NC}\n"

# =============================================================================
# Security & Release Checks
# =============================================================================

audit: ## Run cargo audit
	@printf "${YELLOW}Running cargo audit...${NC}\n"
	$(CARGO) audit
	@printf "${GREEN}Cargo audit completed.${NC}\n"

changelog-rc-check: ## Check Unreleased changelog against previous RC (usage: make changelog-rc-check TAG=vX.Y.Z-rc.N)
	@printf "${YELLOW}Checking RC changelog duplicates...${NC}\n"
	@if [ -z "$(TAG)" ]; then \
		printf "${RED}Error: TAG is required. Usage: make changelog-rc-check TAG=vX.Y.Z-rc.N${NC}\n" >&2; \
		exit 2; \
	fi
	./.github/scripts/check-rc-changelog-dupes.sh "$(TAG)"
	@printf "${GREEN}RC changelog check completed.${NC}\n"

# =============================================================================
# Cleanup
# =============================================================================

clean: ## Remove Cargo build artifacts
	@printf "${YELLOW}Cleaning Cargo build artifacts...${NC}\n"
	$(CARGO) clean
	@printf "${GREEN}Cargo artifacts cleaned.${NC}\n"
