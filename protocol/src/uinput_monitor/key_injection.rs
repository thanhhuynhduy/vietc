// SPDX-License-Identifier: MIT
use std::os::unix::io::AsRawFd;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use super::UinputInjector;

pub(super) const UINPUT_MAX_NAME_SIZE: usize = 80;
pub(super) const UI_SET_EVBIT: u64 = 0x40045564;
pub(super) const UI_SET_KEYBIT: u64 = 0x40045565;
#[allow(dead_code)]
const UI_SET_ABSBIT: u64 = 0x40045566;
pub(super) const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
pub(super) const UI_DEV_SETUP: u64 = 0x405c5503;
pub(super) const EV_KEY: u16 = 0x01;
#[allow(dead_code)]
const EV_ABS: u16 = 0x03;
/// Maximum keyboard keycode (0-255). Stops before mouse button codes
/// (BTN_MISC = 256, BTN_LEFT = 272) to avoid X11 treating the uinput
/// device as a pointer.
pub(super) const KEYBOARD_KEY_MAX: u16 = 255;

fn write_all_with(
    mut remaining: &[u8],
    mut write: impl FnMut(&[u8]) -> std::io::Result<usize>,
) -> std::io::Result<()> {
    while !remaining.is_empty() {
        match write(remaining) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write uinput event data",
                ));
            }
            Ok(written) if written <= remaining.len() => {
                remaining = &remaining[written..];
            }
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "uinput writer reported more bytes than requested",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

pub(super) fn serialize_transaction<T>(
    transaction_lock: &Mutex<()>,
    operation: impl FnOnce() -> T,
) -> std::io::Result<T> {
    let _transaction = transaction_lock
        .lock()
        .map_err(|_| std::io::Error::other("uinput transaction lock poisoned"))?;
    Ok(operation())
}

impl UinputInjector {
    pub(super) fn send_enter(&self) -> std::io::Result<()> {
        serialize_transaction(&self.transaction_lock, || {
            self.send_uinput_event_unlocked(EV_KEY, 28, 1)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(2));

            self.send_uinput_event_unlocked(EV_KEY, 28, 0)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(2));
            Ok(())
        })?
    }

    fn write_events_unlocked(&self, events: &[input_event]) -> std::io::Result<()> {
        let bytes = unsafe {
            std::slice::from_raw_parts(events.as_ptr() as *const u8, std::mem::size_of_val(events))
        };

        write_all_with(bytes, |remaining| {
            let written = unsafe {
                libc::write(
                    self.file.as_raw_fd(),
                    remaining.as_ptr() as *const libc::c_void,
                    remaining.len(),
                )
            };
            if written < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(written as usize)
            }
        })
    }

    pub(super) fn send_uinput_event_unlocked(
        &self,
        type_: u16,
        code: u16,
        value: i32,
    ) -> std::io::Result<()> {
        let event = input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };

        self.write_events_unlocked(&[event])
    }

    fn write_events(&self, events: &[input_event]) -> std::io::Result<()> {
        serialize_transaction(&self.transaction_lock, || {
            self.write_events_unlocked(events)
        })?
    }

    pub(super) fn send_key_stroke(&self, keycode: u16, shift: bool) -> std::io::Result<()> {
        let mut events = Vec::with_capacity(8);
        if shift {
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: EV_KEY,
                code: 42,
                value: 1,
            });
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: 0,
                code: 0,
                value: 0,
            });
        }

        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: EV_KEY,
            code: keycode,
            value: 1,
        });
        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: 0,
            code: 0,
            value: 0,
        });
        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: EV_KEY,
            code: keycode,
            value: 0,
        });
        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: 0,
            code: 0,
            value: 0,
        });

        if shift {
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: EV_KEY,
                code: 42,
                value: 0,
            });
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: 0,
                code: 0,
                value: 0,
            });
        }

        self.write_events(&events)
    }

    pub(super) fn send_backspaces(&self, count: usize) -> std::io::Result<()> {
        if count == 0 {
            return Ok(());
        }
        let mut remaining = count;
        while remaining > 0 {
            let chunk = remaining.min(4);
            let mut events = Vec::with_capacity(chunk * 4);
            for _ in 0..chunk {
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: EV_KEY,
                    code: 14,
                    value: 1,
                });
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: 0,
                    code: 0,
                    value: 0,
                });
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: EV_KEY,
                    code: 14,
                    value: 0,
                });
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: 0,
                    code: 0,
                    value: 0,
                });
            }
            self.write_events(&events)?;
            remaining -= chunk;
            if remaining > 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
    fn clear_modifiers_unlocked(&self) -> std::io::Result<()> {
        for code in &[
            29, 97, // L/R Ctrl
            42, 54, // L/R Shift
            56, 100, // L/R Alt
            125, 126,
        ]
        // L/R Meta (Win)
        {
            self.send_uinput_event_unlocked(EV_KEY, *code, 0)?;
        }
        self.send_uinput_event_unlocked(0, 0, 0)?; // SYN
        self.uinput_mod_mask.store(0, Ordering::Release);
        self.uinput_shift_mask.store(0, Ordering::Release);
        std::thread::sleep(std::time::Duration::from_millis(5));
        Ok(())
    }

    pub(super) fn send_ctrl_v(&self) -> std::io::Result<()> {
        let is_terminal = self.terminal_app.load(Ordering::Relaxed) || {
            let st = self.clip.inner.lock().unwrap_or_else(|p| p.into_inner());
            let app = st.active_window.to_ascii_lowercase();
            app.contains("terminal")
                || app.contains("cosmic-term")
                || app.contains("cosmicterm")
                || app.contains("ghostty")
                || app.contains("foot")
                || app.contains("wezterm")
                || app.contains("ptyxis")
                || app.contains("konsole")
                || app.contains("kitty")
                || app.contains("alacritty")
                || app.contains("xterm")
        };

        serialize_transaction(&self.transaction_lock, || {
            if !is_terminal {
                self.send_uinput_event_unlocked(EV_KEY, 42, 0)?;
                self.send_uinput_event_unlocked(EV_KEY, 54, 0)?;
                self.send_uinput_event_unlocked(0, 0, 0)?;
            }
            self.send_uinput_event_unlocked(EV_KEY, 29, 1)?;
            if is_terminal {
                self.send_uinput_event_unlocked(EV_KEY, 42, 1)?;
            }
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            self.send_uinput_event_unlocked(EV_KEY, 47, 1)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            self.send_uinput_event_unlocked(EV_KEY, 47, 0)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            if is_terminal {
                self.send_uinput_event_unlocked(EV_KEY, 42, 0)?;
            }
            self.send_uinput_event_unlocked(EV_KEY, 29, 0)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            self.clear_modifiers_unlocked()
        })?
    }
}

