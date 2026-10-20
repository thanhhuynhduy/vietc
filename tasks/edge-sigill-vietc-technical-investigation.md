# Dieu tra ky thuat: Microsoft Edge crash SIGILL khi go tieng Viet bang VietC

## 1. Muc dich

Tai lieu nay tong hop toan bo bang chung, thi nghiem doi chung, phan tich core dump va pham vi nguyen nhan da duoc khoanh vung cho loi Microsoft Edge bi khung roi vang khi VietC xu ly chuoi tieng Viet dai.

Tai lieu phan biet ro:

- Su kien da duoc xac nhan bang core dump va runtime evidence.
- Gia thuyet co co so nhung chua duoc chung minh.
- Cac nguyen nhan da bi loai tru.
- Cac buoc ky thuat tiep theo de tim root cause va xac minh ban sua.

## 2. Tom tat su co

### Trieu chung

Khi VietC dang o trang thai `enabled=true`, viec go mot cau dai co nhieu tu tieng Viet tren Microsoft Edge co the lam Edge:

1. Phan hoi cham hoac khung.
2. Dong dot ngot.
3. Tao systemd core dump voi signal `SIGILL`.
4. Dung tai lenh may `ud2` ben trong binary `msedge`.

Chuoi ban dau duoc bao cao:

> Da em chao cac anh chi, anh chi cho em hoi trong group minh co ai co nhac cua Lam Hung khong

Trong lan tai hien co giam sat, Edge da crash khi nguoi dung moi go den tu `cac`. Vi vay loi khong phu thuoc rieng vao tu `khong`.

### Anh huong

- Process bi crash la browser process chinh cua Edge, khong phai chi renderer cua mot tab.
- Tat hoan toan VietC thi cung cau tren duoc go binh thuong.
- VietC chay nhung `enabled=false` thi cung cau tren duoc go binh thuong.
- VietC `enabled=true` va AT-SPI da tat thi Edge van crash.
- Loi anh huong truc tiep den kha nang su dung Edge voi chuoi tieng Viet dai.

## 3. Moi truong tai hien

| Thanh phan | Gia tri ghi nhan |
|---|---|
| He dieu hanh | Linux, COSMIC Wayland |
| Trinh duyet | Microsoft Edge 150.0.4078.96 |
| Edge executable | `/opt/microsoft/msedge/msedge` |
| VietC executable | `/home/thanhhd/Git-projects/vietc/target/release/vietc` |
| Input backend | evdev grab va uinput injection |
| Keyboard grab | Thanh cong |
| Active-window X11 fallback | `xdotool` khong kha dung, `xprop` tra ve `0x0` tren Wayland |
| AT-SPI trong phep thu cuoi | `check_atspi2 = false` |
| Core dump location | `/var/lib/systemd/coredump/` |

VietC ghi nhan:

- Ban phim vat ly duoc grab thanh cong.
- Uinput duoc dung de injection.
- Pointer device duoc mo o che do observer.
- Event loop khoi dong binh thuong.
- Khong co dau hieu daemon crash trong cac lan Edge crash.

## 4. Timeline dieu tra

### 4.1 Xac nhan Edge crash bang core dump

Nhieu core dump Edge duoc tim thay trong `/var/lib/systemd/coredump/`.

Cac crash lien quan gan nhat:

| Thoi diem | PID | Signal | Process |
|---|---:|---|---|
| 2026-10-19 09:03 | 38651 | SIGILL | msedge |
| 2026-10-19 09:10 | 105157 | SIGILL | msedge |
| 2026-10-19 10:58 | 148200 | SIGILL | msedge |
| 2026-10-19 16:15 | 5506 | SIGILL | msedge |
| 2026-10-19 16:37 | 53790 | SIGILL | msedge |
| 2026-10-20 14:40 | 141665 | SIGILL | msedge |
| 2026-10-20 15:41 | 424369 | SIGILL | msedge |
| 2026-10-20 15:55 | Tai hien thu cong | SIGILL | msedge |
| 2026-10-20 16:04 | AT-SPI da tat | SIGILL | msedge |
| 2026-10-20 16:18 | Debug injection | SIGILL | msedge |

Core dump va Crashpad metadata xac nhan:

- `Signal: 4 (ILL)`.
- `ProcessType=browser`.
- `SubCode=0x4`.
- Loi nam trong executable `/opt/microsoft/msedge/msedge`.

### 4.2 Xac nhan lenh `ud2`

Ba core dump ban dau va cac core dump tai hien sau do dung tai cung mot vi tri logic:

