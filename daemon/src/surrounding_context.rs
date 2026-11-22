// SPDX-License-Identifier: MIT
//! Context-aware surrounding-text extraction for Gboard-style editing at any
//! caret position (via AT-SPI2 `org.a11y.atspi.Text` on the background thread).
//!
//! Extracts both:
//! - `word_at_caret`: clean word immediately before the caret when caret sits at
//!   word end (`"xin chao| cac ban"` -> `Some("chao")`), and
//! - `prev_word` / `after_sentence_boundary`: preceding word or sentence boundary
//!   when caret sits after a space (`"bánh |ngon"`, `"cargo |run"`, `"Xin chào. |"`),
//!   giving the linguistic engine instant Bigram/Trigram & Auto-Cap context.
//!
//! Safety invariants:
//! - Never queries password fields or non-text widgets.
//! - Never rehydrates when a non-empty text selection is active.
//! - Never rehydrates in the middle of a word (`"vi|et"` -> `None`).
//! - Never seeds on Backspace (`'\x08'`) so deleting a character never strips tones.
//! - Runs exclusively on the background AT-SPI2 thread (zero blocking in `evdev_loop`).

use std::time::{Duration, Instant};

use dbus::arg::{RefArg, Variant};
use dbus::blocking::Connection;

pub const SURROUNDING_CONTEXT_TTL: Duration = Duration::from_secs(10);
const SURROUNDING_QUERY_TIMEOUT: Duration = Duration::from_millis(15);
const LOOKBACK_CHARS: i32 = 48;
const MAX_WORD_CHARS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurroundingContextData {
    pub word_at_caret: Option<String>,
    pub prev_word: Option<String>,
    pub after_sentence_boundary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurroundingContext {
    pub data: SurroundingContextData,
    pub generation: u64,
    pub captured_at: Instant,
}

fn extract_trailing_clean_word(text: &str) -> Option<String> {
    let mut word_chars: Vec<char> = Vec::new();
    for ch in text.chars().rev() {
        if ch.is_alphabetic() {
            word_chars.push(ch);
            if word_chars.len() > MAX_WORD_CHARS {
                return None;
            }
        } else {
            // Reject identifiers glued to digits/underscores (e.g. `var_1chao`).
            if ch.is_ascii_digit() || ch == '_' {
                return None;
            }
            break;
        }
    }
    if word_chars.is_empty() {
        return None;
    }
    word_chars.reverse();
    Some(word_chars.into_iter().collect())
}

fn ends_with_sentence_boundary(before: &str) -> bool {
    if before.ends_with(['\n', '\r']) {
        return true;
    }
    let trimmed = before.trim_end_matches([' ', '\t']);
    trimmed.len() < before.len() && trimmed.ends_with(['.', '!', '?'])
}

/// Extract Gboard-style surrounding context immediately before the caret.
/// Returns `None` when the caret is in the middle of a word (`"vi|et"`).
pub fn extract_surrounding_context(before: &str, after: &str) -> Option<SurroundingContextData> {
    if before.is_empty() {
        return None;
    }

    if before.ends_with([' ', '\t', '\n', '\r']) {
        let after_sentence_boundary = ends_with_sentence_boundary(before);
        // Only seed `prev_word` when separated by a single `' '` so retroactive
        // 2-word deletion (`trailing_delimiters = " "`) never miscounts spaces.
        let prev_word = if after_sentence_boundary {
            None
        } else {
            before
                .strip_suffix(' ')
                .filter(|rest| !rest.ends_with([' ', '\t', '\n', '\r']))
                .and_then(extract_trailing_clean_word)
        };
        if !after_sentence_boundary && prev_word.is_none() {
            return None;
        }
        return Some(SurroundingContextData {
            word_at_caret: None,
            prev_word,
            after_sentence_boundary,
        });
    }

    // Mid-word or identifier caret (`"vi|et"`, `"foo|_bar"`, `"foo|123"`): never rehydrate.
    if after
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }

    let word = extract_trailing_clean_word(before)?;
    let prefix = before.strip_suffix(&word).unwrap_or("");
    let prev_word = if prefix.ends_with([' ', '\t']) {
        extract_trailing_clean_word(prefix.trim_end_matches([' ', '\t']))
    } else {
        None
    };

    Some(SurroundingContextData {
        word_at_caret: Some(word),
        prev_word,
        after_sentence_boundary: false,
    })
}

/// Compatibility helper returning just the word immediately before the caret.
#[cfg(test)]
pub fn extract_word_before_caret(before: &str, after: &str) -> Option<String> {
    extract_surrounding_context(before, after).and_then(|ctx| ctx.word_at_caret)
}

pub fn query_caret_offset(conn: &Connection, sender: &str, path: &str) -> Option<i32> {
    let proxy = conn.with_proxy(sender, path, SURROUNDING_QUERY_TIMEOUT);
    proxy
        .method_call::<(Variant<Box<dyn RefArg>>,), _, _, _>(
            "org.freedesktop.DBus.Properties",
            "Get",
            ("org.a11y.atspi.Text", "CaretOffset"),
        )
        .ok()
        .and_then(|(v,)| v.0.as_i64())
        .and_then(|v| i32::try_from(v).ok())
        .filter(|&v| v >= 0)
}

