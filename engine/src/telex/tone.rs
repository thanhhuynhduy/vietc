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
    if first_vowel > 0 {
        let onset: String = raw[..first_vowel]
            .iter()
            .enumerate()
            .filter_map(|(idx, &ch)| (!consumed[idx]).then_some(lower(ch)))
            .collect();
        let valid_onset = matches!(
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
                | "z"
        );
        if !valid_onset {
            return false;
        }
    }
    let last_vowel = *vowels.last().unwrap();
    if last_vowel > i {
        // If no consonant separates `i` from a following vowel (e.g. `<vowel> + <s/f/r/x/j> + <vowel>`
        // as in "more", "core", "lose", "rose", "nose", "these", "here", "there", "where",
        // "were", "care", "base", "case", "use", "pure", "cure", "hire", "fire", "house",
        // "mouse", "pause", "cause", "user", "laser", "nasa", "para", "sara", "exe",
        // versus valid free-order Vietnamese "chafo" -> "chào", "neesu" -> "nếu"),
        // require the contiguous vowels around `i` to form a valid Vietnamese vowel cluster.
        let next_unconsumed = (i + 1..raw.len()).find(|&k| !consumed[k]);
        if next_unconsumed.is_some_and(|k| is_vowel(raw[k])) {
            let glide_idx = if raw.len() > 2
                && matches!((lower(raw[0]), lower(raw[1])), ('q', 'u') | ('g', 'i'))
                && is_vowel(raw[2])
            {
                Some(1)
            } else {
                None
            };
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
            let has_coda = last_vowel + 1 < raw.len();
            let valid_cluster = match cluster.as_str() {
                "ai" | "ao" | "au" | "ay" | "âu" | "ây" | "eo" | "êu" | "ia" | "iu" | "ôi"
                | "ơi" | "ua" | "ui" | "ưa" | "ưi" | "ưu" | "oai" | "oao" | "oay" | "oeo"
                | "iêu" | "yêu" | "uôi" | "ươi" | "ươu" | "uây" | "uya" | "uyu" => !has_coda,
                "oa" | "oe" | "uê" | "uy" => true,
                "iê" | "yê" | "uô" | "ươ" | "oă" | "uâ" | "uơ" | "uyê" => true,
                _ => false,
            };
            if !valid_cluster || (cluster == "oe" && lower(raw[last_vowel]) == 'e') {
                return false;
            }
        }
    }
    true
}

pub fn resolve(raw: &[char], marks: &MarkResolution) -> ToneResolution {
    let mut consumed = marks.consumed.clone();
    if marks.mark_undone {
        return ToneResolution {
            tone: Tone::None,
            target: None,
            consumed,
        };
    }

    let vowels: Vec<usize> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, &ch)| (!consumed[i] && is_vowel(ch)).then_some(i))
        .collect();

    // Scan consecutive identical tone triggers after a vowel (e.g. "tess",
    // "tesst", "tesster", "offfer", "isssue"). The first trigger is consumed
    // and the second is kept as a literal character, cancelling tone placement
    // for the rest of the word. Do not match non-adjacent duplicate letters in
    // multi-syllable words like "order", "border", "server", "sister".
    let mut active_trigger: Option<(usize, Tone)> = None;
    for (i, &ch) in raw.iter().enumerate() {
        if consumed[i] {
            continue;
        }
        if let Some(tone) = tone_for(ch) {
            let has_vowel_before = vowels.iter().any(|&v| v < i);
            if !has_vowel_before {
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

    let mut selected = None;
    for (i, &ch) in raw.iter().enumerate() {
        if is_valid_tone_trigger(raw, marks, &consumed, &vowels, i) {
            if let Some(tone) = tone_for(ch) {
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

    let previous_tone_triggers = (0..trigger)
        .filter_map(|index| {
            if !is_valid_tone_trigger(raw, marks, &consumed, &vowels, index) {
                return None;
            }
            let previous = tone_for(raw[index])?;
            Some((index, previous))
        })
        .collect::<Vec<_>>();
    if let Some(&(prev_idx, previous_tone)) = previous_tone_triggers.last() {
        if previous_tone == tone && prev_idx + 1 != trigger {
            // Non-adjacent duplicate tone triggers across other letters (e.g. "order",
            // "border", "server", "sister"): keep the earlier tone trigger active and
            // leave the trailing letter unconsumed as a literal character.
            let (first_idx, first_tone) = previous_tone_triggers[0];
            for &(idx, _) in &previous_tone_triggers[..previous_tone_triggers.len() - 1] {
                consumed[idx] = true;
            }
            consumed[prev_idx] = true;
            let _ = (first_idx, first_tone);
            return compute_target(raw, marks, consumed, vowels, prev_idx, previous_tone);
        }
        for (previous_trigger, _) in &previous_tone_triggers {
            consumed[*previous_trigger] = true;
        }
        if previous_tone == tone {
            consumed[trigger] = false;
            return ToneResolution {
                tone: Tone::None,
                target: None,
                consumed,
            };
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

    let glide_vowel = if raw.len() > 2
        && matches!((lower(raw[0]), lower(raw[1])), ('q', 'u') | ('g', 'i'))
        && is_vowel(raw[2])
    {
        Some(1)
    } else {
        None
    };
    let last_vowel = vowels.last().copied();
    let include_later_vowels = last_vowel.is_some_and(|last| {
        last > trigger
            && raw[trigger + 1..last].iter().all(|ch| {
                is_vowel(*ch)
                    || tone_for(*ch).is_some()
                    || matches!(lower(*ch), 'w' | 'a' | 'e' | 'o')
            })
            && (last + 1 == raw.len()
                || marks.marks[..=last]
                    .iter()
                    .any(|mark| *mark != VowelMark::None))
    });
    let active: Vec<usize> = vowels
        .into_iter()
        .filter(|&i| (i < trigger || include_later_vowels) && Some(i) != glide_vowel)
        .collect();
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
                if matches!(
                    pair,
                    ('o', 'a') | ('o', 'e') | ('u', 'y') | ('i', 'e') | ('y', 'e') | ('u', 'o')
                ) {
                    Some(active[1])
                } else {
                    Some(active[0])
                }
            }
            _ => active.get(1).copied(),
        });

    ToneResolution {
        tone,
        target,
        consumed,
    }
}