pub(super) fn strip_vn_diacritic(ch: char) -> char {
    match ch {
        'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ' | 'ấ' | 'ẩ'
        | 'ẫ' | 'ậ' => 'a',
        'À' | 'Á' | 'Ả' | 'Ã' | 'Ạ' | 'Ă' | 'Ằ' | 'Ắ' | 'Ẳ' | 'Ẵ' | 'Ặ' | 'Â' | 'Ầ' | 'Ấ' | 'Ẩ'
        | 'Ẫ' | 'Ậ' => 'A',
        'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'e',
        'È' | 'É' | 'Ẻ' | 'Ẽ' | 'Ẹ' | 'Ê' | 'Ề' | 'Ế' | 'Ể' | 'Ễ' | 'Ệ' => 'E',
        'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
        'Ì' | 'Í' | 'Ỉ' | 'Ĩ' | 'Ị' => 'I',
        'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ' | 'ớ' | 'ở'
        | 'ỡ' | 'ợ' => 'o',
        'Ò' | 'Ó' | 'Ỏ' | 'Õ' | 'Ọ' | 'Ô' | 'Ồ' | 'Ố' | 'Ổ' | 'Ỗ' | 'Ộ' | 'Ơ' | 'Ờ' | 'Ớ' | 'Ở'
        | 'Ỡ' | 'Ợ' => 'O',
        'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'u',
        'Ù' | 'Ú' | 'Ủ' | 'Ũ' | 'Ụ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ử' | 'Ữ' | 'Ự' => 'U',
        'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
        'Ỳ' | 'Ý' | 'Ỷ' | 'Ỹ' | 'Ỵ' => 'Y',
        'đ' => 'd',
        'Đ' => 'D',
        other => other,
    }
}

// ioctl helper
pub(super) fn ioctl(
    fd: std::os::unix::io::RawFd,
    request: u64,
    arg: u64,
) -> Result<i32, Box<dyn std::error::Error>> {
    unsafe {
        let result = libc::ioctl(fd, request, arg);
        if result < 0 {
            Err(format!("ioctl failed: {}", std::io::Error::last_os_error()).into())
        } else {
            Ok(result)
        }
    }
}

#[repr(C)]
struct input_event {
    time: timeval,
    type_: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct timeval {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
}

#[repr(C)]
pub(super) struct uinput_setup {
    pub(super) id: input_id,
    pub(super) name: [i8; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct input_id {
    pub(super) bustype: u16,
    pub(super) vendor: u16,
    pub(super) product: u16,
    pub(super) version: u16,
}

pub(super) fn destroy_device(
    fd: std::os::unix::io::RawFd,
) -> Result<(), Box<dyn std::error::Error>> {
    ioctl(fd, UI_DEV_DESTROY, 0).map(|_| ())
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::Duration;

    #[test]
    fn short_writes_are_completed() {
        let mut writes = VecDeque::from([Ok(2), Ok(3)]);
        let mut calls = 0;

        let result = write_all_with(&[1, 2, 3, 4, 5], |_| {
            calls += 1;
            writes.pop_front().unwrap()
        });

        assert!(result.is_ok());
        assert_eq!(calls, 2);
    }

    #[test]
    fn interrupted_write_is_retried() {
        let mut writes = VecDeque::from([Err(io::Error::from(io::ErrorKind::Interrupted)), Ok(4)]);

        let result = write_all_with(&[1, 2, 3, 4], |_| writes.pop_front().unwrap());

        assert!(result.is_ok());
    }

    #[test]
    fn zero_byte_write_is_an_error_not_a_loop() {
        let result = write_all_with(&[1], |_| Ok(0));

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WriteZero);
    }

    #[test]
    fn hard_error_propagates() {
        let result = write_all_with(&[1], |_| {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "uinput closed"))
        });

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn concurrent_transactions_do_not_interleave() {
        let transaction_lock = Arc::new(Mutex::new(()));
        let barrier = Arc::new(Barrier::new(3));
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut workers = Vec::new();

        for transaction in [[1, 2, 3], [4, 5, 6]] {
            let transaction_lock = Arc::clone(&transaction_lock);
            let barrier = Arc::clone(&barrier);
            let output = Arc::clone(&output);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                serialize_transaction(&transaction_lock, || {
                    for value in transaction {
                        output.lock().unwrap().push(value);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                })
                .unwrap();
            }));
        }

        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }

        let output = output.lock().unwrap().clone();
        assert!(output == [1, 2, 3, 4, 5, 6] || output == [4, 5, 6, 1, 2, 3]);
    }
}
