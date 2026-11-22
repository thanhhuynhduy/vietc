// SPDX-License-Identifier: MIT
//! Rolling Committed-Word Ring Buffer for Gboard-style seamless re-editing
//! across Space boundaries without preedit underline.

use std::collections::VecDeque;

use crate::english::LanguageContext;
use crate::input_method::InputMethod;

pub(crate) const RING_BUFFER_CAPACITY: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedWordEntry {
    pub raw_keys: String,
    pub visible_word: String,
    pub trailing_delimiters: String,
    pub prior_context: LanguageContext,
    pub committed_context: LanguageContext,
    pub was_auto_restored: bool,
    pub user_override_vn: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CommittedRingBuffer {
    entries: VecDeque<CommittedWordEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BackspaceAcrossBoundary {
    /// No committed history available; pass native Backspace through.
    Empty,
    /// Deleted one trailing delimiter char (`' '`), still has delimiter(s) left before the word.
    ConsumedDelimiterOnly,
    /// Deleted the final `' '` immediately following a committed word, placing the caret
    /// right at the end of `entry.visible_word` so the user can immediately append tones/marks
    /// or continue backspacing character-by-character.
    Rehydrated(CommittedWordEntry),
}

impl CommittedRingBuffer {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(RING_BUFFER_CAPACITY),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn push(&mut self, entry: CommittedWordEntry) {
        if entry.visible_word.is_empty() {
            return;
        }
        if self.entries.len() >= RING_BUFFER_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Append an extra space delimiter to the most recently committed word (e.g. `"xin  "`).
    /// Non-space sentence boundaries clear the ring buffer so Backspace across newlines/periods
    /// never unexpectedly mutates an earlier sentence.
    pub(crate) fn append_delimiter_or_clear(&mut self, ch: char) {
        if ch == ' ' {
            if let Some(last) = self.entries.back_mut() {
                last.trailing_delimiters.push(' ');
            }
        } else {
            self.clear();
        }
    }

    pub(crate) fn last(&self) -> Option<&CommittedWordEntry> {
        self.entries.back()
    }

    pub(crate) fn last_two(&self) -> (Option<&CommittedWordEntry>, Option<&CommittedWordEntry>) {
        let len = self.entries.len();
        match len {
            0 => (None, None),
            1 => (None, self.entries.back()),
            _ => (self.entries.get(len - 2), self.entries.back()),
        }
    }

    pub(crate) fn pop_last(&mut self) -> Option<CommittedWordEntry> {
        self.entries.pop_back()
    }

    pub(crate) fn on_backspace_when_idle(&mut self) -> BackspaceAcrossBoundary {
        let Some(last) = self.entries.back_mut() else {
            return BackspaceAcrossBoundary::Empty;
        };
        if last.trailing_delimiters.pop().is_none() {
            self.entries.pop_back();
            return BackspaceAcrossBoundary::Empty;
        }
        if last.trailing_delimiters.is_empty() {
            let entry = self.entries.pop_back().unwrap();
            BackspaceAcrossBoundary::Rehydrated(entry)
        } else {
            BackspaceAcrossBoundary::ConsumedDelimiterOnly
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VnShape {
    None,
    Breve,      // ă (w / 8)
    Circumflex, // â, ê, ô (aa/ee/oo / 6)
    Horn,       // ơ, ư (w / 7)
    StrokeD,    // đ (dd / 9)
}

fn decompose_vn_char(ch: char) -> (char, VnShape, Option<u8>, bool) {
    let is_upper = ch.is_uppercase();
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    let (base, shape, tone) = match lower {
        'à' => ('a', VnShape::None, Some(2)),
        'á' => ('a', VnShape::None, Some(1)),
        'ả' => ('a', VnShape::None, Some(3)),
        'ã' => ('a', VnShape::None, Some(4)),
        'ạ' => ('a', VnShape::None, Some(5)),
        'ă' => ('a', VnShape::Breve, None),
        'ằ' => ('a', VnShape::Breve, Some(2)),
        'ắ' => ('a', VnShape::Breve, Some(1)),
        'ẳ' => ('a', VnShape::Breve, Some(3)),
        'ẵ' => ('a', VnShape::Breve, Some(4)),
        'ặ' => ('a', VnShape::Breve, Some(5)),
        'â' => ('a', VnShape::Circumflex, None),
        'ầ' => ('a', VnShape::Circumflex, Some(2)),
        'ấ' => ('a', VnShape::Circumflex, Some(1)),
        'ẩ' => ('a', VnShape::Circumflex, Some(3)),
        'ẫ' => ('a', VnShape::Circumflex, Some(4)),
        'ậ' => ('a', VnShape::Circumflex, Some(5)),
        'è' => ('e', VnShape::None, Some(2)),
        'é' => ('e', VnShape::None, Some(1)),
        'ẻ' => ('e', VnShape::None, Some(3)),
        'ẽ' => ('e', VnShape::None, Some(4)),
        'ẹ' => ('e', VnShape::None, Some(5)),
        'ê' => ('e', VnShape::Circumflex, None),
        'ề' => ('e', VnShape::Circumflex, Some(2)),
        'ế' => ('e', VnShape::Circumflex, Some(1)),
        'ể' => ('e', VnShape::Circumflex, Some(3)),
        'ễ' => ('e', VnShape::Circumflex, Some(4)),
        'ệ' => ('e', VnShape::Circumflex, Some(5)),
        'ì' => ('i', VnShape::None, Some(2)),
        'í' => ('i', VnShape::None, Some(1)),
        'ỉ' => ('i', VnShape::None, Some(3)),
        'ĩ' => ('i', VnShape::None, Some(4)),
        'ị' => ('i', VnShape::None, Some(5)),
        'ò' => ('o', VnShape::None, Some(2)),
        'ó' => ('o', VnShape::None, Some(1)),
        'ỏ' => ('o', VnShape::None, Some(3)),
        'õ' => ('o', VnShape::None, Some(4)),
        'ọ' => ('o', VnShape::None, Some(5)),
        'ô' => ('o', VnShape::Circumflex, None),
        'ồ' => ('o', VnShape::Circumflex, Some(2)),
        'ố' => ('o', VnShape::Circumflex, Some(1)),
        'ổ' => ('o', VnShape::Circumflex, Some(3)),
        'ỗ' => ('o', VnShape::Circumflex, Some(4)),
        'ộ' => ('o', VnShape::Circumflex, Some(5)),
        'ơ' => ('o', VnShape::Horn, None),
        'ờ' => ('o', VnShape::Horn, Some(2)),
        'ớ' => ('o', VnShape::Horn, Some(1)),
        'ở' => ('o', VnShape::Horn, Some(3)),
        'ỡ' => ('o', VnShape::Horn, Some(4)),
        'ợ' => ('o', VnShape::Horn, Some(5)),
        'ù' => ('u', VnShape::None, Some(2)),
        'ú' => ('u', VnShape::None, Some(1)),
        'ủ' => ('u', VnShape::None, Some(3)),
        'ũ' => ('u', VnShape::None, Some(4)),
        'ụ' => ('u', VnShape::None, Some(5)),
        'ư' => ('u', VnShape::Horn, None),
        'ừ' => ('u', VnShape::Horn, Some(2)),
        'ứ' => ('u', VnShape::Horn, Some(1)),
        'ử' => ('u', VnShape::Horn, Some(3)),
        'ữ' => ('u', VnShape::Horn, Some(4)),
        'ự' => ('u', VnShape::Horn, Some(5)),
        'ỳ' => ('y', VnShape::None, Some(2)),
        'ý' => ('y', VnShape::None, Some(1)),
        'ỷ' => ('y', VnShape::None, Some(3)),
        'ỹ' => ('y', VnShape::None, Some(4)),
        'ỵ' => ('y', VnShape::None, Some(5)),
        'đ' => ('d', VnShape::StrokeD, None),
        other => (other, VnShape::None, None),
    };
    (base, shape, tone, is_upper)
}

/// Converts a visible Vietnamese word (e.g. `"chào"`, `"chà"`, `"Đường"`, `"hò"`)
/// into a canonical Telex or VNI raw key sequence whose composition equals `visible`.
/// Returns `None` if `visible` cannot be losslessly represented as an active Vietnamese
/// composition (for example, an English word containing literal `'w'` in Telex).
pub(crate) fn visible_to_canonical_raw(visible: &str, method: InputMethod) -> Option<String> {
    if visible.is_empty() {
        return Some(String::new());
    }
    let chars: Vec<char> = visible.chars().collect();
    let all_upper = chars.iter().any(|c| c.is_alphabetic())
        && chars
            .iter()
            .filter(|c| c.is_alphabetic())
            .all(|c| c.is_uppercase());

    let mut raw = String::with_capacity(visible.len() + 4);
    let mut detected_tone: Option<u8> = None;

    for i in 0..chars.len() {
        let (base, shape, tone, is_upper) = decompose_vn_char(chars[i]);
        if !base.is_ascii_alphabetic() {
            return None;
        }
        if let Some(t) = tone {
            if detected_tone.is_some() {
                return None;
            }
            detected_tone = Some(t);
        }

        let base_cased = if is_upper {
            base.to_ascii_uppercase()
        } else {
            base
        };
        raw.push(base_cased);

        let mod_upper = all_upper;
        match method {
            InputMethod::Telex => match shape {
                VnShape::None => {
                    // Vietnamese double-`oo` words (`xoong`, `boong`, `moóc`) need `ooo` in Telex
                    // so the pair does not collapse into `ô`.
                    if base == 'o' && i > 0 {
                        let (pb, ps, _, _) = decompose_vn_char(chars[i - 1]);
                        if pb == 'o' && ps == VnShape::None {
                            raw.push(if mod_upper { 'O' } else { 'o' });
                        }
                    }
                }
                VnShape::StrokeD => {
                    raw.push(if mod_upper { 'D' } else { 'd' });
                }
                VnShape::Circumflex => {
                    raw.push(if mod_upper {
                        base.to_ascii_uppercase()
                    } else {
                        base
                    });
                }
                VnShape::Breve => {
                    raw.push(if mod_upper { 'W' } else { 'w' });
                }
                VnShape::Horn => {
                    // In "ươ" compound nucleus, emit a single 'w' after 'o' ("uow")
                    // so the second 'w' does not undo the horn transform.
                    // However, if there is no trailing consonant after "ươ" (an open prefix
                    // like "đườ" or "thườ"), "uow" would compose as "uơ" for h/kh/th or wait,
                    // wait: for "thườ", "thuowf" composes as "thuờ" (plain 'u') because open "-uơ"
                    // rule applies to h/kh/th! Wait: for "thườ", if we emit "thuwowf"? No, two 'w's in
                    // "thuwowf" undo in mark.rs only if w_positions[0] + 1 == w_positions[1]!
                    // In "thuwowf", w_positions are at index 3 and index 5 (not adjacent!), so both 'u' and 'o' get Horn!
                    let has_tail_after_next = i + 2 < chars.len();
                    let next_is_o_horn = i + 1 < chars.len() && {
                        let (nb, ns, _, _) = decompose_vn_char(chars[i + 1]);
                        base == 'u' && nb == 'o' && ns == VnShape::Horn
                    };
                    if !next_is_o_horn || !has_tail_after_next {
                        raw.push(if mod_upper { 'W' } else { 'w' });
                    }
                }
            },
            InputMethod::Vni => match shape {
                VnShape::None => {}
                VnShape::StrokeD => raw.push('9'),
                VnShape::Circumflex => raw.push('6'),
                VnShape::Breve => raw.push('8'),
                VnShape::Horn => {
                    let has_tail_after_next = i + 2 < chars.len();
                    let next_is_o_horn = i + 1 < chars.len() && {
                        let (nb, ns, _, _) = decompose_vn_char(chars[i + 1]);
                        base == 'u' && nb == 'o' && ns == VnShape::Horn
                    };
                    if !next_is_o_horn || !has_tail_after_next {
                        raw.push('7');
                    }
                }
            },
        }
    }

    if let Some(t) = detected_tone {
        let tone_char = match method {
            InputMethod::Telex => {
                let c = match t {
                    2 => 'f',
                    1 => 's',
                    3 => 'r',
                    4 => 'x',
                    5 => 'j',
                    _ => unreachable!(),
                };
                if all_upper {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            }
            InputMethod::Vni => match t {
                1 => '1',
                2 => '2',
                3 => '3',
                4 => '4',
                5 => '5',
                _ => unreachable!(),
            },
        };
        raw.push(tone_char);
    }

    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    fn eval_raw(method: InputMethod, raw: &str) -> String {
        let mut engine = Engine::new(method);
        let mut screen = String::new();
        for ch in raw.chars() {
            match engine.process_key(ch) {
                Some(crate::EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                }
                None => screen.push(ch),
                _ => {}
            }
        }
        screen
    }

    #[test]
    fn canonical_raw_roundtrips_telex_and_vni_words_and_prefixes() {
        let samples = [
            "chào",
            "chà",
            "hòa",
            "hò",
            "hoàn",
            "khỏe",
            "thủy",
            "đường",
            "đườn",
            "đườ",
            "đư",
            "đ",
            "hươu",
            "thương",
            "Việt",
            "VIỆT",
            "Đường",
            "ĐƯỜNG",
            "Nguyễn",
            "già",
            "quá",
            "quyền",
        ];
        for method in [InputMethod::Telex, InputMethod::Vni] {
            for word in samples {
                let raw = visible_to_canonical_raw(word, method)
                    .unwrap_or_else(|| panic!("expected canonical raw for {word}"));
                let rendered = eval_raw(method, &raw);
                assert_eq!(rendered, word, "method={method:?} word={word} raw={raw}");
            }
        }
    }
}
