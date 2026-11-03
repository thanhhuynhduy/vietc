//! Live check: a remote-desktop window (app_id containing "rustdesk") must get
//! raw keys and no clipboard paste, while a normal local window still gets
//! composed Vietnamese. Needs the real vietc.service.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}
#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [libc::c_char; 80],
    ff_effects_max: u32,
}
#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

struct Vk(std::fs::File);
impl Vk {
    fn new() -> Self {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")
            .unwrap();
        let fd = f.as_raw_fd();
        unsafe {
            libc::ioctl(fd, 0x40045564, 1 as libc::c_int);
            for c in 0..=255 {
                libc::ioctl(fd, 0x40045565, c as libc::c_int);
            }
            let mut s: UinputSetup = std::mem::zeroed();
            s.id.bustype = 3;
            s.id.vendor = 0x1d6b;
            s.id.product = 0x0106;
            for (i, b) in b"vietc-live-remote-kbd".iter().enumerate() {
                s.name[i] = *b as libc::c_char;
            }
            assert_eq!(libc::ioctl(fd, 0x405c5503, &s), 0);
            assert_eq!(libc::ioctl(fd, 0x5501), 0);
        }
        Self(f)
    }
    fn emit(&mut self, t: u16, code: u16, v: i32) {
        let ev = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: t,
            code,
            value: v,
        };
        let b = unsafe {
            std::slice::from_raw_parts(&ev as *const _ as *const u8, std::mem::size_of::<InputEvent>())
        };
        self.0.write_all(b).unwrap();
    }
    fn tap(&mut self, code: u16) {
        self.emit(1, code, 1);
        self.emit(0, 0, 0);
        sleep(10);
        self.emit(1, code, 0);
        self.emit(0, 0, 0);
        sleep(60);
    }
    fn typ(&mut self, s: &str) {
        for ch in s.chars() {
            let code = match ch {
                'a' => 30, 'e' => 18, 'g' => 34, 'i' => 23, 'j' => 36, 'n' => 49,
                's' => 31, 't' => 20, 'v' => 47, 'o' => 24, 'm' => 50, ' ' => 57,
                _ => panic!("{ch:?}"),
            };
            self.tap(code);
        }
    }
}
impl Drop for Vk {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.0.as_raw_fd(), 0x5502);
        }
    }
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn wl_copy(t: &str) {
    let mut c = Command::new("wl-copy").stdin(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(t.as_bytes()).unwrap();
    std::thread::spawn(move || {
        let _ = c.wait();
    });
    sleep(300);
}
fn wl_paste() -> Option<String> {
    let o = Command::new("wl-paste").arg("-n").output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

#[test]
fn remote_desktop_window_is_bypassed() {
    let pages: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let p2 = pages.clone();
    let listener = TcpListener::bind("127.0.0.1:18941").unwrap();
    std::thread::spawn(move || {
        for mut st in listener.incoming().flatten() {
            let _ = st.set_read_timeout(Some(Duration::from_millis(400)));
            let mut data = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                match st.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        data.extend_from_slice(&buf[..n]);
                        if let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&data[..p]).to_lowercase();
                            let len = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if data.len() >= p + 4 + len {
                                break;
                            }
                        }
                    }
                }
            }
            let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            let head = String::from_utf8_lossy(&data[..p]).to_string();
            let host = head
                .lines()
                .find_map(|l| l.strip_prefix("Host: ").or_else(|| l.strip_prefix("host: ")))
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .to_string();
            if head.starts_with("POST") {
                p2.lock()
                    .unwrap()
                    .insert(host, String::from_utf8_lossy(&data[p + 4..]).to_string());
                let _ = st.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            } else {
                let html = "<!DOCTYPE html><html><head><meta charset=utf-8><title>remote-sim</title></head><body><textarea id=t autofocus style='width:90%;height:60vh;font-size:20px'></textarea><script>const t=document.getElementById('t');t.focus();setInterval(()=>{fetch('/r',{method:'POST',body:t.value}).catch(()=>{})},100);</script></body></html>";
                let _ = st.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                        html.len()
                    )
                    .as_bytes(),
                );
            }
        }
    });

    let mut vk = Vk::new();
    sleep(1500);
    let mut report = Vec::new();
    for (host, expect_raw) in [("rustdesk.localhost", true), ("127.0.0.1", false)] {
        Command::new("microsoft-edge")
            .args([
                &format!("--user-data-dir=/tmp/vietc-edge-remote-{host}"),
                "--new-window",
                &format!("--app=http://{host}:18941/"),
                "--no-first-run",
                "--no-default-browser-check",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        sleep(5000);
        wl_copy("REMOTE-OLD-CLIPBOARD");
        vk.typ("tieengs vieetj mootj ");
        sleep(1500);
        let text = pages.lock().unwrap().get(host).cloned().unwrap_or_default();
        let clip = wl_paste();
        let expected = if expect_raw { "tieengs vieetj mootj " } else { "tiếng việt một " };
        report.push(format!(
            "[{}] window={host} text={text:?} expected={expected:?} clipboard_after={clip:?}",
            if text == expected && clip.as_deref() == Some("REMOTE-OLD-CLIPBOARD") { "PASS" } else { "FAIL" }
        ));
        let _ = Command::new("pkill").args(["-f", &format!("vietc-edge-remote-{host}")]).status();
        sleep(1500);
    }
    println!("\n======== REMOTE BYPASS LIVE REPORT ========");
    for l in &report {
        println!("{l}");
    }
    println!("===========================================");
    assert!(report.iter().all(|l| l.starts_with("[PASS]")));
}
