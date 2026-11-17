// SPDX-License-Identifier: MIT

/// Largest non-text clipboard payload we are willing to back up and restore.
pub(super) const MAX_BINARY_CLIPBOARD_BYTES: usize = 64 * 1024 * 1024;

/// Clipboard contents that are not plain text and therefore cannot go through
/// the text snapshot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NonTextClipboard {
    Empty,
    Data { mime: String, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ClipboardKind {
    Empty,
    Text,
    Other(String),
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/plain") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

/// Decide how a clipboard offering `types` must be backed up: plain text is
/// preserved as text; anything else (image, file list...) is preserved as raw
/// bytes of its first offered type; nothing offered means an empty clipboard.
pub(super) fn classify_clipboard_types(types: &[String]) -> ClipboardKind {
    if types.is_empty() {
        ClipboardKind::Empty
    } else if types.iter().any(|mime| is_text_mime(mime)) {
        ClipboardKind::Text
    } else {
        ClipboardKind::Other(types[0].clone())
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
    fn any_plain_text_offer_uses_the_text_path() {
        assert_eq!(
            classify_clipboard_types(&types(&["text/html", "text/plain;charset=utf-8"])),
            ClipboardKind::Text
        );
        assert_eq!(
            classify_clipboard_types(&types(&["UTF8_STRING"])),
            ClipboardKind::Text
        );
    }

    #[test]
    fn image_only_clipboard_keeps_its_own_type_instead_of_becoming_text() {
        assert_eq!(
            classify_clipboard_types(&types(&["image/png"])),
            ClipboardKind::Other("image/png".to_string())
        );
        assert_eq!(
            classify_clipboard_types(&types(&["text/uri-list", "x-special/gnome-copied-files"])),
            ClipboardKind::Other("text/uri-list".to_string())
        );
    }
}