- Runtime instruction: `ud2`.
- ELF offset on dinh: `0xb913710`.
- Dia chi ao thay doi theo ASLR, nhung offset trong binary Edge khong thay doi.

Vi du:

```text
Program terminated with signal SIGILL, Illegal instruction.
rip = 0x...
=> 0x...: ud2
```

Vung ma ngay truoc `ud2` co logic kiem tra target cua indirect call. Khi validation that bai, luong dieu khien nhay vao trap `ud2`.

Dieu nay phu hop voi mot guard noi bo hoac Control Flow Integrity trap cua Chromium. Binary Edge khong co debug symbols day du, do do chua the anh xa offset nay toi ten ham C++ cu the.

### 4.3 Gia thuyet AT-SPI

Code cu duoc cho la:

- Goi Accessibility D-Bus trong luong kiem tra nen.
- Thuc hien lai sau moi 30 phim.
- Duyet recursive toi 7 tang va khoang 300 node.
- Co the truy van cay accessibility dang thay doi cua Chromium.

Code hien tai da:

- Gioi han DFS toi depth 2.
- Gioi han budget con 40 node.
- Dat timeout D-Bus 100 ms.
- Nhan dien Edge, Chrome, Chromium, Brave, Opera va Vivaldi.
- Khong DFS vao web DOM cua Chromium-family.
- Chi kiem tra application root va top-level window.

De kiem tra AT-SPI co phai tac nhan truc tiep hay khong:

1. Dat `check_atspi2 = false`.
2. Restart VietC.
3. Tai hien cung cau tren Edge.

Ket qua: Edge van crash va tao core dump cung chu ky `SIGILL` va `ud2`.

Ket luan: AT-SPI khong phai nguyen nhan duy nhat va khong can thiet de kich hoat crash. Gia thuyet AT-SPI lam tang tai hoac lam crash de xay ra hon van chua duoc loai bo hoan toan, nhung khong con la root cause chinh phu hop voi evidence hien tai.

### 4.4 Phep thu khong co VietC

VietC duoc dung hoan toan:

```bash
systemctl --user stop vietc
```

Da xac minh:

- User service inactive.
- Khong con process `vietc`.

Nguoi dung go cung cau tren Edge bang ky tu khong dau.

Ket qua:

- Go binh thuong.
- Edge khong crash.

Ket luan: Crash co tuong quan manh voi viec VietC tham gia vao input path.

### 4.5 Phep thu VietC `enabled=false`

VietC duoc khoi dong lai, AT-SPI van tat. Nguoi dung bam `Ctrl+Space`.

Log xac nhan:

```text
[vietc] Mode rotated (Ctrl+Space): enabled=false, method=telex, effective=telex
```

Nguoi dung go lai cung cau tren Edge.

Ket qua:

- Go binh thuong.
- Edge khong crash.

Ket luan:

- Keyboard grab, event polling va forward phym thuan khong du de kich hoat crash.
- Crash nam trong nhanh xu ly khi engine duoc bat.
- Pham vi nghi van thu hep vao transform, replacement transaction hoac Unicode injection.

### 4.6 Thu cac tu don

Da thu:

- `ca`
- `ca` co dau
- `cac`
- `cac` co dau

Ket qua:

- Cac tu don duoc xu ly binh thuong.
- Edge khong crash.

Ket luan:

- Khong co bang chung loi gan voi mot Telex rule don le.
- Khong co bang chung rieng chu `s`, tu `cac` hay tu `khong` la trigger tat dinh.
- Crash can mot chuoi dai hon voi nhieu replacement lien tiep hoac mot trang thai tich luy.

### 4.7 Thu chuoi dai voi debug logging

Debug logging duoc bat va VietC duoc restart. Nguoi dung go cau dai nhieu tu co dau cho den khi Edge crash.

Core dump moi duoc tao:

```text
cores_before=12
cores_after=13
new_cores=1
```

Log ghi nhan nhieu chu ky:

```text
grabbed key
EngineEvent::Replace
delete_before_cursor
insert Unicode text
expected_visible_revision
transform_generation
```

Vi du:

```text
grabbed inject: ch='s' cmds=[
  Replace(
    Replacement {
      version: 1,
      delete_before_cursor: 2,
      insert: "co",
      expected_visible_revision: 9,
      transform_generation: 9,
      reason: ApplyTransform
    }
  )
]
```

Mau hanh vi lap lai:

1. Tu thuan duoc go va dua vao engine buffer.
2. Phim Telex tao transform.
3. VietC xoa cac ky tu ASCII dang hien thi.
4. VietC inject chuoi Unicode thay the.
5. Revision va transform generation tang.
6. Qua trinh lap lai tren nhieu tu.
7. Edge browser process cuoi cung trap tai cung offset `ud2`.

