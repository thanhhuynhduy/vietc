// SPDX-License-Identifier: MIT
use super::raw::VowelMark;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkResolution {
    pub letters: Vec<char>,
    pub marks: Vec<VowelMark>,
    pub consumed: Vec<bool>,
    pub mark_undone: bool,
}

fn lower(ch: char) -> char {
    ch.to_ascii_lowercase()
}

fn is_base_vowel(ch: char) -> bool {
    matches!(lower(ch), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

fn is_tone_key(ch: char) -> bool {
    matches!(lower(ch), 's' | 'f' | 'r' | 'x' | 'j')
}

/// Only allow Telex `dd` -> `đ` (and `ddd` -> `dd` undo) before any vowel appears
/// in the syllable: covers onset `ddi`/`ddos`/`DDANG` and consonant-only Vietnamese
/// abbreviations (`QDD-TTg` -> `QĐ-TTg`, `VNDD`) while preventing mid-word `đ`
/// flashes in English words (`add`, `odd`, `reddit`, `hidden`, `middle`, `wedding`).
fn can_apply_dd_at(raw: &[char], d_first_idx: usize) -> bool {
    !raw[..d_first_idx].iter().any(|&ch| is_base_vowel(ch))
}

/// Allow free-order Telex circumflex across a valid Vietnamese coda (`khongo` -> `không`,
/// `khongso` -> `khống`, `vietej` -> `việt`, `vietje` -> `việt`, `tienges` -> `tiếng`,
/// `tiengse` -> `tiếng`, `tana` -> `tân`) only when:
/// - characters strictly between `j` and `i` form a valid Vietnamese coda (`c,ch,m,n,ng,nh,p,t`),
///   optionally followed by a single trailing tone key right before `i`
/// - nothing after `i` is a non-tone letter (blocking `banana`, `canada`, `benefit`, `manage`, `tenet`)
/// - the raw word is not a known English word (blocking `mono`, `mama`, `meme`, `papa`, `toto`, etc.)
fn can_circumflex_across_coda(raw: &[char], consumed: &[bool], j: usize, i: usize) -> bool {
    let trigger = lower(raw[i]);
    // Allow optional repeated trigger (for double-key undo: `khongoo` -> `khongo`)
    // and optional Telex tone/strip keys after the circumflex trigger.
    if !raw[i + 1..]
        .iter()
        .all(|&ch| lower(ch) == trigger || is_tone_key(ch) || lower(ch) == 'z')
    {
        return false;
    }
    // Phonotactic Single-Syllable Prefix Guard: a Vietnamese syllable never has
    // a consonant separating an earlier vowel from the nucleus vowel `j`
    // (blocks multi-syllable words like `solana`, `katana`, `banana`, `canada`
    // while keeping `khongo`, `vietej`, `tienges`, `chuyene`, `hoana`, `tana`).
    let mut seen_prefix_vowel = false;
    for k in 0..j {
        if consumed[k] {
            continue;
        }
        if is_base_vowel(raw[k]) {
            seen_prefix_vowel = true;
        } else if seen_prefix_vowel {
            return false;
        }
    }
    // Phonotactic Reduplication Guard: block 4-letter untoned `C1 V C1 V`
    // reduplicated tokens where onset == coda (e.g. `momo`, `nono`, `caca`,
    // `tete`, `nana`, `papa`). Toned forms (`momof` -> `mồm`, `tetes` -> `tết`)
    // have `raw.len() > 4` and non-reduplicated stems (`tana` -> `tân`) have
    // `raw[0] != raw[2]`, so both remain intact.
    if j == 1
        && i == 3
        && raw.len() == 4
        && !consumed[0]
        && !consumed[2]
        && lower(raw[0]) == lower(raw[2])
    {
        return false;
    }
    let mut coda_buf = [0u8; 3];
    let mut coda_len = 0usize;
    for k in (j + 1)..i {
        if consumed[k] {
            continue;
        }
        let ch = lower(raw[k]);
        if !ch.is_ascii() || coda_len >= coda_buf.len() {
            return false;
        }
        coda_buf[coda_len] = ch as u8;
        coda_len += 1;
    }
    // Allow a single trailing tone key immediately before the circumflex trigger
    // (e.g. `khongso` -> `khống`, `tiengse` -> `tiếng`, `vietje` -> `việt`),
    // while never stripping an internal `s` like `pasta`.
    if coda_len >= 2 && is_tone_key(coda_buf[coda_len - 1] as char) {
        coda_len -= 1;
    }
    let coda = &coda_buf[..coda_len];
    if !matches!(
        coda,
        b"c" | b"ch" | b"m" | b"n" | b"ng" | b"nh" | b"p" | b"t"
    ) {
        return false;
    }
    // Check that the raw ASCII word is not in the English dictionary (prevents
    // transforming English words like `mono`, `mama`, `meme`, `papa` while
    // keeping `khongo`, `viete`, `vietej`, `tienges`, `tana` free-order).
    if raw.len() <= 32 && raw.iter().all(|c| c.is_ascii()) {
        let mut raw_buf = [0u8; 32];
        for (idx, &ch) in raw.iter().enumerate() {
            raw_buf[idx] = lower(ch) as u8;
        }
        if let Ok(raw_lower) = std::str::from_utf8(&raw_buf[..raw.len()]) {
            return !crate::engine::english_dict().is_english_word(raw_lower);
        }
    }
    let raw_lower: String = raw.iter().map(|&ch| lower(ch)).collect();
    !crate::engine::english_dict().is_english_word(&raw_lower)
}

fn first_tone_undo_index(raw: &[char]) -> Option<usize> {
    let mut seen_vowel = false;
    let mut active_tone: Option<(usize, char)> = None;
    for (i, &ch) in raw.iter().enumerate() {
        let c = lower(ch);
        if is_base_vowel(c) {
            seen_vowel = true;
            active_tone = None;
        } else if seen_vowel && is_tone_key(c) {
            if let Some((prev_idx, prev_tone)) = active_tone {
                if prev_tone == c && prev_idx + 1 == i {
                    return Some(i);
                }
            }
            active_tone = Some((i, c));
        } else {
            active_tone = None;
        }
    }
    None
}

fn has_valid_vietnamese_onset(raw: &[char], consumed: &[bool]) -> bool {
    let first_vowel = raw
        .iter()
        .enumerate()
        .find_map(|(i, &ch)| (!consumed[i] && is_base_vowel(ch)).then_some(i));
    let Some(first_vowel) = first_vowel else {
        return true;
    };
    if first_vowel == 0 {
        return true;
    }
    let onset: String = raw[..first_vowel]
        .iter()
        .enumerate()
        .filter_map(|(i, &ch)| (!consumed[i]).then_some(lower(ch)))
        .collect();
    matches!(
        onset.as_str(),
        "" | "b"
            | "c"
            | "ch"
            | "d"
            | "đ"
            | "g"
            | "gh"
            | "gi"
            | "h"
            | "k"
            | "kh"
            | "l"
            | "m"
            | "n"
            | "ng"
            | "ngh"
            | "nh"
            | "p"
            | "ph"
            | "q"
            | "qu"
            | "r"
            | "s"
            | "t"
            | "th"
            | "tr"
            | "v"
            | "x"
    )
}

pub fn resolve(raw: &[char]) -> MarkResolution {
    let mut letters = raw.to_vec();
    let mut marks = vec![VowelMark::None; raw.len()];
    let mut consumed = vec![false; raw.len()];
    let mut mark_undone = false;
    let tone_undo_at = first_tone_undo_index(raw);

    for i in 1..raw.len() {
        if tone_undo_at.is_some_and(|undo_idx| i > undo_idx) {
            break;
        }
        let current = lower(raw[i]);
        let previous = lower(raw[i - 1]);

        if previous == 'd' && current == 'd' && can_apply_dd_at(raw, i - 1) {
            letters[i - 1] = if raw[i - 1].is_uppercase() {
                'Đ'
            } else {
                'đ'
            };
            consumed[i] = true;
            continue;
        }

        if !has_valid_vietnamese_onset(raw, &consumed) {
            continue;
        }

        if matches!(current, 'a' | 'e' | 'o') {
            let matching = (0..i).rev().find(|&j| {
                if consumed[j] || lower(raw[j]) != current || marks[j] != VowelMark::None {
                    return false;
                }
                let between_all_vowels = raw[j + 1..i]
                    .iter()
                    .all(|&ch| matches!(lower(ch), 'a' | 'e' | 'i' | 'o' | 'u' | 'y'));
                if between_all_vowels {
                    return !(current == 'o'
                        && raw[j + 1..i]
                            .iter()
                            .any(|&ch| matches!(lower(ch), 'a' | 'e')));
                }
                can_circumflex_across_coda(raw, &consumed, j, i)
            });
            if let Some(j) = matching {
                marks[j] = VowelMark::Circumflex;
                consumed[i] = true;
            }
        }
    }

    for i in 0..raw.len() {
        if tone_undo_at.is_some_and(|undo_idx| i > undo_idx) {
            break;
        }
        if lower(raw[i]) != 'w' {
            continue;
        }
        if !has_valid_vietnamese_onset(raw, &consumed) {
            continue;
        }

        let candidates: Vec<usize> = (0..i)
            .rev()
            .filter(|&j| !consumed[j] && matches!(lower(raw[j]), 'a' | 'o' | 'u'))
            .collect();

        if let Some(&last) = candidates.first() {
            // Handle `-ươu` triphthong (`huouw` -> `hươu`, `ruouwj` -> `rượu`, `buouws` -> `bướu`)
            if lower(raw[last]) == 'u' && candidates.len() >= 3 {
                let mid = candidates[1];
                let first = candidates[2];
                let is_qu_glide =
                    first > 0 && lower(raw[first]) == 'u' && lower(raw[first - 1]) == 'q';
                if lower(raw[mid]) == 'o' && lower(raw[first]) == 'u' && !is_qu_glide {
                    marks[first] = VowelMark::Horn;
                    marks[mid] = VowelMark::Horn;
                    marks[last] = VowelMark::None;
                    consumed[i] = true;
                    continue;
                }
            }

            // Allow Gboard-style shape override (`aaw` -> `ă`, `oow` -> `ơ`) when adjacent,
            // while avoiding overriding already-marked vowels across consonants.
            if marks[last] != VowelMark::None {
                let adjacent_to_mark = last + 1 == i || (last + 2 == i && consumed[last + 1]);
                if !adjacent_to_mark {
                    continue;
                }
            }

            let base = lower(raw[last]);
            if matches!(base, 'o' | 'a') {
                if let Some(&before) = candidates.get(1) {
                    let is_qu_glide =
                        before > 0 && lower(raw[before]) == 'u' && lower(raw[before - 1]) == 'q';
                    if lower(raw[before]) == 'u' && !is_qu_glide {
                        if base == 'o' {
                            let has_tail = (last + 1..raw.len()).any(|k| {
                                !consumed[k]
                                    && k != i
                                    && !matches!(
                                        lower(raw[k]),
                                        's' | 'f' | 'r' | 'x' | 'j' | 'z' | 'w'
                                    )
                            });
                            let onset: String = raw[..before]
                                .iter()
                                .enumerate()
                                .filter_map(|(k, &c)| (!consumed[k]).then_some(lower(c)))
                                .collect();
                            // Open `-uơ` words (`huơ`, `khuơ`, `thuở`) keep plain `u` + horned `ơ`;
                            // closed syllables (`thương`, `đường`, `tưới`) mark both `ư` and `ơ`.
                            if !has_tail && matches!(onset.as_str(), "h" | "kh" | "th") {
                                marks[last] = VowelMark::Horn;
                            } else {
                                marks[before] = VowelMark::Horn;
                                marks[last] = VowelMark::Horn;
                            }
                        } else {
                            marks[before] = VowelMark::Horn;
                        }
                        consumed[i] = true;
                        continue;
                    }
                }
            }

            marks[last] = match base {
                'a' => VowelMark::Breve,
                'o' | 'u' => VowelMark::Horn,
                _ => VowelMark::None,
            };
            // Compound `-ươ-` when the user types `uw` before `o` (`tuwoi -> tươi`,
            // `dduwocj -> được`, `truwongf -> trường`): `ưo` is never valid Vietnamese.
            if base == 'u'
                && last + 1 == i
                && i + 1 < raw.len()
                && !consumed[i + 1]
                && lower(raw[i + 1]) == 'o'
                && marks[i + 1] == VowelMark::None
            {
                let is_qu_glide = last > 0 && lower(raw[last - 1]) == 'q';
                if !is_qu_glide {
                    marks[i + 1] = VowelMark::Horn;
                }
            }
            consumed[i] = marks[last] != VowelMark::None;
        }
    }

    // Also allow `awa` -> `â` and `owo` -> `ô` shape override when adjacent.
    for i in 2..raw.len() {
        if tone_undo_at.is_some_and(|undo_idx| i > undo_idx) {
            break;
        }
        let cur = lower(raw[i]);
        if matches!(cur, 'a' | 'o')
            && lower(raw[i - 1]) == 'w'
            && consumed[i - 1]
            && lower(raw[i - 2]) == cur
            && matches!(marks[i - 2], VowelMark::Breve | VowelMark::Horn)
        {
            marks[i - 2] = VowelMark::Circumflex;
            consumed[i] = true;
        }
    }

    // Repeating an active Telex mark trigger cancels the transform. Preserve
    // one literal trigger while consuming the key that originally applied it.
    for trigger in ['a', 'e', 'o'] {
        let positions = raw
            .iter()
            .enumerate()
            .filter_map(|(i, &ch)| (lower(ch) == trigger).then_some(i))
            .collect::<Vec<_>>();
        if positions.len() >= 3
            && marks[positions[0]] == VowelMark::Circumflex
            && consumed[positions[1]]
        {
            // If a shape-override `'w'` between `positions[0]` and `positions[1]`
            // (e.g. `awaa` -> `awa`, `owoo` -> `owo`) was consumed, restore it
            // when undoing the circumflex so no literal character is dropped.
            for w_idx in (positions[0] + 1)..positions[1] {
                if lower(raw[w_idx]) == 'w' && consumed[w_idx] {
                    consumed[w_idx] = false;
                }
            }
            for &pos in &positions {
                letters[pos] = raw[pos];
                marks[pos] = VowelMark::None;
            }
            consumed[positions[0]] = false;
            consumed[positions[1]] = true;
            for &pos in &positions[2..] {
                consumed[pos] = false;
            }
            mark_undone = true;
        }
    }

    let w_positions = raw
        .iter()
        .enumerate()
        .filter_map(|(i, &ch)| (lower(ch) == 'w').then_some(i))
        .collect::<Vec<_>>();
    if w_positions.len() >= 2 && consumed[w_positions[0]] && w_positions[0] + 1 == w_positions[1] {
        let last_applies_distinct_mark = marks
            .iter()
            .enumerate()
            .any(|(index, mark)| index > w_positions[0] && *mark != VowelMark::None);
        if !last_applies_distinct_mark {
            for (v_idx, mark) in marks.iter_mut().enumerate() {
                if matches!(mark, VowelMark::Breve | VowelMark::Horn) {
                    *mark = VowelMark::None;
                    // If a duplicate vowel between `v_idx` and `w_positions[0]`
                    // was consumed by an earlier circumflex overridden by `w`
                    // (e.g. `aaww` -> `aaw`, `ooww` -> `oow`), restore it.
                    let base = lower(raw[v_idx]);
                    for k in (v_idx + 1)..w_positions[0] {
                        if lower(raw[k]) == base && consumed[k] {
                            consumed[k] = false;
                        }
                    }
                }
            }
            consumed[w_positions[0]] = true;
            for &pos in &w_positions[1..] {
                consumed[pos] = false;
            }
            mark_undone = true;
        }
    }

    for window in raw.windows(3).enumerate() {
        let (start, chars) = window;
        if chars.iter().all(|&ch| lower(ch) == 'd') && can_apply_dd_at(raw, start) {
            letters[start] = raw[start];
            letters[start + 1] = raw[start + 1];
            letters[start + 2] = if raw[start + 1].is_uppercase() {
                raw[start + 2].to_ascii_uppercase()
            } else {
                raw[start + 2]
            };
            consumed[start] = false;
            consumed[start + 1] = true;
            consumed[start + 2] = false;
            mark_undone = true;
        }
    }

    MarkResolution {
        letters,
        marks,
        consumed,
        mark_undone,
    }
}
