// SPDX-License-Identifier: MIT
//! Gboard-style Bilingual (Vietnamese + English) Lexicon & Context Scorer.
//!
//! Models Gboard Android's dual-hypothesis transliteration decoder:
//! 1. Unigram frequency tiers distinguishing English-dominant collisions (`test`,
//!    `this`, `rust`, `how`, `now`, `most`, `turn`, `own`, `list`, `host`...)
//!    from Vietnamese-dominant collisions (`car`=`cả`, `been`=`bên`, `too`=`tô`,
//!    `gif`=`gì`, `teen`=`tên`, `ddos`=`đó`, `its`=`ít`, `sown`=`sơn`).
//! 2. Bigram language momentum (`LanguageContext`) from the preceding committed
//!    word in the current sentence so phrases like `"bánh tét"` vs `"cargo test"`,
//!    `"cảm ơn"` vs `"my own"`, `"hai bên"` vs `"have been"`, `"bác sĩ"` vs
//!    `"six pack"`, `"cá trê"` vs `"binary tree"` resolve seamlessly.

use std::collections::HashSet;

const COMMON_ENGLISH_WORDS_RAW: &str = include_str!("english_words.txt");
const EXTENDED_ENGLISH_WORDS_RAW: &str = include_str!("english_extended.txt");

/// Words that are overwhelmingly English even inside Vietnamese sentences
/// (`"team mình lets meet"`, `"làm ops"`, `"nuôi pets"`, `"chặn bots"`).
/// Users who genuinely want the rare Vietnamese bound syllable (e.g. `"lét"` in
/// `"lét đét"`) can press Backspace once after Space (Gboard undo) to keep it.
const UNCONDITIONAL_ENGLISH_COLLISIONS: &[&str] = &[
    "of", "lets", "ops", "bots", "dots", "pets", "kits", "puts", "pats", "pits", "cost",
];

const ENGLISH_DOMINANT_COLLISIONS: &[&str] = &[
    "test", "rust", "this", "has", "his", "how", "now", "low", "see", "tree", "most", "host",
    "cost", "lost", "post", "list", "last", "past", "cast", "must", "dust", "trust", "rest",
    "best", "nest", "vest", "turn", "burn", "born", "corn", "horn", "form", "term", "arm", "bar",
    "or", "is", "us", "os", "if", "bus", "gas", "six", "mix", "box", "tax", "max", "own", "down",
    "town", "room", "noon", "soon", "seen", "lawn", "dawn", "more", "core", "lose", "rose", "nose",
    "chose", "those", "these", "here", "there", "where", "were", "care", "share", "base", "case",
    "use", "pure", "cure", "hire", "fire", "user", "laser", "server", "order", "border", "sister",
    "todo", "logo", "solo", "photo", "auto", "demo", "repo", "sudo", "cargo", "macro", "docs",
    "bits", "add", "odd", "odds", "beef", "beer", "boost", "deer", "door", "keen", "leer", "loom",
    "moor", "queen", "reef", "roof", "seer", "tee", "var", "usr", "res", "ref", "pos", "eof",
];

