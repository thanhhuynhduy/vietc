# VietC

Bộ gõ tiếng Việt cho Linux, hỗ trợ Wayland và X11, phát triển bằng Rust.

Dự án kế thừa từ mã nguồn VietC gốc, nâng cấp toàn diện về thuật toán xử lý tiếng Việt / Anh song ngữ (học hỏi từ [Ngó Sen](https://github.com/ngosen/ngosen) và [bamboo-core](https://github.com/bamboo-vi/bamboo-core)), kiểm thử tự động, bảo toàn clipboard, và tăng cường bảo mật.

![License](https://img.shields.io/badge/license-MIT-blue)
![Language](https://img.shields.io/badge/language-Rust-orange)

## ✨ Tính năng

- **Hỗ trợ cả Wayland và X11** — hoạt động trên mọi môi trường desktop Linux phổ biến.
- **Bộ gõ tiếng Việt thông minh** — nhập liệu bằng kiểu Telex, VNI, VIQR.
- **Xử lý song ngữ Việt/Anh** — tự động nhận diện ngữ cảnh và ưu tiên từ điển.
- **Bảo toàn clipboard** — không làm mất nội dung clipboard khi thao tác gõ.
- **Bảo mật nâng cao** — quy tắc truy cập thiết bị đầu vào giới hạn phiên người dùng, không cấp quyền world.
- **Kiểm thử tự động** — bộ test lớn bảo đảm chất lượng xử lý tiếng Việt.
- **Giao diện Web** — bảng điều khiển trực quan để cấu hình và theo dõi.
- **Cài đặt dễ dàng** — script cài đặt tự động, hỗ trợ Nix flake.

## 🚀 Cài đặt

### Cài đặt nhanh
```bash
curl -fsSL https://raw.githubusercontent.com/thanhhuynhduy/vietc/main/install.sh | bash
```

Hoặc tải về và chạy:
```bash
wget https://raw.githubusercontent.com/thanhhuynhduy/vietc/main/install.sh
chmod +x install.sh
sudo ./install.sh
```

**Lưu ý:** Trình cài đặt sẽ thiết lập quy tắc udev và ACL để cấp quyền truy cập thiết bị đầu vào cho người dùng hiện tại. Sau khi cài đặt, khởi động lại hoặc chạy `sudo udevadm control --reload-rules && sudo udevadm trigger`.

### Cài đặt thủ công
```bash
git clone https://github.com/thanhhuynhduy/vietc.git
cd vietc
cargo build --release
sudo cp target/release/vietc /usr/local/bin/
sudo cp target/release/vietc-daemon /usr/local/bin/
# Cài đặt quy tắc udev & systemd service: chạy script install.sh hoặc làm thủ công
```

### Sử dụng Nix
```bash
nix build github:thanhhuynhduy/vietc
# hoặc dùng flake
nix run github:thanhhuynhduy/vietc
```

## 🖥️ Sử dụng

Sau khi cài đặt, bộ gõ sẽ tự khởi động cùng hệ thống. Mặc định:
- **Phím tắt bật/tắt bộ gõ:** `Shift + Alt`
- **Giao diện Web:** [http://localhost:1999](http://localhost:1999) (có thể thay đổi cổng trong cấu hình)

Để chạy thủ công:
```bash
vietc-daemon
```

## 🛠️ Xây dựng từ mã nguồn

Yêu cầu:
- Rust 1.75+
- `cargo` và các công cụ hệ thống (pkg-config, libxdo, libevdev, v.v.)

```bash
git clone https://github.com/thanhhuynhduy/vietc.git
cd vietc
cargo build --release
```

Chạy kiểm thử:
```bash
cargo test --workspace
```

## ⚙️ Cấu hình

Tệp cấu hình mặc định tại `~/.config/vietc/config.toml`. Ví dụ:
```toml
[engine]
input_method = "Telex"          # Telex, VNI, VIQR
free_marking = true              # Cho phép bỏ dấu tự do
spell_check = true               # Kiểm tra chính tả tiếng Anh

[shortcuts]
toggle_input = "Shift+Alt"

[web]
port = 1999
```

## 🌐 Giao diện Web

Truy cập [http://localhost:1999](http://localhost:1999) để xem trạng thái bộ gõ, thay đổi kiểu gõ, và quản lý từ điển.

Giao diện được xây dựng bằng React + TypeScript, đóng gói cùng daemon.

## 🤝 Đóng góp

Đóng góp luôn được chào đón!
1. Fork repo
2. Tạo nhánh tính năng (`git checkout -b feature/tinh-nang`)
3. Commit thay đổi (`git commit -m 'Thêm tính năng mới'`)
4. Push (`git push origin feature/tinh-nang`)
5. Mở Pull Request

## 📄 Giấy phép

Phân phối theo **MIT License**. Xem [LICENSE](LICENSE) để biết chi tiết.

---

Dự án được phát triển bởi cộng đồng vì cộng đồng Linux Việt Nam.
