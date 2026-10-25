# Kế hoạch sửa lỗi khi mô phỏng người dùng gõ văn bản

## 1. Mục tiêu

Xác định chính xác lỗi thuộc engine Telex, transaction nhập liệu, quản lý focus, clipboard hay runtime harness; sau đó chuyển mọi lỗi tái hiện được thành regression test trước khi sửa.

Điều kiện hoàn thành:

- Không còn từ sai dấu trong 100 lượt chạy với mỗi tốc độ 8, 15, 30 và 60 ms/phím.
- Live input và deterministic replay cho cùng một kết quả.
- Fedora COSMIC và Pop!_OS COSMIC pass application matrix.
- Mỗi failure lưu được raw keys, application, tốc độ, focus history, engine events, replacement events và observed text.
- Chưa xóa legacy rollback path cho đến khi toàn bộ runtime matrix pass.

## 2. Các lỗi đã phát hiện

### Lỗi 1: Firefox báo `TargetCopyFailed`

**Hiện tượng**

Firefox controlled runtime exit 101 với `TargetCopyFailed` trên cả Fedora COSMIC và Pop!_OS COSMIC.

**Điều chưa được chứng minh**

Kết quả hiện tại chưa phân biệt được các trường hợp:

1. Firefox chưa khởi động xong.
2. Harness chọn sai process tree hoặc cửa sổ.
3. Native Wayland toplevel chưa được compositor activate.
4. Textarea chưa nhận keyboard focus.
5. Phím đã được gửi nhưng target không nhận đủ.
6. Văn bản nhập đúng nhưng `Ctrl+A` hoặc `Ctrl+C` thất bại.
7. Clipboard read-back diễn ra trước khi Firefox cập nhật clipboard.
8. Văn bản thực sự bị engine bỏ dấu sai.

Do đó, `TargetCopyFailed` chưa phải bằng chứng engine Telex sai.

**Cách sửa**

Tách Firefox scenario thành các checkpoint độc lập:

1. Xác nhận process Firefox còn sống.
2. Ghi PID, child process tree, title và app-id.
3. Tìm đúng COSMIC Wayland toplevel thuộc process tree.
4. Activate toplevel theo title/app-id.
5. Xác nhận compositor báo cửa sổ đã `activated`.
6. Click hoặc focus textarea.
7. Gửi một focus probe ASCII ngắn như `probe`.
8. Đọc lại probe trước khi chạy corpus.
9. Xóa probe và nhập controlled text.
10. Thử clipboard observation bằng bounded polling.
11. Nếu copy vẫn thất bại, dùng observable channel độc lập thay cho clipboard.
12. Ghi lỗi checkpoint cụ thể, không trả chung `TargetCopyFailed`.

**Regression cần thêm**

- `firefox_process_alive`
- `firefox_toplevel_discovered`
- `firefox_toplevel_activated`
- `firefox_textarea_focus_probe`
- `firefox_controlled_input`
- `firefox_copy_selection`
- `firefox_clipboard_readback`

**Acceptance**

- Target nhận đúng controlled text.
- Observation đọc lại đúng text.
- Toàn bộ process được terminate và reap.
- Result artifact ghi `PASS`, exit 0.

---

### Lỗi 2: Clipboard restore quá sớm làm replacement thất bại

**Hiện tượng**

Fast-input trace từng ghi `Unicode insertion failed: clipboard unavailable`. Sau khi đổi sang bounded polling, transaction đầu tiên được ghi là `applied`, nhưng privileged scenario trên Pop!_OS chưa được chạy lại.

**Nguyên nhân khả dĩ**

Runtime dùng clipboard để chèn Unicode rồi restore clipboard cũ trước khi target application hoàn tất paste hoặc trước khi clipboard owner/read-back ổn định.

**Cách sửa**

- Mỗi replacement có transaction ID tăng đơn điệu.
- Transaction đi qua các trạng thái:
  - `prepared`
  - `clipboard_written`
  - `paste_dispatched`
  - `target_observed`
  - `applied`
  - `clipboard_restored`
  - `failed`
