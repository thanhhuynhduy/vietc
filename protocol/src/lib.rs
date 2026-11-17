// SPDX-License-Identifier: MIT
#[cfg(feature = "wayland-protocol")]
mod clipboard_command;
#[cfg(feature = "wayland-protocol")]
mod clipboard_transaction;
pub mod inject;
pub mod monitor;
pub mod uinput_client;
pub mod uinput_monitor;
#[cfg(feature = "wayland-protocol")]
mod wayland_clipboard;
pub mod wayland_im;

#[cfg(feature = "x11")]
mod dylib;

#[cfg(feature = "x11")]
pub mod x11_inject;

#[cfg(feature = "x11")]
pub mod x11_capture;

pub use inject::KeyInjector;
pub use monitor::KeyMonitor;
