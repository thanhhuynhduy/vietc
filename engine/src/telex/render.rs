use super::mark;
use super::raw::{Tone, VowelMark};
use super::tone;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplacementDiff {
    pub common_prefix_chars: usize,
    pub delete_before_cursor: usize,
    pub insert: String,
}

fn shaped(base: char, mark: VowelMark) -> char {
    let upper = base.is_uppercase();
    let result = match (base.to_ascii_lowercase(), mark) {
        ('a', VowelMark::Breve) => 'ă',
        ('a', VowelMark::Circumflex) => 'â',
        ('e', VowelMark::Circumflex) => 'ê',
        ('o', VowelMark::Circumflex) => 'ô',
        ('o', VowelMark::Horn) => 'ơ',
        ('u', VowelMark::Horn) => 'ư',
        (ch, _) => ch,
    };
    if upper {
        result.to_uppercase().next().unwrap_or(result)
    } else {
        result
    }
}

fn toned(base: char, tone: Tone) -> char {
    const LOWER: [&str; 12] = [
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
    let upper = base.is_uppercase();
    let lower = base.to_lowercase().next().unwrap_or(base);
    let offset = match tone {
        Tone::None => 0,
        Tone::Acute => 1,
        Tone::Grave => 2,
        Tone::Hook => 3,
        Tone::Tilde => 4,
        Tone::Dot => 5,
    };
    let result = LOWER
        .iter()
        .find_map(|row| {
            let mut chars = row.chars();
            (chars.next() == Some(lower)).then(|| row.chars().nth(offset).unwrap_or(lower))
        })
        .unwrap_or(base);
    if upper {
        result.to_uppercase().next().unwrap_or(result)
    } else {
        result
    }
}

pub fn resolve_and_render(input: &str) -> String {
    let boundary = input
        .char_indices()
        .find(|(_, ch)| !ch.is_alphabetic())
        .map(|(i, _)| i)
        .unwrap_or(input.len());
    let (word, suffix) = input.split_at(boundary);
    let raw: Vec<char> = word.chars().collect();
    let marks = mark::resolve(&raw);
    let tones = tone::resolve(&raw, &marks);

    let mut output = String::new();
    for i in 0..raw.len() {
        if tones.consumed[i] {
            continue;
        }
        let mut ch = shaped(marks.letters[i], marks.marks[i]);
        if tones.target == Some(i) {
            ch = toned(ch, tones.tone);
        }
        output.push(ch);
    }
    output.push_str(suffix);
    output
}

pub fn replacement_diff(previous: &str, next: &str) -> ReplacementDiff {
    let common_prefix_chars = previous
        .chars()
        .zip(next.chars())
        .take_while(|(a, b)| a == b)
        .count();
    ReplacementDiff {
        common_prefix_chars,
        delete_before_cursor: previous.chars().count() - common_prefix_chars,
        insert: next.chars().skip(common_prefix_chars).collect(),
    }
}
