# Checklist sua loi audit

## Phase 1: Data loss va result contract

- [x] Task 1: Chot `InjectResult` contract va them failure test doubles
- [x] Task 2: Propagate loi cua Wayland `wtype` va clipboard fallback
- [x] Task 3: Serialize uinput transaction, xu ly short write va `EINTR`
- [x] Task 4: X11 paste chi success khi selection request duoc phuc vu

### Checkpoint 1

- [x] `cargo test --workspace --all-targets` pass
- [x] `cargo check --workspace --all-targets --all-features` pass
- [x] Khong con duong xoa text, insert fail roi bao success

## Phase 2: Clipboard correctness

- [x] Task 5: Snapshot/restore clipboard theo tung Wayland burst
- [x] Task 6: Bao toan clipboard ownership tren X11
- [x] Khong restore de len clipboard moi cua user
- [x] Khong chay external clipboard I/O trong luc giu state mutex

## Phase 3: FFI va concurrency

- [x] Task 7: Null-check tat ca `dlsym` va cleanup handle khi fail
- [x] Task 8: Bo unsound `unsafe impl Sync for X11Injector`
- [x] Serialize moi operation tren cung Xlib `Display`
- [x] Them tests cho missing symbol va concurrent access boundary

### Checkpoint 2

- [x] `cargo test --workspace --all-targets` pass
- [x] `cargo clippy -p vietc-protocol --all-targets --all-features -- -D warnings` pass
- [x] Khong con unchecked function pointer
- [x] Khong con interior-mutability state duoc share khong dong bo

## Phase 4: Pipe va process lifecycle

- [x] Task 9: Giu partial XRecord pipe frame qua nhieu read
- [x] Task 10: Them readiness timeout cho `vietc-xrecord`
- [x] Kiem tra `fcntl(F_GETFL)` va `fcntl(F_SETFL)`
- [x] Kill va reap child tren moi startup/restart failure
- [x] Drop khong de zombie process

## Phase 5: Privilege va shutdown

- [x] Task 11: Clear/init supplementary groups truoc khi ha UID
- [x] Set GID truoc UID
- [x] Validate desktop session environment
- [x] Refuse fallback chay desktop command bang root
- [x] Task 12: Luu va join clipboard restorer thread
- [x] Khong restore clipboard sau khi injector Drop hoan tat

### Checkpoint 3

- [x] `cargo test --workspace --all-targets` pass
- [x] `cargo check --workspace --all-targets --all-features` pass
- [x] Khong con detached worker
- [x] Khong con unbounded blocking startup path

## Phase 6: Runtime verification

- [x] Task 13: Wayland `wtype` success test
- [x] Wayland `wtype` fail, clipboard fallback success test
- [x] Wayland ca hai backend fail test
- [x] Chromium Shift-prefix modifier-state test
- [x] Physical Shift held during injection test
- [x] X11 SelectionRequest success test
- [x] X11 timeout/failure test
- [x] X11 clipboard preservation test
- [x] Wayland clipboard preservation test
- [x] User clipboard race test
- [x] Partial pipe frame test
- [x] Helper hang va early-exit tests
- [x] Repeated replacement no-drop/no-duplicate test
- [x] Shutdown/restart lifecycle test

## Phase 7: Quality gate

- [x] Task 14: Xu ly tat ca Clippy warnings
- [x] `cargo fmt --all -- --check` pass
- [x] `cargo test --workspace --all-targets` pass
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass
- [x] CI chay format, test va Clippy tren moi PR
- [x] Privileged runtime tests co job va timeout rieng
- [ ] Runtime matrix X11 pass
- [ ] Runtime matrix GNOME Wayland pass
- [ ] Runtime matrix KDE Wayland pass
- [ ] Chromium/Electron pass
- [ ] GTK, Qt va terminal smoke tests pass

## Release gate

- [ ] Tat ca Critical findings da dong
- [ ] Tat ca High findings da dong hoac duoc chap nhan co ly do
- [ ] Khong silent injection success
- [ ] Khong input/clipboard data loss
- [ ] Khong stuck modifier
- [ ] Khong unchecked FFI
- [ ] Khong unsound `Send/Sync`
- [ ] Khong child/thread leak
- [ ] Tat ca automated va runtime verification da pass