# VietC Packaging Guide

Tài liệu đóng gói **VietC** (kiến trúc thuần `evdev`/`uinput` với `systemd-logind` `uaccess`, hoàn toàn không phụ thuộc IBus hay Fcitx5).

## Đóng gói toàn bộ bản phát hành (`.deb` + `.tar.gz`)

```bash
make release
```

## Đóng gói riêng lẻ

- **Gói Debian / Ubuntu (`.deb`):**
  ```bash
  make deb
  # Xuất ra: packaging/deb/vietc_<version>-1_amd64.deb
  ```
- **Kiểm thử khói gói `.deb` trong container Ubuntu 24.04 sạch:**
  ```bash
  make test-package
  ```
- **Gói nén Linux tổng quát (`.tar.gz`):**
  ```bash
  make tarball
  # Xuất ra: target/dist/vietc_<version>_linux_amd64.tar.gz
  ```

## Quy tắc phân quyền thiết bị (`70-vietc.rules`)

File [`70-vietc.rules`](70-vietc.rules) sử dụng cơ chế `TAG+="uaccess"` (`GROUP="root", MODE="0600"` cho `/dev/uinput`, đồng thời giữ nguyên `GROUP`/`MODE` mặc định của distro trên `/dev/input/event*` để không ảnh hưởng các công cụ dùng nhóm `input`) để `systemd-logind` tự động cấp ACL cho phiên người dùng đang ngồi trực tiếp tại máy tính (active seat), không cần cấp quyền `root` cho daemon và không cần thêm user vào nhóm `input`.