- Chỉ restore clipboard sau khi transaction đạt `applied`.
- Dùng bounded polling thay cho sleep cố định.
- Poll mỗi 5 đến 10 ms, tổng timeout giới hạn 500 đến 1000 ms.
- Callback của transaction cũ không được sửa state của composition mới.
- Khi timeout, log transaction ID, expected replacement, clipboard owner, elapsed time và state cuối.
- Không retry vô hạn và không phát lại phím nếu chưa biết target đã nhận lần đầu hay chưa.

**Regression cần thêm**

- Clipboard cập nhật chậm 20, 50, 100 và 250 ms.
- Clipboard không khả dụng.
- Target paste chậm.
- Focus đổi trong lúc transaction đang chạy.
- Transaction N hoàn thành sau transaction N+1.
- Clipboard cũ được restore đúng một lần.

**Acceptance**

- Không mất dấu ở tốc độ 8 ms/phím.
- Không paste trùng.
- Không restore clipboard trước `applied`.
- Fast-input privileged stress pass liên tiếp 20 lượt.

---

### Lỗi 3: Focus switch có nguy cơ mang composition sang cửa sổ khác

**Hiện tượng**

Deterministic focus-switch contract pass. Fedora privileged focus-switch stress cũng pass, nhưng Pop!_OS privileged run trước đó fail vì harness chưa nhận diện `ydotool`; suite chưa được chạy lại sau khi sửa harness.

**Rủi ro**

Nếu raw buffer của target A được dùng tiếp tại target B, dấu hoặc replacement có thể áp dụng lên sai từ, làm văn bản trông như bị bỏ dấu sai.

**Cách sửa**

- Gắn composition với target identity ổn định gồm seat, focused toplevel/app-id và surface/window identity nếu có.
- Khi target identity đổi:
  - Hủy transaction chưa commit an toàn.
  - Flush hoặc reset raw composition theo contract.
  - Không replay raw keys của target cũ sang target mới.
- Ignore callback đến muộn nếu callback không khớp generation hiện tại.
- Log `focus_generation`, old target, new target và reset reason.

**Regression cần thêm**

- Gõ nửa từ ở A, chuyển B rồi hoàn thành chuỗi.
- Chuyển A → B → A.
- Focus đổi trong lúc clipboard paste đang chờ.
- Cửa sổ bị đóng trong lúc composition active.
- Hai cửa sổ cùng app-id nhưng khác toplevel.
- Focus đổi ngay trước space, punctuation và Enter.

**Acceptance**

- Target A và B giữ văn bản độc lập.
- Không replacement nào áp dụng vào target khác transaction ban đầu.
- Pop!_OS và Fedora focus-switch stress pass 20 lượt liên tiếp.

---

### Lỗi 4: Fast-input scenario timeout

**Hiện tượng**

Pop!_OS fast-input privileged stress exit 101 với `TimedOut`. Fedora cùng scenario đã pass ở key delay 8 ms.

**Nguyên nhân khả dĩ**

- Timeout bao trùm quá nhiều giai đoạn nên không biết bước nào chậm.
- Clipboard polling hoặc target observation chưa hoàn tất.
- Harness chờ exact final text trong khi application vẫn đang xử lý event queue.
- Một replacement thất bại làm expected và observed không bao giờ hội tụ.

**Cách sửa**

- Tách timeout theo phase:
  - startup
  - focus acquisition
  - input dispatch
  - replacement completion
  - final observation
  - teardown
- Ghi first divergent word thay vì chỉ timeout toàn đoạn.
- Sau mỗi boundary, chờ bounded convergence của từ vừa hoàn thành.
- Ghi raw timestamp cho từng key và từng replacement.
- Khi timeout, lưu expected prefix dài nhất khớp với observed text.
- Không tăng timeout trước khi xác định phase bị treo.

**Regression cần thêm**

Chạy corpus ở:

- 8 ms/phím
- 15 ms/phím
- 30 ms/phím
- 60 ms/phím
- burst 5 đến 10 phím không delay, sau đó nghỉ 50 ms

Bao phủ:

- `dduojwc → được`
- `dduocwj → được`
- `tieengs → tiếng`
- `neues` và `neesu` theo policy đã chốt
- punctuation
- newline
- backspace
- macro isolation
- disabled literal mode

**Acceptance**

- 100 lượt mỗi tốc độ không có mismatch.
- Không phase nào timeout.
- Khi fail, artifact chỉ ra từ đầu tiên sai và transaction liên quan.

---

### Lỗi 5: COSMIC Edit daemonize ra ngoài owned process group

**Hiện tượng**

Harness không sở hữu đầy đủ process lifecycle vì COSMIC Edit daemonize ra ngoài process group ban đầu.

**Ảnh hưởng**

Harness có thể:

- Theo dõi sai PID.
- Không xác định đúng cửa sổ.
- Không teardown được process.
- Kết luận timeout hoặc target failure dù ứng dụng vẫn chạy.

Đây là lỗi lifecycle của harness, chưa phải bằng chứng engine bỏ dấu sai.

**Cách sửa**

- Không dùng process group ban đầu làm nguồn chân lý duy nhất.
- Sau launch, resolve application bằng app-id, toplevel identity và process ancestry.
- Chụp baseline process trước test rồi chỉ nhận process/toplevel mới phù hợp.
- Teardown qua PID cụ thể đã chứng minh ownership.
- Không dùng `pkill` hoặc kill theo tên.
- Nếu application tái sử dụng daemon có sẵn, tạo isolated profile/session nếu được hỗ trợ.
- Nếu không chứng minh ownership, đánh dấu `UNAVAILABLE` thay vì `FAIL`.

**Regression cần thêm**

- Launcher fork rồi parent exit.
- Double-fork/daemonize.
- Application dùng process đã chạy sẵn.
- Nhiều cửa sổ cùng app-id.
- Teardown chỉ kill process do fixture tạo.

**Acceptance**

- Harness xác định đúng target window.
- Controlled text được quan sát chính xác.
- Không để lại process do fixture tạo.
- COSMIC Edit scenario exit 0.

---

### Lỗi 6: Qt target thoát trong startup

**Hiện tượng**

Qt controlled runtime exit 101 vì target thoát khi khởi động.

**Ảnh hưởng**

Chưa có input evidence, nên không thể kết luận engine Telex sai.

**Cách sửa**

- Lưu stdout, stderr, exit code và thời gian sống của target.
- Kiểm tra plugin Wayland/Qt, display environment và dependency runtime.
- Thêm startup readiness gate dựa trên toplevel xuất hiện và process còn sống.
- Nếu fixture Qt hiện tại không ổn định, dùng một target Qt tối giản có text widget và observable output.
- Chỉ bắt đầu gửi phím sau readiness gate.

**Acceptance**

- Target sống qua startup timeout.
- Text widget nhận focus probe.
- Corpus được nhập và đọc lại chính xác.
- Process được teardown sạch.

---

### Lỗi 7: Konsole scenario timeout

**Hiện tượng**

Konsole đã được cài nhưng terminal controlled runtime exit 101 với `TimedOut`.

**Nguyên nhân khả dĩ**

- Toplevel chưa activate.
- Shell prompt chưa ready.
- Phím được gửi vào terminal nhưng observation command chưa kết thúc.
- Escape sequence hoặc shell editing làm observed text khác textarea.
- Clipboard read-back chưa ổn định.

**Cách sửa**

- Khởi động shell tối giản, tắt prompt phức tạp và bracketed paste nếu ảnh hưởng test.
- Chờ marker readiness duy nhất trước khi gửi corpus.
- Dùng command đọc stdin rồi ghi kết quả ra file artifact thay vì phụ thuộc hoàn toàn vào clipboard.
- Tách timeout startup, focus, input và observation.
- Escape corpus an toàn để punctuation không bị shell diễn giải.
- Xác minh focus probe ASCII trước Telex corpus.

**Acceptance**

- Marker readiness xuất hiện.
- Controlled text được capture qua observable file/channel.
- Không timeout.
- Konsole scenario exit 0 trên cả hai distro.

---

### Lỗi 8: Electron chưa có executable phù hợp trên Fedora

**Hiện tượng**