const VIETNAMESE_DOMINANT_COLLISIONS: &[&str] = &[
    "as",   // á ("á", "á?", "á!", "á à" vs "as soon", "as a", "run as root")
    "car",  // cả ("tất cả", "cả nhà" vs "my car")
    "been", // bên ("bên cạnh", "hai bên" vs "have been")
    "too",  // tô ("tô phở", "ô tô" vs "me too")
    "gif",  // gì ("cái gì", "làm gì" vs "animated gif")
    "teen", // tên ("họ tên", "tên gì" vs "my teen")
    "moon", // môn ("bộ môn", "môn học" vs "full moon")
    "ddos", // đó ("cái đó", "ở đó" vs "mitigate ddos" / "DDoS")
    "its",  // ít ("ít nhất", "một ít" vs "in its")
    "sown", // sơn ("nước sơn" vs "reap sown")
    "tar",  // tả ("mô tả" vs "run tar")
    "sets", // sét ("sấm sét" vs "data sets")
    "lots", // lót ("miếng lót" vs "lots of")
    "maps", // máp ("má" / "máp" vs "google maps")
    "caps", // cáp ("dây cáp" vs "all caps")
    "tips", // típ ("mắt típ" vs "quick tips")
    "tops", // tóp ("tóp mỡ" vs "tank tops")
    "cuts", // cút ("chim cút" vs "budget cuts")
    "hits", // hít ("hít thở" vs "top hits")
    "mats", // mát ("mát mẻ" vs "floor mats")
    "hats", // hát ("ca hát" vs "wear hats")
    "bats", // bát ("cái bát" vs "baseball bats")
    "cats", // cát ("bãi cát" vs "two cats")
    "rats", // rát ("bỏng rát" vs "lab rats")
    "nuts", // nút ("nút bấm" vs "pine nuts")
    "nets", // nét ("rõ nét" vs "safety nets")
    "cups", // cúp ("cúp điện" vs "two cups")
    "bow",  // bơ ("quả bơ" vs "take a bow")
    "cow",  // cơ ("cơ hội" vs "milk cow")
    "row",  // rơ ("rơ le" vs "next row")
    "tow",  // tơ ("tơ lụa" vs "tow truck")
    "mow",  // mơ ("ước mơ" vs "mow lawn")
    "sow",  // sơ ("hồ sơ" vs "reap sow")
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageContext {
    None,
    Vietnamese,
    English,
}

pub struct EnglishDict {
    words: HashSet<&'static str>,
    extended: Vec<&'static str>,
    unconditional_english_collisions: HashSet<&'static str>,
    english_dominant_collisions: HashSet<&'static str>,
    vietnamese_dominant_collisions: HashSet<&'static str>,
}

impl EnglishDict {
    pub fn new() -> Self {
        let words = COMMON_ENGLISH_WORDS_RAW
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        let extended = EXTENDED_ENGLISH_WORDS_RAW
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        let unconditional_english_collisions =
            UNCONDITIONAL_ENGLISH_COLLISIONS.iter().copied().collect();
        let english_dominant_collisions = ENGLISH_DOMINANT_COLLISIONS.iter().copied().collect();
        let vietnamese_dominant_collisions =
            VIETNAMESE_DOMINANT_COLLISIONS.iter().copied().collect();
        Self {
            words,
            extended,
            unconditional_english_collisions,
            english_dominant_collisions,
            vietnamese_dominant_collisions,
        }
    }

    pub fn is_core_english_word(&self, word: &str) -> bool {
        self.words.contains(word)
    }

    pub fn is_english_word(&self, word: &str) -> bool {
        self.words.contains(word) || self.extended.binary_search(&word).is_ok()
    }

    pub fn is_unconditional_english_collision(&self, raw_lower: &str) -> bool {
        self.unconditional_english_collisions.contains(raw_lower)
    }

    pub fn is_english_dominant_collision(&self, raw_lower: &str) -> bool {
        self.english_dominant_collisions.contains(raw_lower)
    }

    pub fn is_vietnamese_dominant_collision(&self, raw_lower: &str) -> bool {
        self.vietnamese_dominant_collisions.contains(raw_lower)
    }

    /// Classify the language of a committed word to provide Gboard-style Bigram
    /// momentum for the next word in the sentence while keeping unmarked bilingual
    /// homographs (`con`, `in`, `me`, `may`, `to`, `a`, `an`, `do`, `ve`, `re`...)
    /// context-preserving (`prior_context`) instead of flipping language momentum.
    pub fn classify_committed_word_with_prior(
        &self,
        final_word: &str,
        raw: &str,
        prior_context: LanguageContext,
    ) -> LanguageContext {
        if final_word.is_empty() {
            return LanguageContext::None;
        }
        if !final_word.is_ascii() {
            return LanguageContext::Vietnamese;
        }
        // Standalone capitalized English pronoun `"I"` (`"I see"`, `"I own"`, `"I've"`)
        if final_word == "I" && raw == "I" {
            return LanguageContext::English;
        }
        let lower = final_word.to_ascii_lowercase();
        let raw_lower = raw.to_ascii_lowercase();

        // English contraction suffixes after `'` (`I've`, `we're`, `you'll`, `don't`, `it's`, `I'd`, `I'm`)
        if matches!(lower.as_str(), "ve" | "re" | "ll" | "t" | "s" | "d" | "m")
            && prior_context == LanguageContext::English
        {
            return LanguageContext::English;
        }

        let in_vn = crate::spelling::is_valid_vietnamese_syllable(&lower)
            && crate::spelling::is_in_vietnamese_dictionary(&lower);
        let in_en = self.is_english_word(&lower) || self.is_english_word(&raw_lower);

        // Unmarked ASCII homographs valid in BOTH Vietnamese and English (`con`, `in`, `me`,
        // `may`, `son`, `can`, `say`, `to`, `do`, `a`, `an`, `long`, `ban`...) are neutral
        // carriers: preserve the active sentence context rather than hijacking it!
        if in_vn && in_en {
            return prior_context;
        }
        if in_vn && !in_en {
            return LanguageContext::Vietnamese;
        }
        if in_en
            || lower.chars().any(|c| matches!(c, 'f' | 'j' | 'w' | 'z'))
            || !crate::spelling::is_valid_vietnamese_syllable(&lower)
        {
            return LanguageContext::English;
        }
        prior_context
    }
}

/// Detect alphanumeric technical identifiers in VNI/Telex (e.g. `u32`, `i64`, `f32`,
/// `mp4`, `h264`, `x86`, `sha256`, `ec2`, `s3`, `v2`) that should restore to raw ASCII.
pub fn is_alphanumeric_tech_token(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "f32"
            | "f64"
            | "v1"
            | "v2"
            | "v3"
            | "v4"
            | "s3"
            | "ec2"
            | "mp3"
            | "mp4"
            | "h264"
            | "h265"
            | "x86"
            | "x64"
            | "arm64"
            | "utf8"
            | "utf16"
            | "sha1"
            | "sha256"
            | "sha512"
            | "base64"
            | "md5"
            | "ipv4"
            | "ipv6"
            | "http2"
            | "http3"
            | "web3"
    ) {
        return true;
    }
    // Uppercase 2-char standard codes (`A3`, `A4`, `A5`, `I3`, `I5`, `I7`, `I9`,
    // `O2`, `O3`, `U2`) must not be mangled into Vietnamese diacritics in VNI,
    // while lowercase VNI typing (`a1` -> `á`, `o2` -> `ò`) remains untouched.
    if raw.len() == 2
        && raw.as_bytes()[0].is_ascii_uppercase()
        && raw.as_bytes()[1].is_ascii_digit()
        && matches!(
            raw,
            "A3" | "A4" | "A5" | "E1" | "E2" | "I3" | "I5" | "I7" | "I9" | "O2" | "O3" | "U2"
        )
    {
        return true;
    }
    let chars: Vec<char> = lower.chars().collect();
    chars.len() >= 3
        && chars[0].is_ascii_alphabetic()
        && chars[1..].iter().all(|c| c.is_ascii_digit())
        && chars[1] != chars[2]
}

