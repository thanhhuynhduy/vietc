# Bằng chứng phát hành

Mỗi candidate và stable release phải kèm `RELEASE-EVIDENCE.md`, `BUILD-METADATA` và `SHA256SUMS`.

## Gate tự động

Pipeline phải ghi nhận:

- commit SHA và tag;
- Rust/Cargo version;
- distro và kernel của build runner;
- kết quả `rustfmt`, Clippy và headless smoke suite;
- kết quả cài, kiểm tra, remove và purge gói Debian trong Ubuntu sạch;
- checksum SHA-256 của mọi artifact.

## Gate thủ công trước khi promotion

Candidate trên nhánh `next` phải được kiểm tra tối thiểu trên:

1. GNOME Wayland;
2. COSMIC Wayland;
3. một phiên X11.

Với mỗi môi trường, ghi kết quả cài mới, nâng cấp, Telex, VNI, password/PIN bypass, clipboard restore và rollback.

Không promotion sang `main` nếu còn ô `Pending`, trừ khi limitation được ghi rõ trong release notes và người duyệt chấp thuận.

## Rollback

Lưu tag candidate, checksum artifact đã kiểm tra và checksum binary trước khi thay thế. Dùng `scripts/candidate-deploy.sh rollback` để phục hồi candidate cục bộ nếu kiểm thử desktop thất bại.