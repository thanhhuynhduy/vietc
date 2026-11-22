# Dboard Packaging Guide

Tài liệu đóng gói **Dboard** (kiến trúc thuần `evdev`/`uinput` với `systemd-logind` `uaccess`, hoàn toàn không phụ thuộc IBus hay Fcitx5).

## Đóng gói toàn bộ bản phát hành (`.deb` + `.tar.gz`)

```bash
make release
```

## Đóng gói riêng lẻ

- **Gói Debian / Ubuntu (`.deb`):**
  ```bash
  make deb
  # Xuất ra: packaging/deb/dboard_<version>-1_amd64.deb
  ```
- **Kiểm thử khói gói `.deb` trong container Ubuntu 24.04 sạch:**
  ```bash
  make test-package
  ```
- **Gói nén Linux tổng quát (`.tar.gz`):**
  ```bash
  make tarball
  # Xuất ra: target/dist/dboard_<version>_linux_amd64.tar.gz
  ```

## Quy tắc phân quyền thiết bị (`70-dboard.rules`)

File [`70-dboard.rules`](70-dboard.rules) sử dụng cơ chế `TAG+="uaccess"` (`GROUP="root", MODE="0600"` cho `/dev/uinput`, và chỉ gắn `TAG+="uaccess"` cho thiết bị bàn phím/chuột/touchpad `ID_INPUT_KEYBOARD`/`ID_INPUT_MOUSE`/`ID_INPUT_TOUCHPAD` trên `/dev/input/event*`, giữ nguyên `GROUP`/`MODE` mặc định của distro để không ảnh hưởng gamepad, cảm biến hay các công cụ dùng nhóm `input`) để `systemd-logind` tự động cấp ACL cho phiên người dùng đang ngồi trực tiếp tại máy tính (active seat), không cần cấp quyền `root` cho daemon và không cần thêm user vào nhóm `input`.