Ket luan: Bang chung hien tai phu hop nhat voi loi do replacement va uinput injection tich luy trong Edge.

## 5. Ma nguon lien quan

### 5.1 Engine va replacement

- `engine/src/engine.rs`
- `engine/src/event.rs`
- `engine/src/telex.rs`
- `engine/tests/auto_restore.rs`
- `engine/tests/state_machine_model.rs`
- `engine/tests/state_machine_properties.rs`

Engine tests hien tai deu pass. Dieu nay cho thay state machine noi bo co tinh deterministic voi input model dang duoc test, nhung khong chung minh browser se chap nhan chuoi low-level delete va insert tu uinput.

### 5.2 Evdev event loop

File:

- `daemon/src/evdev_loop.rs`

Trach nhiem:

- Grab keyboard.
- Doc evdev events.
- Reset buffer khi gap modifier, navigation, mouse click hoac idle gap.
- Goi engine.
- Chuyen `Replace` thanh injection command.
- Forward key events qua injector.

### 5.3 Injection transaction

File:

- `daemon/src/inject.rs`

Pham vi nghi van:

- Thu tu delete va insert.
- Khoang thoi gian giua cac low-level event.
- Cach Unicode duoc phat qua uinput.
- Transaction acknowledgement.
- `visible_revision`.
- Retry hoac failure rollback.
- Kha nang Edge accessibility/input subsystem nhan chuoi event khong hop le khi replacement lap lai nhanh.

### 5.4 AT-SPI va password detection

File:

- `daemon/src/password_detector.rs`
- `daemon/src/app_state.rs`

AT-SPI da duoc loai khoi phep tai hien cuoi, nhung code nay van can duoc giu ngoai duong active typing cua Chromium-family.

### 5.5 Active window va Wayland

File:

- `daemon/src/app_state.rs`

Tren COSMIC Wayland:

- `xdotool` khong kha dung.
- `xprop` co the tra ve `0x0`.
- Native Wayland window change khong duoc phat hien tin cay bang X11 API.

Day la nguyen nhan cua loi buffer cu khi chuyen cua so, nhung chua co bang chung no la nguyen nhan truc tiep cua Edge `SIGILL`.

## 6. Bang thi nghiem doi chung

| Thi nghiem | AT-SPI | VietC process | Engine | Ket qua |
|---|---:|---:|---:|---|
| Chuoi dai, cau hinh ban dau | Bat | Chay | Bat | Edge crash |
| Chuoi `khoong` rieng | Bat | Chay | Bat | Khong crash trong lan thu |
| Chuoi dai, AT-SPI tat | Tat | Chay | Bat | Edge crash |
| Chuoi dai, dung VietC | Khong lien quan | Dung | Khong | Khong crash |
| Chuoi dai, VietC `enabled=false` | Tat | Chay | Tat | Khong crash |
| Tu don co dau | Tat | Chay | Bat | Khong crash |
| Chuoi dai nhieu replacement, debug bat | Tat | Chay | Bat | Edge crash |

## 7. Nhung dieu da duoc xac nhan

1. Edge thuc su crash, khong phai chi tab treo.
2. Signal la `SIGILL`.
3. Lenh gay crash la `ud2`.
4. Nhieu crash cung dung tai mot offset Edge on dinh.
5. Process bi crash la browser process.
6. AT-SPI da tat nhung crash van xay ra.
7. Dung VietC thi khong tai hien crash.
8. VietC chay voi `enabled=false` thi khong tai hien crash.
9. Tu don co dau khong du de tai hien.
10. Chuoi dai voi nhieu replacement tai hien duoc.
11. VietC daemon khong crash trong cac lan Edge crash.
12. Engine deterministic tests pass.

## 8. Nhung dieu chua duoc chung minh

1. Chua co debug symbol de biet ten ham Edge tai offset `0xb913710`.
2. Chua chung minh co memory corruption trong VietC.
3. Chua chung minh AT-SPI lam hong vtable cua Edge.
4. Chua chung minh chinh xac replacement thu bao nhieu gay crash.
5. Chua biet trigger phu thuoc:
   - Tong so replacement.
   - Toc do go.
   - So low-level events.
   - Revision.
   - Khoang cach delete va insert.
   - Mot loai Unicode cu the.
   - Cursor hoac selection state trong Edge.
