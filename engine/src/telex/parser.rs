use super::model::{Boundary, Glide, ParseStatus, ParsedSyllable};
use super::raw::RawSyllable;

fn is_vowel(ch: char) -> bool {
    matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

fn is_valid_onset(onset: &str) -> bool {
    matches!(
        onset,
        "" | "b"
            | "c"
            | "ch"
            | "d"
            | "dd"
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

fn is_valid_coda(coda: &str) -> bool {
    matches!(coda, "" | "c" | "ch" | "m" | "n" | "ng" | "nh" | "p" | "t")
}

fn boundary(input: &str) -> (&str, Boundary) {
    match input
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_alphabetic())
    {
        Some((index, ch)) => (
            &input[..index],
            if ch.is_whitespace() {
                Boundary::Whitespace
            } else {
                Boundary::Punctuation
            },
        ),
        None => (input, Boundary::EndOfInput),
    }
}

pub fn parse(input: &str) -> ParsedSyllable {
    let (keys, boundary) = boundary(input);
    let raw = RawSyllable::new(keys);
    let chars: Vec<char> = keys.chars().collect();

    let first_vowel = chars.iter().position(|ch| is_vowel(*ch));
    let mut nucleus_start = first_vowel.unwrap_or(chars.len());
    let mut glide = None;

    if chars.len() > 2 && chars[0] == 'q' && chars[1] == 'u' && is_vowel(chars[2]) {
        nucleus_start = 2;
        glide = Some(Glide::Qu);
    } else if chars.len() > 2 && chars[0] == 'g' && chars[1] == 'i' && is_vowel(chars[2]) {
        nucleus_start = 2;
        glide = Some(Glide::Gi);
    }

    let mut nucleus_end = nucleus_start;
    while nucleus_end < chars.len() && is_vowel(chars[nucleus_end]) {
        nucleus_end += 1;
    }

    let onset: String = chars[..nucleus_start].iter().collect();
    let nucleus: String = chars[nucleus_start..nucleus_end].iter().collect();
    let coda: String = chars[nucleus_end..].iter().collect();

    let status = if keys.is_empty() || nucleus.is_empty() {
        ParseStatus::Incomplete
    } else if coda.chars().any(is_vowel)
        || onset.chars().any(is_vowel)
        || !is_valid_onset(&onset)
        || !is_valid_coda(&coda)
    {
        ParseStatus::Invalid
    } else {
        ParseStatus::Complete
    };

    ParsedSyllable::from_parts(raw, onset, nucleus, coda, glide, boundary, status)
        .expect("parser partitions the raw sequence contiguously")
}
