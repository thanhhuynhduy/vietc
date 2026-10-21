use std::io::Result;
use std::thread;
use std::time::Duration;

use evdev::uinput::VirtualDeviceBuilder;
use evdev::{AttributeSet, EventType, InputEvent, Key};

struct VirtualKeyboard {
    device: evdev::uinput::VirtualDevice,
}

impl VirtualKeyboard {
    fn create() -> Result<Self> {
        let mut keys = AttributeSet::new();
        for code in 1..=255u16 {
            keys.insert(Key::new(code));
        }
        let device = VirtualDeviceBuilder::new()?
            .name("vietc-phase6-vk")
            .with_keys(&keys)?
            .build()?;
        Ok(Self { device })
    }

    fn emit(&mut self, keycode: u16, value: i32) -> Result<()> {
        self.device.emit(&[
            InputEvent::new(EventType::KEY, keycode, value),
            InputEvent::new(EventType::SYNCHRONIZATION, 0, 0),
        ])
    }

    fn tap(&mut self, keycode: u16) -> Result<()> {
        self.emit(keycode, 1)?;
        thread::sleep(Duration::from_millis(5));
        self.emit(keycode, 0)
    }

    fn alt_tab(&mut self) -> Result<()> {
        self.emit(56, 1)?;
        self.tap(15)?;
        self.emit(56, 0)
    }

    fn ctrl_combo(&mut self, keycode: u16) -> Result<()> {
        self.emit(29, 1)?;
        self.tap(keycode)?;
        self.emit(29, 0)
    }

    fn select_all_and_copy(&mut self) -> Result<()> {
        self.ctrl_combo(30)?;
        thread::sleep(Duration::from_millis(50));
        self.ctrl_combo(46)
    }

    fn select_all_and_clear(&mut self) -> Result<()> {
        self.ctrl_combo(30)?;
        thread::sleep(Duration::from_millis(50));
        self.tap(14)
    }

    fn type_text(&mut self, text: &str, delay: Duration) -> Result<()> {
        for ch in text.chars() {
            let code = match ch {
                'a' => 30,
                'b' => 48,
                'c' => 46,
                'd' => 32,
                'e' => 18,
                'f' => 33,
                'g' => 34,
                'h' => 35,
                'i' => 23,
                'j' => 36,
                'k' => 37,
                'l' => 38,
                'm' => 50,
                'n' => 49,
                'o' => 24,
                'p' => 25,
                'q' => 16,
                'r' => 19,
                's' => 31,
                't' => 20,
                'u' => 22,
                'v' => 47,
                'w' => 17,
                'x' => 45,
                'y' => 21,
                'z' => 44,
                ' ' => 57,
                '\n' => 28,
                _ => continue,
            };
            self.tap(code)?;
            thread::sleep(delay);
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut keyboard = VirtualKeyboard::create()?;
    let startup_delay = std::env::var("VIETC_VK_STARTUP_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2000);
    thread::sleep(Duration::from_millis(startup_delay));

    match args.first().map(String::as_str) {
        Some("type") => {
            let text = args.get(1).map(String::as_str).unwrap_or_default();
            let delay = args
                .get(2)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(12);
            keyboard.type_text(text, Duration::from_millis(delay))?;
        }
        Some("alt-tab") => keyboard.alt_tab()?,
        Some("copy-all") => keyboard.select_all_and_copy()?,
        Some("scenario") => {
            let switch_delay = args
                .get(1)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(100);
            keyboard.type_text("con hoor", Duration::from_millis(12))?;
            thread::sleep(Duration::from_millis(switch_delay));
            keyboard.alt_tab()?;
            thread::sleep(Duration::from_millis(300));
            keyboard.type_text("xin chaof", Duration::from_millis(12))?;
            thread::sleep(Duration::from_millis(700));
            keyboard.select_all_and_copy()?;
        }
        Some("matrix") => {
            let switch_delay = args
                .get(1)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(100);
            let rounds = args
                .get(2)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(20);

            for round in 1..=rounds {
                keyboard.select_all_and_clear()?;
                keyboard.type_text("con hoor", Duration::from_millis(12))?;
                thread::sleep(Duration::from_millis(switch_delay));
                keyboard.alt_tab()?;
                thread::sleep(Duration::from_millis(750));
                keyboard.select_all_and_clear()?;
                keyboard.type_text("xin chaof", Duration::from_millis(12))?;
                thread::sleep(Duration::from_millis(700));
                keyboard.select_all_and_copy()?;
                thread::sleep(Duration::from_millis(500));

                let observed = std::process::Command::new("wl-paste")
                    .arg("-n")
                    .output()
                    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                    .unwrap_or_default();
                println!("round={round} observed={observed:?}");
                if observed != "xin chao" && observed != "xin chào" {
                    return Err(std::io::Error::other(format!(
                        "round {round}: expected clean xin chao or xin chào, observed {observed:?}"
                    )));
                }

                keyboard.alt_tab()?;
                thread::sleep(Duration::from_millis(750));
            }
        }
        _ => {
            eprintln!(
                "usage: vietc-vk-cli type <text> [delay_ms] | alt-tab | copy-all | scenario [switch_delay_ms] | matrix [switch_delay_ms] [rounds]"
            );
            std::process::exit(2);
        }
    }
    Ok(())
}
