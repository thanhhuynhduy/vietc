// SPDX-License-Identifier: MIT

/// Largest total non-text clipboard payload we are willing to back up and restore.
pub(super) const MAX_BINARY_CLIPBOARD_BYTES: usize = 64 * 1024 * 1024;

/// Clipboard contents that are not pure plain text and therefore must preserve
/// rich/multi-MIME formats (e.g., `text/html` + `text/plain;charset=utf-8`,
/// `image/png`, or `text/uri-list` + `text/plain;charset=utf-8`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NonTextClipboard {
    Empty,
    MultiMime(Vec<(String, Vec<u8>)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ClipboardKind {
    Empty,
    Text,
    Rich(Vec<String>),
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/plain") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

fn is_rich_mime(mime: &str) -> bool {
    mime.starts_with("text/html")
        || mime.starts_with("text/uri-list")
        || mime.starts_with("image/")
        || mime.starts_with("application/")
}

fn preferred_text_fallback(types: &[String]) -> Option<String> {
    for candidate in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"] {
        if let Some(found) = types.iter().find(|m| m.as_str() == candidate) {
            return Some(found.clone());
        }
    }
    types.iter().find(|m| is_text_mime(m)).cloned()
}

/// Decide how a clipboard offering `types` must be backed up:
/// - Empty -> `ClipboardKind::Empty`
/// - Rich formats (`text/html`, `text/uri-list`, `image/*`, `application/*`) ->
///   `ClipboardKind::Rich(mimes)` preserving the primary rich MIME **and** any
///   plain-text fallback MIME so pasting into both rich editors and terminals works.
/// - Pure plain text -> `ClipboardKind::Text` (fast text snapshot path).
pub(super) fn classify_clipboard_types(types: &[String]) -> ClipboardKind {
    if types.is_empty() {
        ClipboardKind::Empty
    } else if let Some(rich) = types.iter().find(|mime| is_rich_mime(mime)) {
        let mut selected = vec![rich.clone()];
        if let Some(text_mime) = preferred_text_fallback(types) {
            if !selected.contains(&text_mime) {
                selected.push(text_mime);
            }
        }
        ClipboardKind::Rich(selected)
    } else if types.iter().any(|mime| is_text_mime(mime)) {
        ClipboardKind::Text
    } else {
        ClipboardKind::Rich(vec![types[0].clone()])
    }
}

pub(super) fn restore_multi_mime_native_first(
    entries: &[(String, Vec<u8>)],
    timeout: std::time::Duration,
) -> bool {
    #[cfg(feature = "wayland-protocol")]
    {
        crate::wayland_clipboard_restore::restore_multi_mime_native(entries.to_vec(), timeout)
            .is_ok()
    }
    #[cfg(not(feature = "wayland-protocol"))]
    {
        let _ = (entries, timeout);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn empty_clipboard_is_backed_up_as_empty() {
        assert_eq!(classify_clipboard_types(&[]), ClipboardKind::Empty);
    }

    #[test]
    fn plain_text_only_offers_use_the_text_path() {
        assert_eq!(
            classify_clipboard_types(&types(&["text/plain;charset=utf-8"])),
            ClipboardKind::Text
        );
        assert_eq!(
            classify_clipboard_types(&types(&["UTF8_STRING"])),
            ClipboardKind::Text
        );
    }

    #[test]
    fn rich_and_binary_clipboard_types_preserve_both_rich_and_plain_text_fallback() {
        assert_eq!(
            classify_clipboard_types(&types(&["text/html", "text/plain;charset=utf-8"])),
            ClipboardKind::Rich(types(&["text/html", "text/plain;charset=utf-8"]))
        );
        assert_eq!(
            classify_clipboard_types(&types(&["text/plain;charset=utf-8", "image/png"])),
            ClipboardKind::Rich(types(&["image/png", "text/plain;charset=utf-8"]))
        );
        assert_eq!(
            classify_clipboard_types(&types(&["image/png"])),
            ClipboardKind::Rich(types(&["image/png"]))
        );
        assert_eq!(
            classify_clipboard_types(&types(&[
                "text/uri-list",
                "text/plain",
                "x-special/gnome-copied-files"
            ])),
            ClipboardKind::Rich(types(&["text/uri-list", "text/plain"]))
        );
    }
}
