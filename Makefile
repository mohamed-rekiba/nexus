# Everyday tasks for Nexus. `make` alone lists them. The scripts in scripts/ do the work, so CI
# and this file run the same commands.

.DEFAULT_GOAL := help
.PHONY: help check fmt fmt-check lint test run run-windowed package deny hooks-install hooks-run clean \
	catalog catalog-refresh

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-16s %s\n", $$1, $$2}'

check: ## Format check, clippy (warnings are errors) and every test, as CI runs them
	scripts/check.sh

fmt: ## Format the code
	cargo fmt --all

fmt-check: ## Fail when the code isn't formatted
	cargo fmt --all --check

lint: ## Clippy on every crate and test, warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

test: ## Every test
	cargo test --workspace

run: ## Run the launcher
	cargo run -p nexus-app

run-windowed: ## Run the launcher in a normal window
	cargo run -p nexus-app -- --windowed

build: ## Build the launcher
	cargo build -p nexus-app --release

package: ## Release build into dist/nexus-<os>-<arch>/
	scripts/package.sh

deny: ## Advisories, licences and sources of every dependency (needs cargo-deny)
	cargo deny check

hooks-install: ## Install the git hooks from .pre-commit-config.yaml (needs pre-commit)
	pre-commit install --hook-type pre-commit

hooks-run: ## Run the git hooks from .pre-commit-config.yaml (needs pre-commit)
	pre-commit run --all-files

catalog: ## Read the forums again (Chrome opens) into assets/catalog/; CONSOLE=pc for one catalog, STEAM=1 adds Steam data
	scripts/catalog/update-catalogs.sh $(if $(CONSOLE),--console $(CONSOLE))

catalog-refresh: ## Update assets/catalog/ from the forums' feed and API, without a browser; CONSOLE=pc for one catalog
	scripts/catalog/update-catalogs.sh --refresh $(if $(CONSOLE),--console $(CONSOLE))

clean: ## Remove the build output and dist/ (with the catalog backups)
	cargo clean
	rm -rf dist target
