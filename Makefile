# Everyday tasks for Nexus. `make` alone lists them. The scripts in scripts/ do the work, so CI
# and this file run the same commands.

.DEFAULT_GOAL := help
.PHONY: help check fmt fmt-check lint test run run-windowed build package linux-packages appimage test-rpm \
	os-image os-iso os-vm deny hooks-install hooks-run clean catalog catalog-refresh

VERSION := $(shell sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
# The Nexus OS image the os-* targets build and use, and the base they build it on.
OS_IMAGE ?= localhost/nexus-os:minimal
OS_BASE ?= quay.io/fedora/fedora-bootc:44
# The USB stick os-iso makes and os-vm starts, named with Nexus OS's version.
OS_ISO ?= nexus-os-x86_64-$(shell cat os/version.txt).iso

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

linux-packages: package ## The deb, rpm and Arch packages into dist/packages/, checked (Linux)
	NFPM=$${NFPM:-scripts/nfpm.sh} packaging/linux/build-packages.sh dist/nexus-linux-x86_64 $(VERSION) dist/packages
	packaging/linux/check-packages.sh dist/packages

appimage: package ## dist/Nexus-x86_64.AppImage (Linux; needs LINUXDEPLOY, the libmpv action's linuxdeploy)
	scripts/appimage.sh

test-rpm: ## A quick rpm of this checkout for this PC, into dist/test-rpm/ (the rest from the latest release)
	scripts/test-rpm.sh

os-image: ## The Nexus OS image as OS_IMAGE (os/build-image.sh, sudo); RPM=<file> bundles that rpm, not the latest release's
	@if [ -n "$(RPM)" ]; then rm -rf os/nexus-rpm && mkdir -p os/nexus-rpm && cp "$(RPM)" os/nexus-rpm/; \
	else os/fetch-nexus.sh; fi
	BASE=$(OS_BASE) os/build-image.sh $(OS_IMAGE)

os-iso: ## The Nexus OS USB stick, OS_ISO, from OS_IMAGE (os/make-iso.sh; make os-image first)
	BASE=$(OS_BASE) os/make-iso.sh $(OS_IMAGE) $(OS_ISO)

os-vm: ## Start OS_ISO in QEMU as a USB stick, with Secure Boot and a 64 GB disk (os/run-vm.sh)
	os/run-vm.sh $(OS_ISO)

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