6. Chua biet crash co tai hien tren Google Chrome cung version voi cung input path hay khong.
7. Chua biet crash co phu thuoc Edge 150.0.4078.96 hay khong.
8. Chua co symbols de khang dinh `ud2` la Chromium CFI trap nao.
9. Chua co ket qua khi injection duoc thay bang clipboard, IBus CommitText hoac mot backend atomically commit text khac.

## 9. Root-cause hypothesis hien tai

### Gia thuyet uu tien cao

Edge nhan mot chuoi dai cac low-level keyboard event do VietC tao ra de thay the text:

1. Physical ASCII keys bi suppress do keyboard grab.
2. VietC engine phat hien Telex transform.
3. VietC inject backspace hoac delete.
4. VietC inject lai ca chuoi Unicode da bien doi.
5. Qua trinh nay lap lai nhieu lan trong mot editing session.
6. Edge cap nhat input, selection, DOM, spellcheck va accessibility state.
7. Mot trang thai noi bo khong hop le tich luy.
8. Edge browser process danh vao indirect-call guard.
9. Guard that bai va Edge chu dong chay `ud2`.

Do tin cay: cao ve pham vi `Replace` va injection, trung binh ve co che noi bo Edge.

### Gia thuyet trung binh

Toc do injection qua nhanh, thieu mot synchronization boundary giua delete va insert, lam Edge xu ly selection hoac accessibility state trong luc chuoi low-level event moi tiep tuc den.

Do tin cay: trung binh.

### Gia thuyet thap hon

- Mot ky tu Unicode cu the lam Edge crash.
- Tu `khong` la trigger duy nhat.
- AT-SPI DFS la root cause duy nhat.
- Viec bat keyboard device hoac forward phim thuan la root cause.
- Engine Telex deterministic logic tu no gay `SIGILL`.

## 10. Ke hoach dieu tra tiep theo

### Buoc 1: Xac dinh replacement cuoi va nguong tai hien

Tao mot runtime trace gom:

- Monotonic timestamp.
- Physical key index.
- Replacement index.
- `delete_before_cursor`.
- Do dai `insert` theo Unicode scalar va UTF-8 byte.
- `expected_visible_revision`.
- `transform_generation`.
- Injection start va end.
- Thoi gian cua tung delete va insert.
- Transaction outcome.

Khong ghi toan bo text nhay cam vao log release. Trong debug test co the ghi input fixture da biet truoc.

### Buoc 2: Binary search chuoi input

Chia cau tai hien thanh prefix:

1. `Da em chao`
2. `Da em chao cac anh chi`
3. `Da em chao cac anh chi, anh chi cho em hoi`
4. Tiep tuc tang cho den khi crash.

Voi prefix dau tien crash, tiep tuc chia theo replacement count de tim nguong toi thieu.

Moi prefix phai duoc thu toi thieu 5 lan vi crash co the phu thuoc timing.

### Buoc 3: Thu toc do injection

Thu cung fixture voi delay co kiem soat:

- 0 ms.
- 1 ms.
- 2 ms.
- 5 ms.
- 10 ms giua low-level events.
- 10 ms, 20 ms va 50 ms giua delete phase va insert phase.

Neu delay lam het crash, timing hoac event batching la tac nhan chinh.

### Buoc 4: Thu transaction shape

So sanh:

- Xoa tung ky tu roi inject tung ky tu.
- Xoa day du, cho mot synchronization boundary, roi insert.
- Commit ca replacement bang backend text protocol neu kha dung.
- Khong replace ca word, chi replace minimal changed suffix.
- Commit preedit mot lan tai word boundary.

Muc tieu la giam so event ma Edge phai xu ly.

### Buoc 5: So sanh browser

Chay cung fixture tren:

- Microsoft Edge 150.0.4078.96.
- Google Chrome.
- Chromium.
- Edge voi profile moi tam.
- Edge tat extensions.
- Edge khong bat accessibility features ngoai he thong.

Moi lan thu phai dung profile rieng va ghi lai browser version, flags va core count.

### Buoc 6: Thu backend khac

Neu moi truong cho phep, so sanh:

- Uinput.
- IBus `CommitText`.
- Wayland input-method protocol.
- Clipboard paste fixture chi dung trong test.
- X11 injection tren X11 runner.

Neu chi uinput gay crash, fix nen nam o injection backend thay vi engine.

### Buoc 7: Phan tich symbols Edge

Can:

- Minidump parser nhu `minidump_stackwalk`.
- Symbol package neu Microsoft cung cap.
- Build ID cua binary Edge.
- Crashpad metadata.
- Mapping chinh xac offset `0xb913710`.

