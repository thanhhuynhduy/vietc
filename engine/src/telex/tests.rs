use super::{parse, Boundary, Glide, ParseStatus, ParsedSyllable, RawSyllable, Tone, VowelMark};

#[test]
fn raw_syllable_construction_preserves_keys_and_independent_state() {
    let raw = RawSyllable::new("duoc")
        .with_tone(Tone::Acute)
        .with_mark(VowelMark::Horn);

    assert_eq!(raw.keys(), "duoc");
    assert_eq!(raw.tone(), Tone::Acute);
    assert_eq!(raw.mark(), VowelMark::Horn);
}

#[test]
fn raw_syllable_stops_at_boundaries() {
    for input in ["xin chao", "xin,chao", "xin.chao", "xin-chao"] {
        let parsed = parse(input);
        assert_eq!(parsed.raw().keys(), "xin");
        assert_ne!(parsed.boundary(), Boundary::EndOfInput);
    }
}

#[test]
fn parses_onset_nucleus_and_coda() {
    let cases = [
        ("an", "", "a", "n"),
        ("ban", "b", "a", "n"),
        ("nghieng", "ngh", "ie", "ng"),
        ("thuyen", "th", "uye", "n"),
        ("oanh", "", "oa", "nh"),
    ];

    for (input, onset, nucleus, coda) in cases {
        let parsed = parse(input);
        assert_eq!(
            (parsed.onset(), parsed.nucleus(), parsed.coda()),
            (onset, nucleus, coda),
            "input={input}"
        );
    }
}

#[test]
fn qu_and_gi_are_conditional_glides() {
    let cases = [
        ("qua", "qu", "a", Glide::Qu),
        ("quyen", "qu", "ye", Glide::Qu),
        ("gia", "gi", "a", Glide::Gi),
        ("gieng", "gi", "e", Glide::Gi),
    ];

    for (input, onset, nucleus, glide) in cases {
        let parsed = parse(input);
        assert_eq!(parsed.onset(), onset, "input={input}");
        assert_eq!(parsed.nucleus(), nucleus, "input={input}");
        assert_eq!(parsed.glide(), Some(glide), "input={input}");
    }

    assert_eq!(parse("qu").glide(), None);
    assert_eq!(parse("gi").glide(), None);
    assert_eq!(parse("quit").glide(), Some(Glide::Qu));
    assert_eq!(parse("gin").glide(), None);
}

#[test]
fn reports_incomplete_and_invalid_inputs_without_resolving_telex_triggers() {
    assert_eq!(parse("").status(), ParseStatus::Incomplete);
    assert_eq!(parse("ngh").status(), ParseStatus::Incomplete);
    assert_eq!(parse("b").status(), ParseStatus::Incomplete);
    assert_eq!(parse("bca").status(), ParseStatus::Invalid);

    let unresolved = parse("dduojwc");
    assert_eq!(unresolved.raw().keys(), "dduojwc");
    assert_eq!(unresolved.tone(), Tone::None);
    assert_eq!(unresolved.mark(), VowelMark::None);
}

#[test]
fn parsed_syllable_construction_enforces_contiguous_parts() {
    let parsed = ParsedSyllable::from_parts(
        RawSyllable::new("ban"),
        "b",
        "a",
        "n",
        None,
        Boundary::EndOfInput,
        ParseStatus::Complete,
    )
    .expect("contiguous parts");

    assert_eq!(parsed.raw().keys(), "ban");
    assert!(ParsedSyllable::from_parts(
        RawSyllable::new("ban"),
        "b",
        "e",
        "n",
        None,
        Boundary::EndOfInput,
        ParseStatus::Complete,
    )
    .is_err());
}

#[test]
fn bounded_generated_inputs_never_panic() {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz ,.-";
    let mut state = 0x9e37_79b9_u32;

    for length in 0..=24 {
        for _ in 0..256 {
            let input: String = (0..length)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ALPHABET[(state as usize) % ALPHABET.len()] as char
                })
                .collect();
            let _ = parse(&input);
        }
    }
}

#[test]
fn phase3_mark_matrix_resolves_without_word_hardcoding() {
    let cases = [
        ("aa", "â"),
        ("aw", "ă"),
        ("ee", "ê"),
        ("oo", "ô"),
        ("ow", "ơ"),
        ("uw", "ư"),
        ("uow", "ươ"),
        ("uaw", "ưa"),
        ("dd", "đ"),
    ];

    for (input, expected) in cases {
        assert_eq!(super::resolve_and_render(input), expected, "input={input}");
    }
}

#[test]
fn phase3_tone_matrix_and_free_order_converge() {
    let cases = [
        ("as", "á"),
        ("af", "à"),
        ("ar", "ả"),
        ("ax", "ã"),
        ("aj", "ạ"),
        ("tieengs", "tiếng"),
        ("dduowngf", "đường"),
        ("dduojwc", "được"),
        ("dduocwj", "được"),
        ("neues", "nếu"),
        ("neesu", "nếu"),
    ];

    for (input, expected) in cases {
        assert_eq!(super::resolve_and_render(input), expected, "input={input}");
    }
}

#[test]
fn phase3_glides_casing_and_fallback_are_deterministic() {
    let cases = [
        ("quas", "quá"),
        ("gias", "giá"),
        ("TIEENGS", "TIẾNG"),
        ("readme", "readme"),
        ("type", "type"),
        ("ngh", "ngh"),
        ("bca", "bca"),
    ];

    for (input, expected) in cases {
        let first = super::resolve_and_render(input);
        assert_eq!(first, expected, "input={input}");
        assert_eq!(super::resolve_and_render(input), first, "input={input}");
    }
}

#[test]
fn phase3_replacement_diff_counts_unicode_scalars_not_utf8_bytes() {
    let diff = super::replacement_diff("được", "đường");
    assert_eq!(diff.common_prefix_chars, 2);
    assert_eq!(diff.delete_before_cursor, 2);
    assert_eq!(diff.insert, "ờng");
}

#[test]
fn bounded_generated_inputs_resolve_and_render_without_panicking() {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ ,.-";
    let mut state = 0x517c_c1b7_u32;

    for length in 0..=24 {
        for _ in 0..256 {
            let input: String = (0..length)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ALPHABET[(state as usize) % ALPHABET.len()] as char
                })
                .collect();
            let rendered = super::resolve_and_render(&input);
            let _ = super::replacement_diff(&rendered, &input);
        }
    }
}