/// Detect mixed internal uppercase tech casing (`"DDoS"`, `"GraphQL"`, `"PyTorch"`, `"iPhone"`, `"macOS"`)
/// while preserving inverted-CapsLock Vietnamese (`"cHÀO"`, `"vIỆT"`) and early-Shift trailing
/// Telex modifiers (`"chaoF"` -> `"chào"`, `"VieetJ"` -> `"Việt"`, `"vieeTj"` -> `"việt"`).
pub fn has_mixed_internal_caps(raw: &str, composed: &str) -> bool {
    let alpha: Vec<char> = raw.chars().filter(|c| c.is_alphabetic()).collect();
    if alpha.len() < 3 {
        return false;
    }
    // Inverted CapsLock (`cHAOF` -> `cHÀO`, `vIEETJ` -> `vIỆT`): first letter lowercase, all rest uppercase.
    if alpha[0].is_lowercase() && alpha[1..].iter().all(|c| c.is_uppercase()) {
        return false;
    }
    // Early-Shift on trailing Telex modifier (`chaoF` -> `chào`, `VieetJ` -> `Việt`):
    // interior letters (`1..len-1`) are lowercase and the composed word has no internal uppercase!
    if alpha[1..alpha.len() - 1].iter().all(|c| c.is_lowercase())
        && alpha[alpha.len() - 1].is_uppercase()
        && !composed.chars().skip(1).any(|c| c.is_uppercase())
    {
        return false;
    }
    // Single accidental early-Shift on a vowel/coda letter before the trailing tone/mark key
    // inside a longer (>= 5 alpha) valid non-ASCII Vietnamese dictionary syllable
    // (`vieeTj` -> `việt`, `nguowIf` -> `người`, `dduowNgf` -> `đường`), while preserving
    // short 3-4 char camelCase tokens (`dOs`, `cAs`, `tOs`, `iOs`, `dDos`).
    if !composed.is_ascii()
        && !raw.eq_ignore_ascii_case("ddos")
        && alpha.len() >= 5
        && alpha[1..].iter().filter(|c| c.is_uppercase()).count() == 1
        && crate::spelling::is_valid_vietnamese_syllable(composed)
        && crate::spelling::is_in_vietnamese_dictionary(composed)
    {
        return false;
    }
    alpha
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase())
}

