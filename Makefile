.PHONY: build build-ui build-popup build-dev test test-ci-smoke test-soak test-runtime-targets test-runtime-matrix test-properties-exhaustive test-cli test-package run run-release clean install install-ui install-popup install-config deb tarball release fmt lint tree

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
	PACKAGE=$$(find packaging/deb -maxdepth 1 -type f -name 'dboard_*_amd64.deb' -print -quit) && \
	test -n "$$PACKAGE" && \
	bash scripts/package-smoke-test.sh "$$PACKAGE"

# Build core crates (Pure Wayland)
build:
	cargo build --release

# Build tray icon UI (requires libdbus-1-dev)
build-ui:
	cargo build --release --manifest-path ui/Cargo.toml

# Build optional Wayland floating candidate strip popup
build-popup:
	cargo build --release --manifest-path ui-popup/Cargo.toml

# Build debug
build-dev:
	cargo build

# Run all workspace tests headlessly
CARGO_FLAGS ?= --locked
HEADLESS = env -u DISPLAY -u WAYLAND_DISPLAY -u DBOARD_RUNTIME_TESTS -u DBOARD_CONTROLLED_DISPLAY

test:
	$(HEADLESS) cargo test --workspace
	$(HEADLESS) cargo test --manifest-path ui/Cargo.toml
	$(HEADLESS) cargo test --manifest-path ui-popup/Cargo.toml

# Explicit targets keep live device/desktop tests out, even when run as root.
test-ci-smoke:
	python3 scripts/check_source_size.py
	python3 scripts/test_installer.py
	python3 scripts/test_build_targets.py
	bash -n install.sh uninstall.sh packaging/release.sh packaging/build-tarball.sh packaging/deb/build-deb.sh
	$(HEADLESS) cargo test --workspace --lib --bins $(CARGO_FLAGS)
	$(HEADLESS) DBOARD_PROPERTY_PROFILE=smoke cargo test -p dboard-engine --tests $(CARGO_FLAGS)
	$(HEADLESS) cargo test -p dboard-daemon --test deterministic_ci --test typing_corpus $(CARGO_FLAGS) -- --test-threads=1
	$(HEADLESS) cargo test -p dboard-daemon --test daemon_suite runtime::deterministic_ $(CARGO_FLAGS) -- --test-threads=1
	$(HEADLESS) cargo test --workspace --doc $(CARGO_FLAGS)
	$(HEADLESS) cargo test --manifest-path ui/Cargo.toml $(CARGO_FLAGS)
	$(HEADLESS) cargo test --manifest-path ui-popup/Cargo.toml $(CARGO_FLAGS)

# Opt-in controlled runtime targets; requires a disposable GUI session and target dependencies
test-runtime-targets:
	DBOARD_RUNTIME_TESTS=1 DBOARD_CONTROLLED_DISPLAY=1 cargo test -p dboard-daemon --test daemon_suite runtime::privileged_runtime_targets_execute_controlled_scenarios $(CARGO_FLAGS) -- --ignored --nocapture --test-threads=1

# Opt-in privileged runtime capability evidence; does not launch unvalidated desktop fixtures
test-runtime-matrix:
	DBOARD_RUNTIME_TESTS=1 cargo test -p dboard-daemon --test daemon_suite runtime::privileged_runtime_matrix_records_exact_ready_and_skip_evidence $(CARGO_FLAGS) -- --ignored --nocapture --test-threads=1

# Opt-in exhaustive deterministic property budget
test-properties-exhaustive:
	$(HEADLESS) DBOARD_PROPERTY_PROFILE=exhaustive cargo test -p dboard-engine --test state_machine_properties -- --nocapture

# Release-mode 100,000-keystroke soak & latency benchmark (p50/p95/p99)
test-soak:
	$(HEADLESS) cargo test --release -p dboard-engine --test soak_perf -- --nocapture

# Run the interactive CLI test harness
test-cli:
	cargo run --bin dboard-cli

# Run the daemon
run: build-dev
	cargo run --bin dboard

# Run daemon in release mode
run-release: build
	target/release/dboard

# Install to /usr/bin (matches dboard.service ExecStart)
install: build
	sudo install -m 755 target/release/dboard /usr/bin/dboard-daemon
	@echo "Installed dboard-daemon to /usr/bin/"

# Install tray icon UI
install-ui: build-ui
	sudo install -m 755 ui/target/release/dboard-tray /usr/bin/dboard-tray
	@echo "Installed dboard-tray to /usr/bin/"

# Install optional floating candidate strip popup
install-popup: build-popup
	sudo install -m 755 ui-popup/target/release/dboard-popup /usr/bin/dboard-popup
	@echo "Installed dboard-popup to /usr/bin/"

# Install config to user dir
install-config:
	mkdir -p ~/.config/dboard
	cp dboard.toml ~/.config/dboard/config.toml
	@echo "Config installed to ~/.config/dboard/config.toml"

# Clean build artifacts
clean:
	cargo clean
	@if [ -d ui ]; then cargo clean --manifest-path ui/Cargo.toml; fi
	@if [ -d ui-popup ]; then cargo clean --manifest-path ui-popup/Cargo.toml; fi

# Format code
fmt:
	cargo fmt --all
	cargo fmt --manifest-path ui/Cargo.toml --all
	cargo fmt --manifest-path ui-popup/Cargo.toml --all

# Lint
lint:
	$(HEADLESS) cargo clippy --workspace --all-targets --all-features -- -D warnings
	$(HEADLESS) cargo clippy --manifest-path ui/Cargo.toml --all-targets -- -D warnings
	$(HEADLESS) cargo clippy --manifest-path ui-popup/Cargo.toml --all-targets -- -D warnings

# Show project structure
tree:
	@find . -type f \( -name "*.rs" -o -name "*.toml" \) | grep -v target | sort
