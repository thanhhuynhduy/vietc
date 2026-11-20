// SPDX-License-Identifier: MIT
mod clipboard_command;
#[cfg(feature = "wayland-protocol")]
mod clipboard_transaction;
pub mod inject;
pub mod monitor;
pub mod uinput_monitor;
#[cfg(feature = "wayland-protocol")]
mod wayland_clipboard;
#[cfg(feature = "wayland-protocol")]
mod wayland_vk;
#[cfg(feature = "wayland-protocol")]
mod wayland_vk_keymap;

#[cfg(feature = "x11")]
mod dylib;

#[cfg(feature = "x11")]
pub mod x11_inject;

pub use inject::KeyInjector;
pub use monitor::KeyMonitor;