Electron được phân loại `UNAVAILABLE` vì harness không tìm thấy executable được hỗ trợ.

**Cách sửa**

- Không coi đây là lỗi behavior.
- Thêm capability discovery cho các executable/package layout thực tế.
- Ưu tiên fixture Electron do repository kiểm soát thay vì phụ thuộc ứng dụng hệ thống.
- Fixture cần có textarea, focus marker và observable output độc lập.
- Nếu dependency không có, scenario phải trả `UNAVAILABLE` với lý do cụ thể.

**Acceptance**

- Capability gate tìm đúng executable hoặc fixture.
- Electron target khởi động, nhận focus và trả observed text.
- Scenario pass trên Fedora và Pop!_OS.

## 3. Corpus mô phỏng văn bản thật

Tạo corpus data-driven gồm:

1. Từ đơn bao phủ toàn bộ mark và tone.
2. Free-order trigger:
   - mark trước tone
   - tone trước mark
   - các permutation được policy cho phép
3. Câu có:
   - viết hoa đầu câu
   - dấu phẩy, chấm, hai chấm, ngoặc và dấu hỏi
   - xuống dòng
   - số
   - URL
   - email
   - từ tiếng Anh
4. Editing:
   - backspace giữa từ
   - xóa hết rồi gõ lại
   - di chuyển focus giữa hai cửa sổ
   - bật/tắt bộ gõ giữa câu
5. Runtime stress:
   - input đều
   - burst input
   - focus change
   - clipboard delay
   - target processing delay

Mỗi case phải lưu:

- Case ID.
- Raw keystrokes.
- Expected Unicode.
- Allowed fallback nếu có.
- Application.
- Key delay.
- Random seed.
- Focus timeline.
- Expected event sequence.

## 4. Cải tiến hệ thống quan sát lỗi

Mỗi failure artifact cần có:

- Distro, desktop, session type và application version.
- Application PID tree, title và app-id.
- Raw key stream kèm monotonic timestamp.
- Engine raw buffer trước và sau mỗi key.
- Parsed onset, nucleus, coda, mark và tone.
- Rendered text.
- Replacement delete count và inserted Unicode.
- Transaction ID và state transitions.
- Focus generation.
- Expected text.
- Observed text.
- First differing Unicode scalar/index.
- First incorrect word.
- Clipboard polling timeline.
- Exit reason và teardown result.

Không dùng một lỗi tổng quát như `TargetCopyFailed` hoặc `TimedOut` khi có thể xác định checkpoint cụ thể.

### Trạng thái triển khai

Đã hoàn thiện transaction clipboard trong `protocol/src/uinput_monitor.rs`:

- State machine bắt buộc đi theo `Prepared -> ClipboardWritten -> PasteDispatched -> TargetObserved -> Applied -> ClipboardRestored`.
- Mọi transition và completion đều được ràng buộc bằng transaction ID; callback hoặc completion cũ không thể sửa transaction mới.
- Clipboard chỉ được restore sau `Applied`.
- Polling dùng chu kỳ 5 ms, timeout tổng 750 ms và cửa sổ quan sát paste 250 ms.
- Không dispatch lại Ctrl+V khi trạng thái nhận paste chưa xác định.
- Regression test bao phủ clipboard delay 20/50/100/250 ms, paste chậm, clipboard unavailable, stale/out-of-order completion và duplicate restore.

## 5. Quy trình sửa theo tầng

### Tầng 1: Engine deterministic

- Chạy golden corpus.
- Chạy permutation suite.
- Chạy replay/backspace/boundary properties.
- Mọi từ sai phải được sửa ở đây trước nếu deterministic replay cũng sai.

### Tầng 2: Replacement transaction

- Test transaction state machine không cần Wayland.
- Mô phỏng clipboard và target delay.
- Chứng minh không restore sớm, không duplicate paste và không callback stale.

### Tầng 3: Direct-uinput

- Xác minh PING, TYPE, PASTE, BACKSPACE và REPLACE.
- So sánh raw dispatch với expected event sequence.

### Tầng 4: Single-application controlled tests

Thứ tự xử lý:

