# Implementation Plan: Global Telex Raw-Keystroke Recomposition

## 1. Mục tiêu

Thay cơ chế xử lý Telex tăng dần theo từng nhánh đặc biệt bằng một pipeline tái phân tích âm tiết từ toàn bộ raw keystrokes sau mỗi phím.

Pipeline mới phải:

1. Giữ raw keystrokes làm nguồn sự thật duy nhất cho âm tiết đang nhập.
2. Phân tách phụ âm đầu, nucleus, phụ âm cuối và các trigger Telex.
3. Chuẩn hóa mark và tone không phụ thuộc thứ tự trigger.
4. Dựng lại toàn bộ output của âm tiết sau mỗi phím.
5. Giữ đầy đủ undo, backspace, auto-restore tiếng Anh, macro và casing.
6. Không thay đổi hợp đồng giữa engine với IBus, Wayland, X11 hoặc uinput.

## 2. Vấn đề hiện tại

`BambooEngine` đang sửa trực tiếp `composition` theo thứ tự nhận phím. Kết quả phụ thuộc trạng thái trung gian và vị trí trigger:

- `dduocwj` có thể hoạt động nhưng `dduojwc` từng không hoạt động.
- `neesu` có thể hoạt động nhưng `neues` từng không hoạt động.
- Mỗi ngoại lệ mới đang được bổ sung bằng backtrack hoặc nhánh xử lý riêng.
- Các nhánh `uo + w`, `ua + w`, double-vowel, tone placement và undo dùng quy tắc cục bộ khác nhau.
- Việc mở rộng backtrack có nguy cơ sửa nhầm các từ như `readme`.

Đây là vấn đề mô hình trạng thái, không chỉ là thiếu một vài pattern.

## 3. Định nghĩa “xử lý toàn cục”

Bản thiết kế được xem là toàn cục khi mọi chuỗi phím hợp lệ cho cùng một âm tiết được đưa qua cùng một pipeline phân tích, thay vì có nhánh hard-code cho từng từ.

Ví dụ, các hoán vị hợp lệ của mark và tone phải hội tụ về cùng output:

| Âm tiết | Input đại diện | Output |
|---|---|---|
| được | `dduocwj`, `dduojwc`, các hoán vị được chính sách cho phép | `được` |
| nếu | `neesu`, `neues`, các hoán vị được chính sách cho phép | `nếu` |
| đường | trigger mark/tone ở các vị trí hợp lệ | `đường` |
| tiếng | trigger `ee` và tone ở thứ tự hợp lệ | `tiếng` |

“Toàn cục” không có nghĩa mọi hoán vị ký tự đều được chấp nhận. Parser vẫn phải có ranh giới âm tiết và chính sách rõ ràng để tránh biến từ tiếng Anh thành tiếng Việt ngoài ý muốn.

## 4. Kiến trúc đề xuất

### 4.1 Raw syllable state

Tạo một state riêng cho âm tiết đang soạn:

```rust
struct RawSyllable {
    keys: Vec<RawKey>,
    case_shape: CaseShape,
}

enum RawKey {
    Letter(char),
    ToneTrigger(char),
    MarkTrigger(char),
}
```

Raw keys không bị xóa hoặc thay thế khi render. Backspace xóa raw key cuối rồi chạy lại parser.

### 4.2 Parsed syllable

Parser tạo mô hình trung gian có invariant rõ ràng:

```rust
struct ParsedSyllable {
    onset: Vec<char>,
    nucleus: Vec<VowelUnit>,
    coda: Vec<char>,
    tone: Option<Tone>,
    stroke_d: bool,
    consumed_triggers: Vec<usize>,
    literal_keys: Vec<usize>,
}
```

`VowelUnit` giữ base vowel và mark độc lập:

```rust
struct VowelUnit {
    base: char,
    mark: Option<VowelMark>,
}
```

Tone không được gắn trực tiếp vào ký tự cho tới bước render.

### 4.3 Pipeline sau mỗi phím

```text
raw keystrokes
    -> xác định boundary
    -> tokenize letters và triggers
    -> parse onset/nucleus/coda
    -> resolve mark transformations
    -> resolve tone
    -> validate Vietnamese syllable
    -> render Unicode output
    -> tính Replace diff với output trước
```

### 4.4 Ranh giới âm tiết

Recomposition chỉ chạy trong âm tiết hiện tại. Boundary gồm:

- khoảng trắng, tab, newline;
- dấu câu;
- flush do daemon hoặc ứng dụng đích;
- paste boundary;
- reset, đổi input method hoặc tắt engine.

Không được tìm mark/tone qua boundary.

### 4.5 Giải quyết mark

Mark resolver áp dụng trên nucleus đã parse, không scan tùy ý toàn buffer.

Các nhóm chính:

