// SPDX-License-Identifier: MIT
use super::mark::MarkResolution;
use super::raw::{Tone, VowelMark};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToneResolution {
    pub tone: Tone,
    pub target: Option<usize>,
    pub consumed: Vec<bool>,
}

fn lower(ch: char) -> char {
    ch.to_ascii_lowercase()
}

fn tone_for(ch: char) -> Option<Tone> {
    match lower(ch) {
        's' => Some(Tone::Acute),
        'f' => Some(Tone::Grave),
        'r' => Some(Tone::Hook),
        'x' => Some(Tone::Tilde),
        'j' => Some(Tone::Dot),
        _ => None,
    }
}

fn is_vowel(ch: char) -> bool {
    matches!(lower(ch), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

fn detect_onset(raw: &[char], consumed: &[bool], first_vowel: usize) -> String {
    raw[..first_vowel]
        .iter()
        .enumerate()
        .filter_map(|(idx, &ch)| (!consumed[idx]).then_some(lower(ch)))
        .collect()
}

fn is_valid_onset_str(onset: &str) -> bool {
    matches!(
        onset,
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

/// Detect `qu` or `gi` glide index (index 1) even when free-order tone/mark
/// triggers sit right after index 1 (e.g. `gifa` -> `già`, `qusa` -> `quá`).
fn detect_glide_vowel(raw: &[char], consumed: &[bool], ignore_idx: Option<usize>) -> Option<usize> {
    if raw.len() <= 2 || consumed[0] || consumed[1] {
        return None;
    }
    if !matches!((lower(raw[0]), lower(raw[1])), ('q', 'u') | ('g', 'i')) {
        return None;
    }
    let next_unconsumed = (2..raw.len()).find(|&k| {
        !consumed[k] && Some(k) != ignore_idx && tone_for(raw[k]).is_none() && lower(raw[k]) != 'z'
    });
    if next_unconsumed.is_some_and(|k| is_vowel(raw[k])) {
        Some(1)
    } else {
        None
    }
}

fn is_valid_vietnamese_coda(coda: &str) -> bool {
    matches!(coda, "" | "c" | "ch" | "m" | "n" | "ng" | "nh" | "p" | "t")
}

fn is_valid_tone_trigger(
    raw: &[char],
    marks: &MarkResolution,
    consumed: &[bool],
    vowels: &[usize],
    i: usize,
) -> bool {
    if consumed[i] || tone_for(raw[i]).is_none() {
        return false;
    }
    let Some(&first_vowel) = vowels.first() else {
        return false;
    };
    if first_vowel >= i {
        return false;
    }
    let onset = if first_vowel > 0 {
        detect_onset(raw, consumed, first_vowel)
    } else {
        String::new()
    };
    if !is_valid_onset_str(&onset) {
        return false;
    }
    let last_vowel = *vowels.last().unwrap();
    if last_vowel > i {
        let next_unconsumed = (i + 1..raw.len()).find(|&k| !consumed[k]);
        if next_unconsumed.is_some_and(|k| is_vowel(raw[k])) {
            let glide_idx = detect_glide_vowel(raw, consumed, Some(i));
            let mut cluster = String::new();
            for &v in vowels {
                if Some(v) == glide_idx {
                    continue;
                }
                let base = lower(raw[v]);
                let ch = match (base, marks.marks[v]) {
                    ('a', VowelMark::Breve) => 'ă',
                    ('a', VowelMark::Circumflex) => 'â',
                    ('e', VowelMark::Circumflex) => 'ê',
                    ('o', VowelMark::Circumflex) => 'ô',
                    ('o', VowelMark::Horn) => 'ơ',
                    ('u', VowelMark::Horn) => 'ư',
                    _ => base,
                };
                cluster.push(ch);
            }
            let coda: String = raw[last_vowel + 1..]
                .iter()
                .enumerate()
                .filter_map(|(offset, &ch)| {
                    let idx = last_vowel + 1 + offset;
                    (!consumed[idx] && tone_for(ch).is_none() && lower(ch) != 'z')
                        .then_some(lower(ch))
                })
                .collect();
            let has_coda = !coda.is_empty();
            let valid_cluster = match cluster.as_str() {
                "a" | "ă" | "â" | "e" | "ê" | "i" | "o" | "ô" | "ơ" | "u" | "ư" | "y" => {
                    glide_idx.is_some() && is_valid_vietnamese_coda(&coda)
                }
                "ai" | "ao" | "au" | "ay" | "âu" | "ây" | "eo" | "êu" | "ia" | "iu" | "ôi"
                | "ơi" | "ua" | "ui" | "ưa" | "ưi" | "ưu" | "oai" | "oao" | "oay" | "oeo"
                | "iêu" | "yêu" | "uôi" | "ươi" | "ươu" | "uây" | "uya" | "uyu" => {
                    !has_coda
                }
                "oa" | "oe" | "uê" | "uy" => is_valid_vietnamese_coda(&coda),
                "iê" | "yê" | "uô" | "ươ" | "oă" | "uâ" | "uơ" | "uyê" => {
                    is_valid_vietnamese_coda(&coda)
                }
                _ => false,
            };
            // Keep English silent-e words ("more", "core", "lose", "rose", "nose", "pose",
            // "dose", "chose", "those", "worse") intact mid-word, while allowing genuine Vietnamese
            // `oe` syllables (`khore` -> `khỏe`, `hofe` -> `hòe`, `xofe` -> `xòe`).
            let english_silent_oe = cluster == "oe"
                && !has_coda
                && matches!(
                    onset.as_str(),
                    "m" | "c" | "l" | "r" | "n" | "p" | "d" | "ch" | "th" | "w"
                );
            if !valid_cluster || english_silent_oe {
                return false;
            }
        }
    }
    true
}

pub fn resolve(raw: &[char], marks: &MarkResolution) -> ToneResolution {
    let mut consumed = marks.consumed.clone();
    let vowels: Vec<usize> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, &ch)| (!consumed[i] && is_vowel(ch)).then_some(i))
        .collect();

    if marks.mark_undone {
        let is_closed_vn_oo = vowels.len() == 2
            && lower(raw[vowels[0]]) == 'o'
            && lower(raw[vowels[1]]) == 'o'
            && vowels[0]
                .checked_sub(0)
                .is_some_and(|fv| is_valid_onset_str(&detect_onset(raw, &consumed, fv)))
            && {
                let coda: String = raw[vowels[1] + 1..]
                    .iter()
                    .enumerate()
                    .filter_map(|(off, &ch)| {
                        let idx = vowels[1] + 1 + off;
                        (!consumed[idx] && tone_for(ch).is_none() && lower(ch) != 'z')
                            .then_some(lower(ch))
                    })
                    .collect();
                coda == "ng"
                    || (coda == "c"
                        && raw[vowels[1] + 1..]
                            .iter()
                            .any(|&ch| matches!(tone_for(ch), Some(Tone::Acute | Tone::Dot))))
            };
        if !is_closed_vn_oo {
            return ToneResolution {
                tone: Tone::None,
                target: None,
                consumed,
            };
        }
    }

    // Scan consecutive identical tone triggers after a valid Vietnamese onset + vowel
    // (e.g. "tess", "tesst", "offfer", "isssue", "fixx").
    let first_vowel = vowels.first().copied();
    let onset_allows_escape = first_vowel.is_some_and(|fv| {
        if fv == 0 {
            true
        } else {
            let onset = detect_onset(raw, &consumed, fv);
            is_valid_onset_str(&onset)
                || (onset.chars().count() == 1 && !matches!(onset.as_str(), "w" | "j" | "z"))
        }
    });

    let mut active_trigger: Option<(usize, Tone)> = None;
    for (i, &ch) in raw.iter().enumerate() {
        if consumed[i] {
            continue;
        }
        if let Some(tone) = tone_for(ch) {
            let has_vowel_before = vowels.iter().any(|&v| v < i);
            if !has_vowel_before || !onset_allows_escape {
                continue;
            }
            if let Some((prev_idx, prev_tone)) = active_trigger {
                if prev_idx + 1 == i {
                    consumed[prev_idx] = true;
                    if prev_tone == tone {
                        consumed[i] = false;
                        return ToneResolution {
                            tone: Tone::None,
                            target: None,
                            consumed,
                        };
                    }
                }
            }
            active_trigger = Some((i, tone));
        } else {
            active_trigger = None;
        }
    }

    let mut selected: Option<(usize, Tone)> = None;
    let mut history: Vec<(usize, Tone)> = Vec::new();

    for (i, &ch) in raw.iter().enumerate() {
        if consumed[i] {
            continue;
        }
        if lower(ch) == 'z' {
            // Telex 'z' strips the currently active tone (`toansz` -> `toan`).
            // If no tone is active (`size`, `zero`, `toanszz`), 'z' stays literal!
            if let Some((prev_idx, _)) = selected.take() {
                consumed[prev_idx] = true;
                for (h_idx, _) in history.drain(..) {
                    consumed[h_idx] = true;
                }
                consumed[i] = true;
            }
            continue;
        }
        if is_valid_tone_trigger(raw, marks, &consumed, &vowels, i) {
            if let Some(tone) = tone_for(ch) {
                if let Some(prev) = selected {
                    history.push(prev);
                }
                selected = Some((i, tone));
            }
        }
    }

    let Some((trigger, tone)) = selected else {
        return ToneResolution {
            tone: Tone::None,
            target: None,
            consumed,
        };
    };

    if let Some(&(prev_idx, previous_tone)) = history.last() {
        if previous_tone == tone && prev_idx + 1 != trigger {
            for &(idx, _) in &history[..history.len() - 1] {
                consumed[idx] = true;
            }
            consumed[prev_idx] = true;
            return compute_target(raw, marks, consumed, vowels, prev_idx, previous_tone);
        }
        for (previous_trigger, _) in &history {
            consumed[*previous_trigger] = true;
        }
    }

    consumed[trigger] = true;
    compute_target(raw, marks, consumed, vowels, trigger, tone)
}

fn compute_target(
    raw: &[char],
    marks: &MarkResolution,
    consumed: Vec<bool>,
    vowels: Vec<usize>,
    trigger: usize,
    tone: Tone,
) -> ToneResolution {
    let glide_vowel = detect_glide_vowel(raw, &consumed, None);
    let last_vowel = vowels.last().copied();
    let coda: String = last_vowel
        .map(|last| {
            raw[last + 1..]
                .iter()
                .enumerate()
                .filter_map(|(offset, &ch)| {
                    let idx = last + 1 + offset;
                    (!consumed[idx]).then_some(lower(ch))
                })
                .collect()
        })
        .unwrap_or_default();
    let has_coda = !coda.is_empty();

    let include_later_vowels = last_vowel.is_some_and(|last| {
        last > trigger
            && raw[trigger + 1..last].iter().all(|ch| {
                is_vowel(*ch)
                    || tone_for(*ch).is_some()
                    || matches!(lower(*ch), 'w' | 'a' | 'e' | 'o' | 'z')
            })
            && is_valid_vietnamese_coda(&coda)
    });

    let active: Vec<usize> = vowels
        .into_iter()
        .filter(|&i| (i < trigger || include_later_vowels) && Some(i) != glide_vowel)
        .collect();

    // Old-Style Vietnamese Tone Placement ("Kiểu cũ"):
    // - Marked vowels (`ă`, `â`, `ê`, `ô`, `ơ`, `ư`) always take the tone (for `ươ`, `ơ` takes it).
    // - 3 unmarked vowels (`oai`, `oay`, `oeo`, `uyu`, `uya`): middle vowel (`active[1]`).
    // - 2 unmarked vowels:
    //   * Open syllable (`!has_coda`, e.g. `hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`): 1st vowel (`active[0]`)!
    //   * Closed syllable (`has_coda`, e.g. `toán`, `hoàn`, `huỳnh`, `tuýt`, `tién`, `muón`): 2nd vowel (`active[1]`)!
    let target = active
        .iter()
        .rev()
        .copied()
        .find(|&i| marks.marks[i] != VowelMark::None)
        .or_else(|| match active.len() {
            0 => None,
            1 => active.first().copied(),
            2 => {
                let pair = (lower(raw[active[0]]), lower(raw[active[1]]));
                if has_coda
                    && matches!(
                        pair,
                        ('o', 'a')
                            | ('o', 'e')
                            | ('o', 'o')
                            | ('u', 'y')
                            | ('i', 'e')
                            | ('y', 'e')
                            | ('u', 'o')
                    )
                {
                    Some(active[1])
                } else {
                    Some(active[0])
                }
            }
            _ => active.get(1).copied(),
        });

    ToneResolution {
        tone: if target.is_some() { tone } else { Tone::None },
        target,
        consumed,
    }
}
