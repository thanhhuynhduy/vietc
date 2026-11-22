// SPDX-License-Identifier: MIT
use crate::input_method::{get_rules, InputMethod, InputMethodRules, UndoBehavior};

#[derive(Debug, Clone)]
struct Transformation {
    base_char: char,
    mark_applied: Option<char>,
    tone_applied: Option<char>,
    is_upper: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Vietnamese,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelexUndo {
    AppendTrigger,
    ConsumeTrigger,
}

impl Mode {
    fn is_vn(self) -> bool {
        matches!(self, Mode::Vietnamese)
    }
}

#[derive(Clone)]
pub struct BambooEngine {
    composition: Vec<Transformation>,
    rules: InputMethodRules,
    mode: Mode,
    undone_in_word: bool,
}

impl BambooEngine {
    pub fn new(method: InputMethod) -> Self {
        Self {
            composition: Vec::new(),
            rules: get_rules(method),
            mode: Mode::Vietnamese,
            undone_in_word: false,
        }
    }

    pub fn set_method(&mut self, method: InputMethod) {
        self.rules = get_rules(method);
        self.reset();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.mode = if enabled {
            Mode::Vietnamese
        } else {
            Mode::English
        };
        if !enabled {
            self.reset();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.mode.is_vn()
    }

    pub fn reset(&mut self) {
        self.composition.clear();
        self.undone_in_word = false;
    }

    pub fn process_key(&mut self, ch: char) -> Option<String> {
        if !self.mode.is_vn() {
            return Some(ch.to_string());
        }

        let lower = ch.to_ascii_lowercase();

        if self.undone_in_word {
            if let Some(&(tone_char, _)) = self.rules.tone_keys.get(&lower) {
                let syllable_str: String = self.composition[self.last_syllable_range()]
                    .iter()
                    .map(|t| t.mark_applied.unwrap_or(t.base_char).to_ascii_lowercase())
                    .collect();
                let (_, vo, lc) = crate::spelling::partition(&syllable_str);
                if vo == "oo"
                    && (lc == "ng" || (lc == "c" && matches!(lower, 's' | 'j' | '1' | '5')))
                {
                    return self.apply_tone(tone_char);
                }
            }
            self.append_char(ch);
            return Some(self.flatten());
        }

        // Tone removal key ('z' in Telex, '0' in VNI) when a tone is currently active
        if (self.rules.method == InputMethod::Telex && lower == 'z')
            || (self.rules.method == InputMethod::Vni && lower == '0')
        {
            let last_syllable = self.last_syllable_range();
            let had_tone = self.composition[last_syllable.clone()]
                .iter()
                .any(|t| t.tone_applied.is_some());
            if had_tone {
                for idx in last_syllable {
                    if let Some(t) = self.composition.get_mut(idx) {
                        t.tone_applied = None;
                    }
                }
                return Some(self.flatten());
            }
        }

        // Check tone keys — only apply if composition has a vowel, else treat as normal char
        if let Some(&(tone_char, _tone_name)) = self.rules.tone_keys.get(&lower) {
            let last_syllable = self.last_syllable_range();
            let has_vowel = self.composition[last_syllable]
                .iter()
                .any(|t| is_vowel(t.mark_applied.unwrap_or(t.base_char)));
            if has_vowel {
                return self.apply_tone(tone_char);
            }
        }

        // VNI '9' -> 'đ' at syllable start (supports free-order `dan9` -> `đan`, `d99` -> `d9`)
        if self.rules.method == InputMethod::Vni && lower == '9' && !self.composition.is_empty() {
            let last_syllable = self.last_syllable_range();
            if last_syllable.start < last_syllable.end {
                let first_idx = last_syllable.start;
                if self.composition[first_idx].mark_applied == Some('đ') {
                    let was_upper = self.composition[first_idx].is_upper;
                    self.composition[first_idx].base_char = if was_upper { 'D' } else { 'd' };
                    self.composition[first_idx].mark_applied = None;
                    self.undone_in_word = true;
                    self.append_char(ch);
                    return Some(self.flatten());
                } else if self.composition[first_idx]
                    .base_char
                    .eq_ignore_ascii_case(&'d')
                {
                    self.composition[first_idx].base_char = 'đ';
                    self.composition[first_idx].mark_applied = Some('đ');
                    return Some(self.flatten());
                }
            }
        }

        // Smart "uo" / "uou" → "ươ" / "ươu" shortcut with flexible backtrack
        if self.rules.method == InputMethod::Telex && lower == 'w'
            || self.rules.method == InputMethod::Vni && lower == '7'
        {
            if self.composition.len() >= 2 {
                // Check `-ươu` triphthong (`huou7` / `huouw` -> `hươu`)
                if self.composition.len() >= 3 {
                    let last_idx = self.composition.len() - 1;
                    let c_last = self.composition[last_idx].base_char.to_ascii_lowercase();
                    let c_mid = self.composition[last_idx - 1]
                        .base_char
                        .to_ascii_lowercase();
                    let c_first = self.composition[last_idx - 2]
                        .base_char
                        .to_ascii_lowercase();
                    let preceded_by_q = last_idx >= 3
                        && self.composition[last_idx - 3]
                            .base_char
                            .eq_ignore_ascii_case(&'q');
                    if c_last == 'u' && c_mid == 'o' && c_first == 'u' && !preceded_by_q {
                        let last_syllable = self.last_syllable_range();
                        let tone = self.composition[last_syllable]
                            .iter()
                            .find_map(|t| t.tone_applied);
                        self.composition[last_idx - 2].base_char = 'ư';
                        self.composition[last_idx - 2].mark_applied = Some('ư');
                        self.composition[last_idx - 1].base_char = 'ơ';
                        self.composition[last_idx - 1].mark_applied = Some('ơ');
                        if let Some(tone_char) = tone {
                            return self.apply_tone(tone_char);
                        }
                        return Some(self.flatten());
                    }
                }

                let last_syllable = self.last_syllable_range();
                for offset in 0..5usize.min(self.composition.len() - 1) {
                    let o_idx = self.composition.len() - 1 - offset;
                    if o_idx <= last_syllable.start {
                        break;
                    }
                    let o_ch = self.composition[o_idx].base_char.to_ascii_lowercase();
                    if o_ch == 'o' && o_idx > 0 {
                        let u_idx = o_idx - 1;
                        if u_idx < last_syllable.start {
                            break;
                        }
                        let u_ch = self.composition[u_idx].base_char.to_ascii_lowercase();
                        let preceded_by_q = u_idx > 0
                            && self.composition[u_idx - 1]
                                .base_char
                                .eq_ignore_ascii_case(&'q');
                        if u_ch == 'u' && !preceded_by_q {
                            let onset: String = self.composition[last_syllable.start..u_idx]
                                .iter()
                                .map(|t| t.mark_applied.unwrap_or(t.base_char).to_ascii_lowercase())
                                .collect();
                            let open_uo = o_idx == self.composition.len() - 1
                                && matches!(onset.as_str(), "h" | "kh" | "th");
                            let tone = self.composition[last_syllable]
                                .iter()
                                .find_map(|t| t.tone_applied);
                            let u_was_upper = self.composition[u_idx].is_upper;
                            let o_was_upper = self.composition[o_idx].is_upper;
                            self.composition.drain(u_idx..=o_idx);
                            self.composition.insert(
                                u_idx,
                                Transformation {
                                    base_char: if open_uo { 'u' } else { 'ư' },
                                    mark_applied: if open_uo { None } else { Some('ư') },
                                    tone_applied: None,
                                    is_upper: u_was_upper,
                                },
                            );
                            self.composition.insert(
                                u_idx + 1,
                                Transformation {
                                    base_char: 'ơ',
                                    mark_applied: Some('ơ'),
                                    tone_applied: None,
                                    is_upper: o_was_upper,
                                },
                            );
                            if let Some(tone_char) = tone {
                                return self.apply_tone(tone_char);
                            }
                            return Some(self.flatten());
                        }
                    }
                    // Allow scanning past trailing 'i'/'u' glide for free-order `-ươi`/`-ươu`
                    // (e.g. VNI `nguoi72` -> `người`, `tuoi7` -> `tươi`, `huou7` -> `hươu`).
                    if (o_ch == 'u' || is_vowel(o_ch))
                        && !(offset == 0 && matches!(o_ch, 'i' | 'u'))
                    {
                        break;
                    }
                }
            }

            if self.composition.len() >= 2 {
                let a_idx = self.composition.len() - 1;
                let u_idx = a_idx - 1;
                let a_ch = self.composition[a_idx].base_char.to_ascii_lowercase();
                let u_ch = self.composition[u_idx].base_char.to_ascii_lowercase();
                let preceded_by_q = u_idx > 0
                    && self.composition[u_idx - 1]
                        .base_char
                        .eq_ignore_ascii_case(&'q');
                if a_ch == 'a'
                    && u_ch == 'u'
                    && self.composition[u_idx].mark_applied.is_none()
                    && !preceded_by_q
                {
                    self.composition[u_idx].base_char = 'ư';
                    self.composition[u_idx].mark_applied = Some('ư');
                    return Some(self.flatten());
                }
            }
        }

        let mark_match = self.find_mark_backtrack(lower);

        if let Some((idx, pattern, result)) = mark_match {
            self.apply_mark_at(idx, &pattern, &result);
            let last_syllable = self.last_syllable_range();
            let existing_tone = self.composition[last_syllable]
                .iter()
                .find_map(|t| t.tone_applied);
            if let Some(tone_char) = existing_tone {
                return self.apply_tone(tone_char);
            }
            return Some(self.flatten());
        }

        // VNI shape override (`a68` -> `ă`, `o67` -> `ơ`) & Telex `w` shape override (`aaw` -> `ă`, `oow` -> `ơ`)
        if self.rules.method == InputMethod::Vni && matches!(lower, '6' | '7' | '8') {
            let last_syllable = self.last_syllable_range();
            for idx in last_syllable.rev() {
                if let Some(marked) = self.composition[idx].mark_applied {
                    let raw_base = match marked {
                        'ă' | 'â' => Some('a'),
                        'ê' => Some('e'),
                        'ô' | 'ơ' => Some('o'),
                        'ư' => Some('u'),
                        _ => None,
                    };
                    if let Some(base) = raw_base {
                        let next_mark = match (base, lower) {
                            ('a', '6') => Some('â'),
                            ('e', '6') => Some('ê'),
                            ('o', '6') => Some('ô'),
                            ('o', '7') => Some('ơ'),
                            ('u', '7') => Some('ư'),
                            ('a', '8') => Some('ă'),
                            _ => None,
                        };
                        if let Some(target_mark) = next_mark {
                            if target_mark == marked {
                                self.composition[idx].base_char = base;
                                self.composition[idx].mark_applied = None;
                                if target_mark == 'ơ'
                                    && idx > 0
                                    && self.composition[idx - 1].mark_applied == Some('ư')
                                {
                                    self.composition[idx - 1].base_char =
                                        if self.composition[idx - 1].is_upper {
                                            'U'
                                        } else {
                                            'u'
                                        };
                                    self.composition[idx - 1].mark_applied = None;
                                }
                                self.undone_in_word = true;
                                self.composition.push(Transformation {
                                    base_char: ch,
                                    mark_applied: None,
                                    tone_applied: None,
                                    is_upper: ch.is_uppercase(),
                                });
                                return Some(self.flatten());
                            }
                            self.composition[idx].base_char = target_mark;
                            self.composition[idx].mark_applied = Some(target_mark);
                            return Some(self.flatten());
                        }
                    }
                }
            }
        } else if self.rules.method == InputMethod::Telex && lower == 'w' {
            if let Some(last) = self.composition.last_mut() {
                let override_mark = match last.mark_applied {
                    Some('â') => Some('ă'),
                    Some('ô') => Some('ơ'),
                    _ => None,
                };
                if let Some(target_mark) = override_mark {
                    last.base_char = target_mark;
                    last.mark_applied = Some(target_mark);
                    return Some(self.flatten());
                }
            }
        }

        // If a vowel is typed immediately after a vowel that just absorbed a
        // Telex tone key (e.g. "mỏ" + 'e' -> "more", "thé" + 'e' -> "these",
        // "ná" + 'a' -> "nasa"), check whether the resulting vowel cluster is a
        // valid free-order Vietnamese diphthong (such as "chà" + 'o' -> "chào",
        // "nế" + 'u' -> "nếu"). If not, restore the tone key as a literal letter.
        if self.rules.method == InputMethod::Telex && is_vowel(lower) {
            if let Some(last) = self.composition.last() {
                if let Some(tone_char) = last.tone_applied {
                    let last_syllable = self.last_syllable_range();
                    let mut vowels: Vec<char> = self.composition[last_syllable.clone()]
                        .iter()
                        .filter_map(|t| {
                            let c = t.mark_applied.unwrap_or(t.base_char).to_ascii_lowercase();
                            is_vowel(c).then_some(c)
                        })
                        .collect();
                    let first_vowel_pos = self.composition[last_syllable.clone()]
                        .iter()
                        .position(|t| is_vowel(t.mark_applied.unwrap_or(t.base_char)));
                    let onset: String = first_vowel_pos
                        .map(|fv| {
                            self.composition[last_syllable.start..last_syllable.start + fv]
                                .iter()
                                .map(|t| t.mark_applied.unwrap_or(t.base_char).to_ascii_lowercase())
                                .collect()
                        })
                        .unwrap_or_default();
                    let had_glide = if !vowels.is_empty() && last_syllable.len() >= 2 {
                        let c0 = self.composition[last_syllable.start]
                            .base_char
                            .to_ascii_lowercase();
                        let c1 = self.composition[last_syllable.start + 1]
                            .base_char
                            .to_ascii_lowercase();
                        if (c0 == 'q' && c1 == 'u') || (c0 == 'g' && c1 == 'i') {
                            vowels.remove(0);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    vowels.push(lower);
                    let cluster: String = vowels.into_iter().collect();
                    let english_silent_oe = cluster == "oe"
                        && matches!(
                            onset.as_str(),
                            "m" | "c" | "l" | "r" | "n" | "p" | "d" | "ch" | "th"
                        );
                    let valid_free_order = !english_silent_oe
                        && ((had_glide
                            && matches!(
                                cluster.as_str(),
                                "a" | "ă"
                                    | "â"
                                    | "e"
                                    | "ê"
                                    | "i"
                                    | "o"
                                    | "ô"
                                    | "ơ"
                                    | "u"
                                    | "ư"
                                    | "y"
                            ))
                            || matches!(
                                cluster.as_str(),
                                "ai" | "ao"
                                    | "au"
                                    | "ay"
                                    | "âu"
                                    | "ây"
                                    | "eo"
                                    | "êu"
                                    | "ia"
                                    | "iu"
                                    | "ôi"
                                    | "ơi"
                                    | "ua"
                                    | "ui"
                                    | "ưa"
                                    | "ưi"
                                    | "ưu"
                                    | "oa"
                                    | "oe"
                                    | "uê"
                                    | "uy"
                                    | "oai"
                                    | "oao"
                                    | "oay"
                                    | "oeo"
                                    | "iêu"
                                    | "yêu"
                                    | "uôi"
                                    | "ươi"
                                    | "ươu"
                                    | "uây"
                                    | "uya"
                                    | "uyu"
                                    | "iê"
                                    | "yê"
                                    | "uô"
                                    | "ươ"
                                    | "oă"
                                    | "uâ"
                                    | "uơ"
                                    | "uyê"
                            ));
                    if !valid_free_order {
                        if let Some(last_mut) = self.composition.last_mut() {
                            last_mut.tone_applied = None;
                        }
                        self.append_char(tone_char);
                        self.append_char(ch);
                        return Some(self.flatten());
                    }
                }
            }
        }

        self.append_char(ch);
        if lower.is_ascii_alphabetic() {
            let len = self.composition.len();
            if len >= 3
                && !(len >= 4
                    && self.composition[len - 4]
                        .base_char
                        .eq_ignore_ascii_case(&'q'))
            {
                if self.composition[len - 2].mark_applied == Some('ơ')
                    && self.composition[len - 3]
                        .base_char
                        .eq_ignore_ascii_case(&'u')
                    && self.composition[len - 3].mark_applied.is_none()
                {
                    self.composition[len - 3].base_char = if self.composition[len - 3].is_upper {
                        'Ư'
                    } else {
                        'ư'
                    };
                    self.composition[len - 3].mark_applied = Some('ư');
                } else if self.composition[len - 2]
                    .base_char
                    .eq_ignore_ascii_case(&'o')
                    && self.composition[len - 2].mark_applied.is_none()
                    && self.composition[len - 3].mark_applied == Some('ư')
                {
                    self.composition[len - 2].base_char = if self.composition[len - 2].is_upper {
                        'Ơ'
                    } else {
                        'ơ'
                    };
                    self.composition[len - 2].mark_applied = Some('ơ');
                }
            }
            let last_syllable = self.last_syllable_range();
            let existing_tone = self.composition[last_syllable.clone()]
                .iter()
                .find_map(|t| t.tone_applied);
            if let Some(tone_char) = existing_tone {
                return self.apply_tone(tone_char);
            }
        }
        Some(self.flatten())
    }

    fn find_mark_backtrack(&self, lower: char) -> Option<(usize, String, String)> {
        let scan_limit = 5usize.min(self.composition.len());
        for offset in 0..scan_limit {
            let idx = self.composition.len() - 1 - offset;
            let ch = self.composition[idx].base_char.to_ascii_lowercase();
            let seq = format!("{}{}", ch, lower);
            if let Some(rule) = self
                .rules
                .mark_rules
                .iter()
                .find(|rule| seq == rule.pattern)
            {
                if rule.pattern == "dd" && offset > 0 {
                    continue;
                }
                if matches!(rule.pattern.as_str(), "aa" | "ee" | "oo") {
                    if self.composition[idx].tone_applied.is_some() {
                        continue;
                    }
                    if offset > 0
                        && (!self.composition[idx + 1..]
                            .iter()
                            .all(|t| is_vowel(t.base_char.to_ascii_lowercase()))
                            || (rule.pattern == "oo"
                                && self.composition[idx + 1..].iter().any(|t| {
                                    matches!(t.base_char.to_ascii_lowercase(), 'a' | 'e')
                                })))
                    {
                        continue;
                    }
                }
                if rule.pattern.ends_with('w')
                    && offset > 0
                    && self.composition[idx].mark_applied.is_some()
                {
                    continue;
                }
                return Some((idx, rule.pattern.clone(), rule.result.clone()));
            }

            if is_vowel(ch) {
                // Allow a doubled-vowel mark trigger to cross one adjacent vowel
                // in the same nucleus, e.g. \"neue\" -> \"nêu\". Do not cross a
                // vowel reached after consonants, which would turn English words
                // such as \"readme\" into \"rêadme\".
                if offset == 0 {
                    continue;
                }
                break;
            }
        }
        None
    }

    fn apply_mark_at(&mut self, idx: usize, _pattern: &str, result: &str) {
        let result_chars: Vec<char> = result.chars().collect();
        let was_upper = self.composition[idx].is_upper;
        let old_tone = self.composition[idx].tone_applied;

        // Replace the char at idx with result chars
        self.composition.remove(idx);
        for (i, &ch) in result_chars.iter().enumerate() {
            self.composition.insert(
                idx + i,
                Transformation {
                    base_char: ch,
                    mark_applied: Some(ch),
                    tone_applied: old_tone,
                    is_upper: was_upper && i == 0,
                },
            );
        }
    }

    pub fn get_output(&self) -> String {
        self.flatten()
    }

    pub fn undo_telex_transform(&mut self, trigger: char, raw: &str) -> Option<TelexUndo> {
        if self.undone_in_word {
            return None;
        }

        let trigger = trigger.to_ascii_lowercase();
        if raw.chars().last().map(|last| last.to_ascii_lowercase()) == Some(trigger) {
            if let Some(&(tone_char, _)) = self.rules.tone_keys.get(&trigger) {
                for transformation in self.composition.iter_mut().rev() {
                    if transformation.tone_applied == Some(tone_char) {
                        transformation.tone_applied = None;
                        self.undone_in_word = true;
                        return Some(TelexUndo::AppendTrigger);
                    }
                }
            }
        }

        let raw = raw.to_ascii_lowercase();
        for rule in &self.rules.mark_rules {
            let undo_behavior = rule.undo_behavior.unwrap_or(UndoBehavior::AppendTrigger);
            if !rule.pattern.ends_with(trigger) || !has_active_mark_pattern(&raw, &rule.pattern) {
                continue;
            }

            let Some(base_char) = rule.pattern.chars().next() else {
                continue;
            };
            let Some(result) = rule.result.chars().next() else {
                continue;
            };

            for idx in (0..self.composition.len()).rev() {
                if self.composition[idx]
                    .mark_applied
                    .map(|ch| ch.to_ascii_lowercase())
                    == Some(result)
                    && self.composition[idx].tone_applied.is_none()
                {
                    self.composition[idx].base_char = if self.composition[idx].is_upper {
                        base_char.to_ascii_uppercase()
                    } else {
                        base_char
                    };
                    self.composition[idx].mark_applied = None;
                    if result == 'ơ'
                        && idx > 0
                        && self.composition[idx - 1].mark_applied == Some('ư')
                    {
                        self.composition[idx - 1].base_char = if self.composition[idx - 1].is_upper
                        {
                            'U'
                        } else {
                            'u'
                        };
                        self.composition[idx - 1].mark_applied = None;
                    }
                    self.undone_in_word = true;
                    return Some(match undo_behavior {
                        UndoBehavior::AppendTrigger => TelexUndo::AppendTrigger,
                        UndoBehavior::ConsumeTrigger => TelexUndo::ConsumeTrigger,
                    });
                }
            }
        }

        None
    }

    pub fn append_literal(&mut self, ch: char) {
        self.append_char(ch);
    }

    fn append_char(&mut self, ch: char) {
        self.composition.push(Transformation {
            base_char: ch,
            mark_applied: None,
            tone_applied: None,
            is_upper: ch.is_uppercase(),
        });
    }

    fn apply_tone(&mut self, tone_char: char) -> Option<String> {
        if self.composition.is_empty() {
            return Some(tone_char.to_string());
        }

        // Find the last syllable
        let last_syllable = self.last_syllable_range();
        let tone_pos = self.find_tone_position(last_syllable.clone());

        for idx in last_syllable {
            if let Some(t) = self.composition.get_mut(idx) {
                t.tone_applied = None;
            }
        }

        if let Some(t) = self.composition.get_mut(tone_pos) {
            t.tone_applied = Some(tone_char);
            return Some(self.flatten());
        }

        Some(self.flatten())
    }

    fn last_syllable_range(&self) -> std::ops::Range<usize> {
        let mut start = 0usize;
        for (i, t) in self.composition.iter().enumerate().rev() {
            let ch = t.mark_applied.unwrap_or(t.base_char);
            if ch.is_whitespace()
                || ch == '.'
                || ch == ','
                || ch == '!'
                || ch == '?'
                || ch == ';'
                || ch == ':'
            {
                start = i + 1;
                break;
            }
        }
        start..self.composition.len()
    }

    fn find_tone_position(&self, range: std::ops::Range<usize>) -> usize {
        let start = range.start;
        let end = range.end;
        let mut vowels: Vec<usize> = Vec::new();

        for i in range {
            let ch = self.composition[i]
                .mark_applied
                .unwrap_or(self.composition[i].base_char);
            if is_vowel(ch) {
                vowels.push(i);
            }
        }

        if vowels.is_empty() {
            return self.composition.len().saturating_sub(1);
        }

        // Exclude onset glides: in "qu…" the u and in "gi…" the i belong to the
        // initial consonant, not the vowel nucleus — so they must never carry the
        // tone (e.g. "quả" not "qủa", "giờ" not "gìơ"). Only strip the glide when
        // another vowel follows it; bare "gì"/"qu" keep the letter as the nucleus.
        if vowels.len() >= 2 && vowels[0] == start + 1 {
            let onset = self.composition[start].base_char.to_ascii_lowercase();
            let glide = self.composition[start + 1].base_char.to_ascii_lowercase();
            if (onset == 'q' && glide == 'u') || (onset == 'g' && glide == 'i') {
                vowels.remove(0);
            }
        }

        if vowels.len() == 1 {
            return vowels[0];
        }

        // General phonological rule: if any vowel in the nucleus carries a
        // diacritic mark (ă, â, ê, ô, ơ, ư), the tone attaches to the last
        // marked vowel (covering iê, yê, uô, ươ, oă, uâ, uê, uơ, ưa, ưi, ưu,
        // ươi, ươu, uôi, iêu, yêu, uây, etc.).
        if let Some(&marked_idx) = vowels.iter().rev().find(|&&idx| {
            let ch = self.composition[idx]
                .mark_applied
                .unwrap_or(self.composition[idx].base_char)
                .to_ascii_lowercase();
            matches!(ch, 'ă' | 'â' | 'ê' | 'ô' | 'ơ' | 'ư')
        }) {
            return marked_idx;
        }

        // Three+ unmarked vowels (oai, oay, oeo, uya, uyu): tone on middle vowel.
        if vowels.len() >= 3 {
            return vowels[1];
        }

        // Two unmarked vowels: tone on the second vowel for medial-glide clusters
        // (oa, oe, uy, io) or closed syllables with ie/ye/uo.
        let cv1 = self.composition[vowels[0]]
            .mark_applied
            .unwrap_or(self.composition[vowels[0]].base_char)
            .to_ascii_lowercase();
        let cv2 = self.composition[vowels[1]]
            .mark_applied
            .unwrap_or(self.composition[vowels[1]].base_char)
            .to_ascii_lowercase();
        let has_final_consonant = vowels[1] + 1 < end;

        // Old-Style tone placement ("Kiểu cũ"):
        // Open unmarked oa/oe/uy place tone on first vowel (`hòa`, `khỏe`, `thủy`);
        // closed syllables with final consonant place tone on second vowel (`hoàn`, `toán`, `huỳnh`).
        if (has_final_consonant
            && matches!(
                (cv1, cv2),
                ('o', 'a')
                    | ('o', 'e')
                    | ('o', 'o')
                    | ('u', 'y')
                    | ('i', 'e')
                    | ('y', 'e')
                    | ('u', 'o')
            ))
            || (cv1, cv2) == ('i', 'o')
        {
            return vowels[1];
        }

        vowels[0]
    }

    fn flatten(&self) -> String {
        let mut output = String::new();

        for t in &self.composition {
            let base = t.mark_applied.unwrap_or(t.base_char);
            let mut ch = if let Some(tone) = t.tone_applied {
                apply_tone_to_char(base, tone)
            } else {
                base
            };

            if t.is_upper && !ch.is_uppercase() {
                ch = ch.to_uppercase().next().unwrap_or(ch);
            }

            output.push(ch);
        }

        output
    }
}

fn has_active_mark_pattern(raw: &str, pattern: &str) -> bool {
    let Some(pattern_start) = raw.rfind(pattern) else {
        return false;
    };
    let suffix = &raw[pattern_start + pattern.len()..];
    if pattern.ends_with('w') {
        return suffix.is_empty();
    }
    !suffix
        .chars()
        .any(|ch| matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y'))
}

fn is_vowel(ch: char) -> bool {
    matches!(
        ch.to_ascii_lowercase(),
        'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'ă' | 'â' | 'ê' | 'ô' | 'ơ' | 'ư'
    )
}

fn apply_tone_to_char(ch: char, tone: char) -> char {
    const TABLE: [&str; 12] = [
        "aáàảãạ",
        "ăắằẳẵặ",
        "âấầẩẫậ",
        "eéèẻẽẹ",
        "êếềểễệ",
        "iíìỉĩị",
        "oóòỏõọ",
        "ôốồổỗộ",
        "ơớờởỡợ",
        "uúùủũụ",
        "ưứừửữự",
        "yýỳỷỹỵ",
    ];
    let offset = match tone {
        's' | '1' => 1,
        'f' | '2' => 2,
        'r' | '3' => 3,
        'x' | '4' => 4,
        'j' | '5' => 5,
        _ => return ch,
    };
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    TABLE
        .iter()
        .find_map(|row| {
            let mut chars = row.chars();
            (chars.next() == Some(lower)).then(|| row.chars().nth(offset).unwrap_or(ch))
        })
        .unwrap_or(ch)
}

#[cfg(test)]
#[path = "bamboo_tests.rs"]
mod tests;
