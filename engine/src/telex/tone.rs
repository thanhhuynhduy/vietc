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

pub fn resolve(raw: &[char], marks: &MarkResolution) -> ToneResolution {
    let mut consumed = marks.consumed.clone();
    let vowels: Vec<usize> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, &ch)| (!consumed[i] && is_vowel(ch)).then_some(i))
        .collect();

    let mut selected = None;
    for (i, &ch) in raw.iter().enumerate() {
        if consumed[i] {
            continue;
        }
        if let Some(tone) = tone_for(ch) {
            let has_vowel_before = vowels.iter().any(|&v| v < i);
            let has_vowel_after = vowels.iter().any(|&v| v > i);
            if has_vowel_before
                && (has_vowel_after
                    || i + 1 == raw.len()
                    || raw[i + 1..].iter().all(|c| !is_vowel(*c)))
            {
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

    let previous_tone_triggers = raw[..trigger]
        .iter()
        .enumerate()
        .filter_map(|(index, ch)| {
            let previous = tone_for(*ch)?;
            let has_vowel_before = raw[..index].iter().any(|candidate| is_vowel(*candidate));
            has_vowel_before.then_some((index, previous))
        })
        .collect::<Vec<_>>();
    if let Some((_, previous_tone)) = previous_tone_triggers.last() {
        for (previous_trigger, _) in &previous_tone_triggers {
            consumed[*previous_trigger] = true;
        }
        if *previous_tone == tone {
            consumed[trigger] = false;
            return ToneResolution {
                tone: Tone::None,
                target: None,
                consumed,
            };
        }
    }

    consumed[trigger] = true;

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