- `aa -> â`
- `aw -> ă`
- `ee -> ê`
- `oo -> ô`
- `ow -> ơ`
- `uw -> ư`
- `uo + w -> ươ`
- `ua + w -> ưa`
- `dd -> đ`

Resolver phải xử lý mark trigger ở vị trí được phép trong cùng âm tiết và lưu trigger nào đã consume. Trigger không thể áp dụng hợp lệ phải được render literal hoặc chờ thêm input theo chính sách xác định.

### 4.6 Giải quyết tone

Tone resolver:

1. Thu thập tone trigger cuối cùng có hiệu lực.
2. Loại onset glide của `qu` và `gi` khi có nucleus tiếp theo.
3. Chọn vị trí tone từ nucleus đã được mark hóa.
4. Render tone sau cùng.

Như vậy tone nhập trước mark vẫn tự dịch sang vị trí đúng khi nucleus thay đổi, ví dụ `úo + w` được phân tích lại thành `ượ`.

### 4.7 Validation và English fallback

Không dùng backtrack rộng làm tiêu chí duy nhất.

Sau khi parse:

1. Nếu cấu trúc là âm tiết Việt hợp lệ, dùng output đã compose.
2. Nếu cấu trúc chưa hoàn chỉnh nhưng còn khả năng trở thành hợp lệ, giữ composing state.
3. Nếu cấu trúc rõ ràng không hợp lệ hoặc khớp English dictionary policy hiện có, giữ/raw-restore literal input.
4. Không tự sửa qua phụ âm nằm ngoài nucleus.

Cần tái sử dụng `spelling::is_valid_vietnamese_syllable` và chính sách `should_restore_word`, không tạo validator cạnh tranh.

## 5. Hợp đồng tương thích

### Engine

- `Engine::process_key` vẫn trả `EngineEvent`.
- `buffer()`, replay, macro, auto-restore và undo giữ hành vi công khai hiện tại.
- Replacement phải được tính dựa trên common prefix giữa output cũ và output mới.

### COSMIC, Wayland, X11 và uinput

Các lớp tích hợp chỉ nhận replacement cuối cùng nên không cần biết parser bên trong đã thay đổi.

Phải xác minh:

- số backspace Unicode đúng theo số ký tự hiển thị;
- focus change không mang raw syllable sang ứng dụng khác;
- flush trước clipboard/paste;
- replay tạo output giống xử lý trực tiếp;
- không thay đổi protocol giữa daemon và uinputd.

## 6. Chiến lược triển khai

Không thay engine hiện tại trong một lần. Dùng shadow parser và feature flag nội bộ.

### Phase 1: Đóng băng hành vi hiện tại

- Tạo bảng golden cases từ toàn bộ test Telex hiện có.
- Bổ sung các case tự do, English fallback, backspace, undo, casing và boundary.
- Ghi rõ input nào được hỗ trợ và input nào phải giữ literal.

### Phase 2: Parser thuần

- Tách type `Tone`, `VowelMark`, `RawSyllable`, `ParsedSyllable`.
- Viết parser không I/O và không phụ thuộc desktop.
- Mỗi raw sequence tạo output xác định và lặp lại được.

### Phase 3: Mark và tone resolver

- Cài mark resolver theo nucleus.
- Cài tone resolver sau mark resolver.
- Loại bỏ phụ thuộc vào thứ tự tone/mark.
- Thêm property tests cho hoán vị trigger.

### Phase 4: Shadow comparison

Trong test hoặc debug build:

1. Chạy engine cũ.
2. Chạy parser mới trên cùng raw keys.
3. So sánh output.
4. Cho phép khác biệt chỉ trong danh sách behavior mới đã phê duyệt.

Mục tiêu là phát hiện regression trước khi chuyển luồng production.

### Phase 5: Tích hợp EngineEvent

- Dùng parser mới làm nguồn composing output.
- Tính replacement diff.
- Kết nối backspace, reset, flush, replay và macro.
- Giữ engine cũ sau feature flag trong một giai đoạn chuyển tiếp.

### Phase 6: Runtime validation trên COSMIC

Chạy matrix tối thiểu:

| Hệ thống | Display | Đường inject |
|---|---|---|
| Pop!_OS COSMIC | Wayland | uinput |
| Pop!_OS COSMIC | Wayland | wtype hoặc fallback hiện có |
| Fedora COSMIC | Wayland | uinput |
| Phiên X11 được hỗ trợ | X11 | XTest/X11 path hiện có |

Ứng dụng mục tiêu tối thiểu:

- COSMIC Text Editor;
- Terminal;
- Chromium;
- Firefox;
- ứng dụng GTK;
- ứng dụng Electron.

### Phase 7: Xóa engine legacy

Chỉ xóa các nhánh backtrack cũ khi:

- golden suite pass;
- property tests pass;
- shadow mismatch bằng 0 ngoài allowlist;
- runtime matrix pass;
- không còn caller dùng state legacy.