Khong upload core dump hoac user profile ra ben ngoai neu chua duoc phep vi co the chua du lieu nhay cam.

## 11. Yeu cau cho ban sua

Ban sua duoc chap nhan khi dap ung tat ca dieu kien:

1. Cau tai hien duoc go toi thieu 20 lan tren Edge ma khong co core dump moi.
2. Khong co browser process, renderer hoac daemon crash.
3. Text quan sat duoc dung hoan toan.
4. Khong duplicate ky tu.
5. Khong drop ky tu.
6. Khong stuck modifier.
7. Clipboard khong bi thay doi.
8. Cursor va selection khong bi sai.
9. Chromium va Google Chrome runtime tests van pass.
10. GTK va Qt runtime tests van pass.
11. Workspace tests pass.
12. Strict Clippy pass.
13. Format check pass.
14. Runtime artifact ghi browser version, backend, scenario va process health.
15. Core dump count truoc va sau test khong tang.

## 12. Workaround tam thoi

### Khi dung Microsoft Edge

Bam `Ctrl+Space` va xac nhan log:

```text
enabled=false
```

Khi `enabled=false`, VietC chi forward phim va phep thu hien tai khong tai hien crash.

### Tat AT-SPI trong thoi gian dieu tra

File cau hinh:

`~/.config/vietc/config.toml`

Gia tri:

```toml
[password_detection]
check_atspi2 = false
```

AT-SPI khong phai root cause duy nhat, nhung nen duoc giu tat trong khi dieu tra de loai bo bien phu.

### Kiem tra trang thai

```bash
journalctl --user -u vietc --no-pager -n 100 |
  grep -E 'Mode rotated|enabled=' |
  tail -n 1
```

### Dung VietC hoan toan

```bash
systemctl --user stop vietc
```

### Chay lai VietC

```bash
systemctl --user restart vietc
```

## 13. Runtime artifacts da tao

Cac artifact dieu tra nam trong:

`target/crash-analysis/`

Cac file quan trong:

- `msedge-0.txt`
- `msedge-1.txt`
- `msedge-2.txt`
- `msedge-crash-thread-details.txt`
- `edge-vietc-correlation.txt`
- `edge-crashpad-and-accessibility.txt`
- `edge-crashpad-latest.txt`
- `manual-repro-latest-core.txt`
- `manual-khong-repro-state.txt`
- `manual-khong-repro-vietc.log`
- `long-sentence-vietc-debug.log`
- `long-sentence-last-events.txt`
- `long-sentence-state.txt`
- `long-sentence-engine-trace.txt`
- `atspi-disabled-crash-state.txt`

Thu muc `target/` khong nen duoc xem la tai lieu release lau dai neu bi clean boi Cargo. Cac evidence can giu nen duoc sao chep vao noi luu tru release evidence truoc khi chay `cargo clean`.

## 14. Trang thai hien tai

### Da hoan thanh

- Tai hien duoc Edge crash.
- Xac nhan browser process `SIGILL`.
- Xac nhan lenh `ud2`.
- Xac nhan crash signature lap lai.
- Loai tru AT-SPI la nguyen nhan duy nhat.
- Loai tru keyboard forwarding thuan khi engine tat.
- Loai tru mot tu don la trigger tat dinh.
- Khoanh vung vao chuoi replacement va Unicode injection tich luy.
- Luu core dump, Crashpad va VietC debug evidence.

### Chua hoan thanh

- Chua tim duoc replacement index toi thieu.
- Chua tim duoc timing threshold.
- Chua xac dinh ham Edge bang symbols.
- Chua sua code.
- Chua co regression runtime test tu dong cho cau dai.
- Chua the danh dau `Khong browser process crash`.
- Release verdict phai tiep tuc la `FAIL` cho den khi ban sua duoc xac minh tren runtime matrix bat buoc.

## 15. Ket luan

Bang chung hien tai cho thay Microsoft Edge 150.0.4078.96 crash trong browser process tai mot `ud2` guard on dinh khi VietC dang bat engine va thuc hien nhieu replacement qua uinput trong mot chuoi tieng Viet dai.

AT-SPI khong phai dieu kien can thiet cho crash. Engine tat hoac VietC dung hoan toan thi khong tai hien. Cac tu don co dau hoat dong binh thuong. Vi vay huong dieu tra va sua dung la transaction shape, timing va so luong event trong duong `Replace` va uinput injection, khong phai tiep tuc sua quy tac Telex cua mot tu rieng le.

Cho den khi co ban sua va runtime evidence moi, workaround an toan la dat VietC `enabled=false` khi dung Edge hoac dung VietC hoan toan.