1. Firefox
2. COSMIC Edit
3. Qt
4. Konsole
5. Electron

Chỉ chạy scenario đang sửa, không chạy lại toàn matrix sau mỗi thay đổi.

### Tầng 5: Full paragraph stress

- Chạy 100 lượt mỗi tốc độ.
- Chạy focus-switch interleaving.
- Chạy burst input.
- Lưu seed của mọi failure.

### Tầng 6: Cross-distro matrix

- Fedora COSMIC.
- Pop!_OS COSMIC.
- So sánh theo cùng corpus, tốc độ và checkpoint.

## 6. Quy tắc phân loại failure

### `ENGINE_MISMATCH`

Raw replay không cho expected Unicode.

**Hành động:** sửa parser, resolver, renderer hoặc boundary semantics và thêm golden regression.

### `TRANSACTION_FAILURE`

Engine output đúng nhưng replacement không được target áp dụng đúng.

**Hành động:** sửa clipboard/replacement state machine.

### `FOCUS_FAILURE`

Không chứng minh được target đã activate hoặc focus probe thất bại.

**Hành động:** sửa toplevel activation và focus acquisition.

### `OBSERVATION_FAILURE`

Input có thể đã đúng nhưng clipboard hoặc observable channel không đọc lại được.

**Hành động:** sửa observation, không sửa engine dựa trên failure này.

### `STARTUP_FAILURE`

Target thoát hoặc không đạt readiness gate.

**Hành động:** sửa fixture, dependency hoặc launch configuration.

### `LIFECYCLE_FAILURE`

Không chứng minh được process ownership hoặc teardown.

**Hành động:** sửa process/toplevel tracking.

### `UNAVAILABLE`

Thiếu executable hoặc capability bắt buộc.

**Hành động:** cung cấp fixture/dependency hoặc ghi rõ unsupported environment.

## 7. Thứ tự triển khai

1. Chuẩn hóa failure taxonomy và artifact schema.
2. Thêm word-by-word oracle và first-divergence report.
3. Hoàn thiện replacement transaction state machine. Đã triển khai state machine Unicode với transaction ID tăng đơn điệu, kiểm soát transition, restore clipboard chỉ sau `applied`, stale-transition guard và regression tests.
4. Thêm focus generation và stale callback guard. Đã hoàn thành: target identity gồm seat, app-id, toplevel và surface/window khi có; generation chỉ tăng khi target đổi; composition cũ được reset trước phím kế tiếp; replacement đang chờ bị hủy theo generation; completion cũ phải khớp cả transaction ID và focus generation; log ghi generation, target cũ/mới và reset reason. Regression bao phủ A → B, A → B → A, hai cửa sổ cùng app-id khác toplevel, đóng target khi đang compose, đổi focus trong clipboard transaction và đổi focus ngay trước space/punctuation/Enter.
5. Sửa Firefox checkpoint pipeline.
6. Sửa COSMIC Edit lifecycle tracking.
7. Ổn định Qt fixture.
8. Thêm Konsole observable channel.
9. Cung cấp Electron fixture.
10. Chạy paragraph stress theo bốn tốc độ.
11. Chạy cross-distro matrix.
12. Chỉ khi matrix pass mới bắt đầu xóa legacy rollback path.

## 8. Điều kiện đóng toàn bộ kế hoạch

- Golden, permutation, replay, backspace và boundary suites pass.
- Direct-uinput protocol pass.
- Firefox, COSMIC Edit, Qt, Konsole, Chromium, Chrome, GTK và Electron pass trên Fedora và Pop!_OS.
- Focus-switch và fast-input pass 20 lượt liên tiếp trên mỗi distro.
- Paragraph corpus pass 100 lượt ở từng tốc độ 8, 15, 30 và 60 ms/phím.
- Không còn failure bị gộp chung thành `TargetCopyFailed` hoặc `TimedOut`.
- Mọi failure mới đều có deterministic reproduction hoặc checkpoint runtime xác định.
- Không có process fixture bị rò rỉ.
- Chỉ sau các điều kiện trên mới thực hiện Task 14 để xóa legacy implementation.