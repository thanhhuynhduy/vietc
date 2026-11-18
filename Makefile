.PHONY: build build-x11 build-ui build-dev test test-ci-smoke test-runtime-targets test-runtime-matrix test-properties-exhaustive test-cli test-package run run-x11 run-release clean install install-x11 install-ui install-config deb tarball release fmt lint tree

# Build .deb + tarball release artifacts
release:
	bash packaging/release.sh

# Build .deb package
deb:
	VERSION=$$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1) && \
	bash packaging/deb/build-deb.sh "$$VERSION"

# Build generic Linux tarball
tarball:
	bash packaging/build-tarball.sh

# Build and exercise the Debian package in a clean Ubuntu container
test-package: deb
	PACKAGE=$$(find packaging/deb -maxdepth 1 -type f -name 'vietc_*_amd64.deb' -print -quit) && \
	test -n "$$PACKAGE" && \
	bash scripts/package-smoke-test.sh "$$PACKAGE"

# Build core crates (default features include x11)
build:
	cargo build --release

# Build explicitly with X11 support
build-x11:
	cargo build --release --features x11

# Build tray icon UI (requires libdbus-1-dev)
build-ui:
	cargo build --release --manifest-path ui/Cargo.toml

# Build debug
build-dev:
	cargo build

# Run all workspace tests headlessly
CARGO_FLAGS ?= --locked
HEADLESS = env -u DISPLAY -u WAYLAND_DISPLAY -u VIETC_RUNTIME_TESTS -u VIETC_CONTROLLED_DISPLAY

test:
	$(HEADLESS) cargo test --workspace
	$(HEADLESS) cargo test --manifest-path ui/Cargo.toml

# Explicit targets keep live device/desktop tests out, even when run as root.
test-ci-smoke:
	python3 scripts/check_source_size.py
	python3 scripts/test_installer.py
	python3 scripts/test_build_targets.py
	bash -n install.sh uninstall.sh packaging/release.sh packaging/build-tarball.sh packaging/deb/build-deb.sh
	$(HEADLESS) cargo test --workspace --lib --bins $(CARGO_FLAGS)
	$(HEADLESS) VIETC_PROPERTY_PROFILE=smoke cargo test -p vietc-engine --tests $(CARGO_FLAGS)
	$(HEADLESS) cargo test -p vietc-daemon --test deterministic_ci --test typing_corpus $(CARGO_FLAGS) -- --test-threads=1
	$(HEADLESS) cargo test -p vietc-daemon --test daemon_suite runtime::deterministic_ $(CARGO_FLAGS) -- --test-threads=1
	$(HEADLESS) cargo test --workspace --doc $(CARGO_FLAGS)
	$(HEADLESS) cargo test --manifest-path ui/Cargo.toml $(CARGO_FLAGS)

# Opt-in controlled runtime targets; requires a disposable GUI session and target dependencies
test-runtime-targets:
	VIETC_RUNTIME_TESTS=1 VIETC_CONTROLLED_DISPLAY=1 cargo test -p vietc-daemon --test daemon_suite runtime::privileged_runtime_targets_execute_controlled_scenarios $(CARGO_FLAGS) -- --ignored --nocapture --test-threads=1

# Opt-in privileged runtime capability evidence; does not launch unvalidated desktop fixtures
test-runtime-matrix:
	VIETC_RUNTIME_TESTS=1 cargo test -p vietc-daemon --test daemon_suite runtime::privileged_runtime_matrix_records_exact_ready_and_skip_evidence $(CARGO_FLAGS) -- --ignored --nocapture --test-threads=1

# Opt-in exhaustive deterministic property budget
test-properties-exhaustive:
	$(HEADLESS) VIETC_PROPERTY_PROFILE=exhaustive cargo test -p vietc-engine --test state_machine_properties -- --nocapture

# Run the interactive CLI test harness
test-cli:
	cargo run --bin vietc-cli

# Run the daemon
run: build-dev
	cargo run --bin vietc

# Run daemon with X11 support
run-x11: build-dev
	cargo run --bin vietc --features x11

# Run daemon in release mode
run-release: build
	target/release/vietc

# Install to /usr/local/bin
install: build
	sudo install -m 755 target/release/vietc /usr/local/bin/vietc
	@echo "Installed vietc to /usr/local/bin/"

# Install with X11 support
install-x11: build-x11
	sudo install -m 755 target/release/vietc /usr/local/bin/vietc
	@echo "Installed vietc (with X11) to /usr/local/bin/"

# Install tray icon UI
install-ui: build-ui
	sudo install -m 755 ui/target/release/vietc-tray /usr/local/bin/vietc-tray
	@echo "Installed vietc-tray to /usr/local/bin/"

# Install config to user dir
install-config:
	mkdir -p ~/.config/vietc
	cp vietc.toml ~/.config/vietc/config.toml
	@echo "Config installed to ~/.config/vietc/config.toml"

# Clean build artifacts
clean:
	cargo clean
	@if [ -d ui ]; then cargo clean --manifest-path ui/Cargo.toml; fi

# Format code
fmt:
	cargo fmt --all
	cargo fmt --manifest-path ui/Cargo.toml --all

# Lint
lint:
	$(HEADLESS) cargo clippy --workspace --all-targets --all-features -- -D warnings
	$(HEADLESS) cargo clippy --manifest-path ui/Cargo.toml --all-targets -- -D warnings

# Show project structure
tree:
	@find . -type f \( -name "*.rs" -o -name "*.toml" \) | grep -v target | sort