## 7. Kiểm thử bắt buộc

### Unit tests

- parse onset/nucleus/coda;
- mark resolution;
- tone placement;
- `qu` và `gi`;
- casing;
- invalid syllable fallback;
- boundary và reset.

### Permutation tests

Với mỗi âm tiết mẫu:

1. Xác định letters và trigger hợp lệ.
2. Sinh các hoán vị trigger trong phạm vi cùng âm tiết.
3. Khẳng định tất cả hoán vị được chính sách cho phép hội tụ cùng output.
4. Khẳng định hoán vị không được phép giữ literal, không âm thầm đổi sang từ khác.

Nhóm mẫu tối thiểu:

- mark đơn: `ă`, `â`, `ê`, `ô`, `ơ`, `ư`;
- nucleus đôi: `oa`, `oe`, `uy`, `iê`, `uô`, `ươ`;
- nucleus ba nguyên âm;
- coda: `c`, `ch`, `m`, `n`, `ng`, `nh`, `p`, `t`;
- onset glide: `qu`, `gi`;
- đủ năm tone;
- chữ hoa và mixed case.

### Property tests

Dùng `proptest` nếu chấp nhận thêm dev-dependency, hoặc generator nội bộ nếu không muốn thêm dependency.

Các invariant:

- render không panic với mọi chuỗi ASCII có độ dài giới hạn;
- parse cùng raw input luôn cho cùng output;
- backspace từng phím trả dần về chuỗi rỗng;
- replay và live processing tương đương;
- trigger không vượt boundary;
- output luôn là Unicode hợp lệ;
- transform rồi undo trả lại raw representation theo policy.

### Integration tests

- `EngineEvent::Replace` xóa đúng số ký tự;
- focus change reset composition;
- clipboard/paste không trộn raw keys;
- macro expansion không bị parser tái phân tích;
- auto-restore không lặp hoặc mất phím.

### Runtime tests

- gõ chậm và nhanh;
- key repeat;
- chuyển cửa sổ khi đang composing;
- backspace giữa mark và tone;
- bật/tắt bộ gõ giữa âm tiết;
- restart daemon;
- Chromium/Electron với global Telex;
- terminal và password field phải tuân theo chính sách hiện có.

## 8. Điều kiện chứng minh “toàn cục”

Chỉ được tuyên bố xử lý toàn cục khi đạt toàn bộ điều kiện:

1. Không có code path kiểm tra nguyên chuỗi cụ thể như `dduojwc` hoặc `neues`.
2. Mark và tone được biểu diễn độc lập.
3. Output luôn được dựng lại từ raw keys.
4. Tất cả hoán vị trong policy được kiểm thử tự động.
5. Boundary tests chứng minh không sửa qua âm tiết hoặc từ.
6. Replay, backspace và live input cho kết quả tương đương.
7. Golden suite của hành vi cũ không regression.
8. Runtime matrix trên Pop!_OS COSMIC và Fedora COSMIC pass.

Ngay cả khi đạt các điều kiện này, “toàn cục” vẫn được hiểu trong phạm vi ngữ pháp Telex đã định nghĩa. Không thể đảm bảo mọi chuỗi tùy ý đều cho kết quả người dùng mong muốn nếu không có quy tắc rõ ràng cho chuỗi đó.

## 9. Rủi ro và giảm thiểu

| Rủi ro | Mức | Giảm thiểu |
|---|---|---|
| Auto-restore nhầm từ tiếng Anh | Cao | Golden English corpus, boundary rules, shadow comparison |
| Replacement sai số backspace Unicode | Cao | Diff theo `char`, integration tests với dấu dựng lại |
| Thay đổi undo semantics | Cao | Đóng băng policy hiện tại trước khi refactor |
| Mất state khi đổi focus | Cao | Runtime tests và reset contract |
| Parser chấp nhận quá nhiều hoán vị | Trung bình | Allowlist policy theo loại trigger, không scan vô hạn |
| Hiệu năng sau mỗi phím | Thấp | Chỉ parse âm tiết hiện tại, thường dưới 32 raw keys |
| Hai engine lệch hành vi khi chuyển tiếp | Trung bình | Shadow mismatch report và feature flag |
| File `bamboo.rs` tiếp tục phình to | Trung bình | Tách parser, resolver, renderer và compatibility adapter |

## 10. Definition of Done

- Tất cả unit, property, integration và workspace tests pass.
- `cargo clippy --workspace --all-targets -- -D warnings` pass.
- `cargo test --workspace` pass.
- Không có mismatch ngoài allowlist trong shadow suite.
- Runtime matrix COSMIC pass với evidence.
- Không còn logic backtrack case-by-case trong production path.
- Tài liệu Telex ghi rõ phạm vi free-order và hành vi literal fallback.
- Engine legacy chỉ bị xóa sau một checkpoint riêng.