/// Detect natural English double-letter stems (`cheese`, `geese`, `business`, `address`, `stuff`, `IEEE`, `wedding`)
/// so a Telex double-key undo restores the eaten letter rather than truncating the word.
pub fn has_natural_english_double(raw_lower: &str) -> bool {
    if matches!(
        raw_lower,
        "ieee"
            | "geese"
            | "cheese"
            | "breeze"
            | "freeze"
            | "sneeze"
            | "squeeze"
            | "sleeve"
            | "groove"
            | "choose"
            | "loose"
            | "goose"
            | "moose"
            | "noose"
            | "soothe"
            | "smooth"
    ) {
        return true;
    }
    let chars: Vec<char> = raw_lower.chars().collect();
    if chars.windows(3).any(|w| w[0] == w[1] && w[1] == w[2]) {
        return false;
    }
    if raw_lower.ends_with("less")
        || raw_lower.ends_with("ness")
        || raw_lower.ends_with("press")
        || raw_lower.ends_with("gress")
        || raw_lower.ends_with("cess")
        || raw_lower.ends_with("pass")
        || raw_lower.ends_with("class")
        || raw_lower.ends_with("glass")
        || raw_lower.ends_with("cross")
        || raw_lower.ends_with("dress")
        || raw_lower.ends_with("stress")
        || raw_lower.ends_with("off")
        || matches!(
            raw_lower,
            "ass"
                | "bass"
                | "boss"
                | "brass"
                | "chess"
                | "css"
                | "discuss"
                | "floss"
                | "gloss"
                | "grass"
                | "gross"
                | "guess"
                | "harass"
                | "kiss"
                | "lass"
                | "loss"
                | "mass"
                | "mess"
                | "miss"
                | "moss"
                | "possess"
                | "rss"
                | "swiss"
                | "toss"
                | "truss"
                | "unless"
                | "stuff"
                | "staff"
                | "stiff"
                | "cliff"
                | "bluff"
                | "sniff"
                | "buff"
                | "cuff"
                | "puff"
                | "diff"
                | "tariff"
                | "sheriff"
                | "add"
                | "odd"
                | "odds"
                | "err"
                | "arr"
        )
    {
        return true;
    }
    if raw_lower.starts_with("passw")
        || raw_lower.starts_with("class")
        || raw_lower.starts_with("cross")
        || raw_lower.starts_with("addr")
        || raw_lower.starts_with("busin")
        || raw_lower.starts_with("add")
        || raw_lower.starts_with("odd")
    {
        return true;
    }
    for i in 0..chars.len().saturating_sub(2) {
        let c0 = chars[i];
        let c1 = chars[i + 1];
        let c2 = chars[i + 2];
        if c0 == c1
            && matches!(c0, 's' | 'f' | 'r')
            && matches!(c2, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
        {
            return true;
        }
        if i > 0
            && c0 == 'd'
            && c1 == 'd'
            && matches!(chars[i - 1], 'a' | 'e' | 'i' | 'o' | 'u')
            && matches!(c2, 'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'l' | 'r')
        {
            return true;
        }
    }
    false
}

/// Detect standard ALLCAPS office/tech acronyms (`AWS`, `DDOS`, `ECS`, `ORM`, `GIF`, `TAR`,
/// `RAR`, `EOF`, `COO`, `POS`, `EST`, `TOS`, `SOW`, `ATS`, `IR`, `AR`, `OOO`, `ERR`, `ARR`)
/// whose raw letters collide with short Vietnamese Telex syllables (`Ắ`, `ĐÓ`, `ÉC`, `GÌ`, `CÔ`).
pub fn is_english_allcaps_acronym(raw: &str, raw_lower: &str) -> bool {
    raw.len() >= 2
        && raw.chars().all(|c| c.is_ascii_uppercase())
        && matches!(
            raw_lower,
            "aws"
                | "ecs"
                | "ddos"
                | "dos"
                | "orm"
                | "gif"
                | "tar"
                | "rar"
                | "eof"
                | "err"
                | "arr"
                | "coo"
                | "ir"
                | "ar"
                | "ats"
                | "sow"
                | "pos"
                | "est"
                | "ooo"
                | "tos"
        )
}

/// Detect English/tech tokens that triggered >= 3 Telex tone overwrites inside a single word
/// (e.g. `nextjs` consuming `x`, `j`, `s` to collapse into `nét`; `nuxtjs` into `nút`).
pub fn has_multi_tone_overwrite_anomaly(raw: &str, composed: &str) -> bool {
    if raw.len() < 5 || raw.len() < composed.chars().count() + 2 {
        return false;
    }
    let mut seen_vowel = false;
    let mut tone_triggers = 0usize;
    for ch in raw.chars() {
        let lo = ch.to_ascii_lowercase();
        if matches!(lo, 'a' | 'e' | 'i' | 'o' | 'u' | 'y') {
            seen_vowel = true;
        } else if seen_vowel && matches!(lo, 's' | 'f' | 'r' | 'x' | 'j') {
            tone_triggers += 1;
        }
    }
    tone_triggers >= 3
}

pub(crate) fn should_restore_word_decision(
    dict: &EnglishDict,
    composed: &str,
    raw: &str,
    prev_context: LanguageContext,
    prev_visible_word: Option<&str>,
    boundary_context: Option<(Option<char>, char)>,
) -> bool {
    if composed == raw {
        return false;
    }

    let raw_lower = raw.to_lowercase();

    if let Some((prev_b, next_b)) = boundary_context {
        if is_path_or_code_context(prev_b, next_b, raw, composed, dict) {
            return true;
        }
    }

    if has_mixed_internal_caps(raw, composed)
        || (is_alphanumeric_tech_token(raw)
            && !crate::engine_helpers::is_vn_d_abbreviation(composed, &raw_lower))
        || is_english_allcaps_acronym(raw, &raw_lower)
        || has_multi_tone_overwrite_anomaly(raw, composed)
    {
        return true;
    }

    if composed.is_ascii() {
        return dict.is_english_word(&raw_lower)
            || has_natural_english_double(&raw_lower)
            || crate::bigram::has_english_compound_morphology(&raw_lower);
    }

    let vn_candidate = composed.trim_start_matches(|c: char| c.is_ascii_digit());
    let valid_syllable =
        !vn_candidate.is_empty() && crate::spelling::is_valid_vietnamese_syllable(vn_candidate);
    let valid_vn = valid_syllable && crate::spelling::is_in_vietnamese_dictionary(vn_candidate);
    if !valid_vn {
        let is_english = dict.is_english_word(&raw_lower)
            || has_natural_english_double(&raw_lower)
            || crate::bigram::has_english_compound_morphology(&raw_lower);
        if is_english {
            return true;
        }

        if crate::engine_helpers::is_vn_d_abbreviation(vn_candidate, &raw_lower) {
            return false;
        }
        return !valid_syllable;
    }

    if dict.is_unconditional_english_collision(&raw_lower) {
        return true;
    }

    if let Some(bigram_decision) =
        crate::bigram::evaluate_forward_bigram(prev_visible_word, composed, raw, prev_context)
    {
        return bigram_decision;
    }

    if dict.is_english_dominant_collision(&raw_lower) {
        return prev_context != LanguageContext::Vietnamese;
    }

    if dict.is_vietnamese_dominant_collision(&raw_lower) {
        return prev_context == LanguageContext::English;
    }

    if prev_context == LanguageContext::English && dict.is_english_word(&raw_lower) {
        return true;
    }

    false
}

/// Common technical token, directory/file name, or extension abbreviations.
pub fn is_tech_token(raw_lower: &str) -> bool {
    matches!(
        raw_lower,
        "conf"
            | "inf"
            | "das"
            | "xml"
            | "bin"
            | "src"
            | "doc"
            | "docs"
            | "lib"
            | "var"
            | "etc"
            | "opt"
            | "usr"
            | "dev"
            | "def"
            | "buf"
            | "cur"
            | "ver"
            | "dir"
            | "sys"
            | "proc"
            | "tmp"
            | "app"
            | "pkg"
            | "cmd"
            | "cli"
            | "sdk"
            | "api"
            | "sql"
            | "db"
            | "web"
            | "git"
            | "cfg"
            | "log"
            | "logs"
            | "dist"
            | "deps"
            | "yml"
            | "yaml"
            | "json"
            | "toml"
            | "properties"
    )
}

/// Checks if a word is typed in a path, URL, file-extension, or code identifier context
/// where raw ASCII must be restored instead of being corrupted into Vietnamese diacritics.
pub fn is_path_or_code_context(
    prev_boundary: Option<char>,
    next_boundary: char,
    raw: &str,
    composed: &str,
    dict: &EnglishDict,
) -> bool {
    let raw_lower = raw.to_ascii_lowercase();

    // Vietnamese abbreviations with 'Đ' / 'đ' (`QĐ`, `HĐ`, `NĐ`, `UBND`, `VNĐ`)
    // must NEVER be mangled back into 'DD' inside documents or legal codes,
    // while lowercase snake_case code identifiers (`config_ddi`, `btn_ddo`) restore safely.
    let is_snake_case_lower =
        (prev_boundary == Some('_') || next_boundary == '_') && !composed.contains('Đ');
    if (composed.contains('Đ') || composed.contains('đ'))
        && raw_lower != "ddos"
        && !is_snake_case_lower
    {
        return false;
    }

    let is_prev_path = matches!(prev_boundary, Some('/') | Some('\\'));
    let is_next_path = matches!(next_boundary, '/' | '\\');
    let is_prev_dot = prev_boundary == Some('.');
    let is_next_dot = next_boundary == '.';
    let is_prev_hyphen = matches!(prev_boundary, Some('-') | Some('_'));
    let is_next_hyphen = matches!(next_boundary, '-' | '_');

    // 1. Path/URL context: preceded or succeeded by slash (`/conf/`, `WEB-INF/`, `/src/`)
    if (is_prev_path || is_next_path)
        && (is_tech_token(&raw_lower)
            || dict.is_english_word(&raw_lower)
            || is_alphanumeric_tech_token(raw))
    {
        return true;
    }

    // 2. Package / Domain / File extension context (`it.das.`, `web.xml`, `app.conf`)
    if is_prev_dot {
        if is_tech_token(&raw_lower)
            || dict.is_english_word(&raw_lower)
            || is_alphanumeric_tech_token(raw)
        {
            return true;
        }
    } else if is_next_dot
        && (is_tech_token(&raw_lower)
            || is_alphanumeric_tech_token(raw)
            || is_prev_path
            || is_prev_hyphen)
    {
        return true;
    }

    // 3. Snake-case code identifier (`config_ddi`, `raw_buf`, `ioffice_web`)
    //    or kebab-case technical identifier (`WEB-INF`, `my-app`).
    if is_snake_case_lower {
        return true;
    }
    (is_prev_hyphen || is_next_hyphen)
        && (is_tech_token(&raw_lower)
            || dict.is_english_word(&raw_lower)
            || is_alphanumeric_tech_token(raw))
}

#[cfg(test)]
mod tests {
    use super::EnglishDict;

    #[test]
    fn extended_english_lexicon_is_sorted_and_collision_free_with_telex() {
        let dict = EnglishDict::new();
        assert!(
            dict.extended.len() > 30_000,
            "expected >30k extended English words, got {}",
            dict.extended.len()
        );
        for pair in dict.extended.windows(2) {
            assert!(
                pair[0] < pair[1],
                "english_extended.txt must be strictly sorted for binary_search: {:?} >= {:?}",
                pair[0],
                pair[1]
            );
        }
        for &word in &dict.extended {
            assert!(
                !crate::spelling::is_in_vietnamese_dictionary(word),
                "extended English word {:?} collides directly with Vietnamese dictionary",
                word
            );
            let composed = crate::telex::resolve_and_render(word);
            let collides_with_vn = composed != word
                && crate::spelling::is_valid_vietnamese_syllable(&composed)
                && crate::spelling::is_in_vietnamese_dictionary(&composed);
            assert!(
                !collides_with_vn,
                "extended English word {:?} collides with Vietnamese Telex syllable {:?}",
                word, composed
            );
        }
    }
}