/// Query AT-SPI2 `org.a11y.atspi.Text` for surrounding context immediately
/// preceding `caret_offset`. Returns `None` if a selection is active, the widget
/// does not expose text, or the caret is inside a word.
pub fn query_surrounding_context(
    conn: &Connection,
    sender: &str,
    path: &str,
    caret_offset: i32,
) -> Option<SurroundingContextData> {
    if caret_offset <= 0 {
        return None;
    }
    let proxy = conn.with_proxy(sender, path, SURROUNDING_QUERY_TIMEOUT);

    // Fail closed if selection state cannot be queried or any non-empty selection
    // is active so we never delete unintended characters before a highlighted range.
    let Ok((n_sel,)) =
        proxy.method_call::<(i32,), _, _, _>("org.a11y.atspi.Text", "GetNSelections", ())
    else {
        return None;
    };
    if n_sel > 0 {
        let Ok((start, end)) = proxy.method_call::<(i32, i32), _, _, _>(
            "org.a11y.atspi.Text",
            "GetSelection",
            (0i32,),
        ) else {
            return None;
        };
        if start != end {
            return None;
        }
    }

    let start = (caret_offset - LOOKBACK_CHARS).max(0);
    let Ok((before,)) = proxy.method_call::<(String,), _, _, _>(
        "org.a11y.atspi.Text",
        "GetText",
        (start, caret_offset),
    ) else {
        return None;
    };

    // Fail closed on timeout/error when probing the char after the caret so we
    // never mistake a mid-word caret (`vi|et`) for word-end. (At EOF, AT-SPI2
    // returns `Ok(("",))` rather than an error.)
    let Ok((after,)) = proxy.method_call::<(String,), _, _, _>(
        "org.a11y.atspi.Text",
        "GetText",
        (caret_offset, caret_offset.saturating_add(1)),
    ) else {
        return None;
    };

    extract_surrounding_context(&before, &after)
}

pub fn maybe_seed_surrounding_word(
    daemon: &mut crate::daemon::Daemon,
    shared_focus: &crate::focus::SharedFocusSnapshot,
    focus_generation: u64,
    ch: char,
) {
    if daemon.engine.is_enabled()
        && daemon.engine.buffer().is_empty()
        && ch != '\x08'
        && !crate::event::is_flush_char(ch)
        && !daemon.app_state.is_terminal_app()
    {
        if let Some(ctx) = crate::focus::take_surrounding_context(shared_focus, focus_generation) {
            daemon.engine.seed_surrounding_context(
                ctx.word_at_caret.as_deref(),
                ctx.prev_word.as_deref(),
                ctx.after_sentence_boundary,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_surrounding_context, extract_word_before_caret};

    #[test]
    fn extracts_word_and_prev_word_at_end_of_word_mid_sentence() {
        let ctx = extract_surrounding_context("xin chao", " cac ban").unwrap();
        assert_eq!(ctx.word_at_caret.as_deref(), Some("chao"));
        assert_eq!(ctx.prev_word.as_deref(), Some("xin"));
        assert!(!ctx.after_sentence_boundary);

        assert_eq!(
            extract_word_before_caret("xin chào", ",").as_deref(),
            Some("chào")
        );
        assert_eq!(extract_word_before_caret("hòa", "").as_deref(), Some("hòa"));
    }

    #[test]
    fn extracts_prev_word_and_sentence_boundary_after_space() {
        let ctx = extract_surrounding_context("bánh ", "ngon").unwrap();
        assert_eq!(ctx.word_at_caret, None);
        assert_eq!(ctx.prev_word.as_deref(), Some("bánh"));
        assert!(!ctx.after_sentence_boundary);

        let sent = extract_surrounding_context("Xin chào. ", "bạn").unwrap();
        assert_eq!(sent.word_at_caret, None);
        assert_eq!(sent.prev_word, None);
        assert!(sent.after_sentence_boundary);

        // Multiple trailing spaces must NOT seed prev_word (prevents retroactive space miscount)
        assert_eq!(extract_surrounding_context("Last  ", "nữa"), None);
    }

    #[test]
    fn rejects_mid_word_caret_and_code_identifiers() {
        // Caret inside `vi|et` must NOT rehydrate `vi`
        assert_eq!(extract_surrounding_context("vi", "et"), None);
        // Glued to underscore or digit -> code identifier, do not mutate
        assert_eq!(extract_word_before_caret("foo_bar", " "), None);
        assert_eq!(extract_word_before_caret("item2chao", " "), None);
        assert_eq!(extract_word_before_caret("foo", "_bar"), None);
        assert_eq!(extract_word_before_caret("foo", "123"), None);
        // Empty input
        assert_eq!(extract_surrounding_context("", ""), None);
    }
}
