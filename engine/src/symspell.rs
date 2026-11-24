// SPDX-License-Identifier: MIT
//! Weighted Spatial SymSpell Autocorrect, Context Ranking & Compound Splitting (Task C2).
//!
//! Extends Dboard's conservative single-candidate autocorrect (`autocorrect.rs`)
//! with O(1) SymSpell delete-index lookup, layout-aware spatial edit costs,
//! Kneser-Ney n-gram contextual disambiguation (`ngram_lm` + `stat_lm`), and
//! high-precision Vietnamese missing-space splitting (`khoongbieets -> không biết`).
//!
//! Safety invariants:
//! - Zero `unsafe` code.
//! - Zero impact on mid-word typing fast path (runs only on Space).
//! - Strict zero-false-positive guards for code/dev identifiers, acronyms,
//!   valid VN/EN words, and personal dictionary entries.
//! - Every accepted correction is paired with One-Tap Undo (`Backspace`).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::autocorrect::{key_distance_for_layout, AutocorrectLayout};
use crate::english::LanguageContext;
use crate::input_method::InputMethod;

const MAX_SPATIAL_DIST: f32 = 1.25;
const CORE_ENGLISH_RAW: &str = include_str!("english_words.txt");
const EXTENDED_ENGLISH_RAW: &str = include_str!("english_extended.txt");

struct SymSpellIndex {
    vn_exact: HashMap<String, &'static str>,
    vn_deletes: HashMap<String, Vec<(&'static str, String)>>,
    vni_exact: HashMap<String, &'static str>,
    vni_deletes: HashMap<String, Vec<(&'static str, String)>>,
    en_core: HashSet<&'static str>,
    en_exact: HashSet<&'static str>,
    en_deletes: HashMap<String, Vec<&'static str>>,
}

fn symspell_index() -> &'static SymSpellIndex {
    static INDEX: OnceLock<SymSpellIndex> = OnceLock::new();
    INDEX.get_or_init(build_index)
}

/// Pre-builds the static SymSpell delete index so the first Space keystroke
/// never incurs a lazy-initialization latency spike on the typing thread.
pub fn warm_up() {
    let _ = symspell_index();
}

fn push_deletes_1<V: Clone>(map: &mut HashMap<String, Vec<V>>, key: &str, value: V) {
    let bytes = key.as_bytes();
    for i in 0..bytes.len() {
        let mut del = String::with_capacity(bytes.len() - 1);
        del.push_str(&key[..i]);
        del.push_str(&key[i + 1..]);
        map.entry(del).or_default().push(value.clone());
    }
}

fn build_index() -> SymSpellIndex {
    let mut vn_exact: HashMap<String, &'static str> = HashMap::with_capacity(7000);
    let mut vn_deletes: HashMap<String, Vec<(&'static str, String)>> =
        HashMap::with_capacity(28000);
    let mut vni_exact: HashMap<String, &'static str> = HashMap::with_capacity(7000);
    let mut vni_deletes: HashMap<String, Vec<(&'static str, String)>> =
        HashMap::with_capacity(28000);

    for line in crate::spelling::vietnamese_dict_raw().lines() {
        let syl = line.trim();
        if syl.is_empty() || syl.is_ascii() {
            continue;
        }
        if let Some(canon) = crate::ring_buffer::visible_to_canonical_raw(syl, InputMethod::Telex) {
            if canon.len() >= 3
                && canon.len() <= 10
                && canon.is_ascii()
                && vn_exact.insert(canon.clone(), syl).is_none()
            {
                let key = canon.clone();
                push_deletes_1(&mut vn_deletes, &key, (syl, canon));
            }
        }
        if let Some(canon) = crate::ring_buffer::visible_to_canonical_raw(syl, InputMethod::Vni) {
            if canon.len() >= 3 && canon.len() <= 10 && canon.is_ascii() {
                if vni_exact.insert(canon.clone(), syl).is_none() {
                    let key = canon.clone();
                    push_deletes_1(&mut vni_deletes, &key, (syl, canon.clone()));
                }
                // Also index free-order end-marked VNI form (`"khong6"` alongside `"kho6ng"`).
                let mut end_marked = String::with_capacity(canon.len());
                for b in canon.bytes() {
                    if b.is_ascii_alphabetic() {
                        end_marked.push(b as char);
                    }
                }
                for b in canon.bytes() {
                    if b.is_ascii_digit() {
                        end_marked.push(b as char);
                    }
                }
                if end_marked != canon && vni_exact.insert(end_marked.clone(), syl).is_none() {
                    let key = end_marked.clone();
                    push_deletes_1(&mut vni_deletes, &key, (syl, end_marked));
                }
            }
        }
    }

    let mut en_core: HashSet<&'static str> = HashSet::with_capacity(1600);
    let mut en_exact: HashSet<&'static str> = HashSet::with_capacity(32000);
    let mut en_deletes: HashMap<String, Vec<&'static str>> = HashMap::with_capacity(180000);
    for line in CORE_ENGLISH_RAW.lines() {
        let w = line.trim();
        if w.len() >= 5
            && w.len() <= 16
            && w.is_ascii()
            && w.bytes().all(|b| b.is_ascii_lowercase())
        {
            en_core.insert(w);
            if en_exact.insert(w) {
                push_deletes_1(&mut en_deletes, w, w);
            }
        }
    }
    for line in EXTENDED_ENGLISH_RAW.lines() {
        let w = line.trim();
        if w.len() >= 6
            && w.len() <= 16
            && w.is_ascii()
            && w.bytes().all(|b| b.is_ascii_lowercase())
            && en_exact.insert(w)
        {
            push_deletes_1(&mut en_deletes, w, w);
        }
    }

    SymSpellIndex {
        vn_exact,
        vn_deletes,
        vni_exact,
        vni_deletes,
        en_core,
        en_exact,
        en_deletes,
    }
}

/// Compute `(edit_distance, spatial_cost)` for ASCII strings `src` -> `dst`
/// using layout-aware Damerau-Levenshtein (`edit_distance <= 2`).
fn weighted_spatial_edit(src: &str, dst: &str, layout: AutocorrectLayout) -> Option<(u8, i32)> {
    let a: Vec<char> = src.chars().collect();
    let b: Vec<char> = dst.chars().collect();
    let n = a.len();
    let m = b.len();
    if n.abs_diff(m) > 2 {
        return None;
    }

    // Fast path: edit distance 1 checks (exact spatial weighting)
    if n == m {
        let diffs: Vec<usize> = (0..n).filter(|&i| a[i] != b[i]).collect();
        match diffs.len() {
            0 => return Some((0, 0)),
            1 => {
                let i = diffs[0];
                // Never substitute across letter <-> digit rows (Task B2 safety).
                if a[i].is_ascii_digit() != b[i].is_ascii_digit() {
                    return None;
                }
                let d = key_distance_for_layout(layout, a[i], b[i]);
                let cost = if d <= MAX_SPATIAL_DIST { 10 } else { 26 };
                return Some((1, cost));
            }
            2 => {
                let (i, j) = (diffs[0], diffs[1]);
                // Single adjacent transposition
                if j == i + 1 && a[i] == b[j] && a[j] == b[i] {
                    return Some((1, 10));
                }
                return None;
            }
            _ => return None,
        }
    }

    // Single insertion (user typed 1 extra char in `src`)
    if n == m + 1 {
        let mut i = 0;
        let mut j = 0;
        let mut extra_idx: Option<usize> = None;
        while i < n && j < m {
            if a[i] == b[j] {
                i += 1;
                j += 1;
            } else if extra_idx.is_none() {
                extra_idx = Some(i);
                i += 1;
            } else {
                return None;
            }
        }
        let idx = extra_idx.unwrap_or(n - 1);
        // Protect single-letter prefixed brand/tool names (`gboard`, `dboard`, `kmail`, `vcode`)
        // from being truncated at index 0 unless the leading key was double-pressed (`bboard`).
        let leading_non_dup = idx == 0 && n > 1 && a[0] != a[1];
        let adj_prev =
            idx > 0 && key_distance_for_layout(layout, a[idx], a[idx - 1]) <= MAX_SPATIAL_DIST;
        let adj_next = !leading_non_dup
            && idx + 1 < n
            && key_distance_for_layout(layout, a[idx], a[idx + 1]) <= MAX_SPATIAL_DIST;
        let cost = if adj_prev || adj_next { 12 } else { 18 };
        return Some((1, cost));
    }

    // Single deletion (user missed 1 char in `src`)
    if m == n + 1 {
        let mut i = 0;
        let mut j = 0;
        let mut skipped = false;
        while i < n && j < m {
            if a[i] == b[j] {
                i += 1;
                j += 1;
            } else if !skipped {
                skipped = true;
                j += 1;
            } else {
                return None;
            }
        }
        return Some((1, 14));
    }

    None
}

fn context_support_score(
    user_dict: &crate::user_dict::UserDict,
    prev_words: &[&str],
    candidate: &str,
    is_vn: bool,
) -> i32 {
    if prev_words.is_empty() {
        return 0;
    }
    let mut score = i32::from(crate::ngram_lm::score_word_in_context(
        prev_words, candidate,
    ));
    if let Some(&p1) = prev_words.last() {
        if let Some((p_vn, p_en)) = crate::stat_lm::lookup_bigram_scores(p1, candidate) {
            score += i32::from(if is_vn { p_vn } else { p_en });
        }
        score += user_dict.personal_bigram_bonus(p1, candidate);
    }
    if crate::ngram_lm::predict_next_words(prev_words, 3)
        .iter()
        .any(|p| p == candidate)
    {
        score += 180;
    }
    score
}

fn looks_like_dev_identifier(raw_lower: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "kube", "sys", "hash", "vec", "str", "impl", "serde", "tokio", "async", "lib", "dev",
        "cfg", "env", "tmp", "var", "usr", "etc", "proc", "sock", "ipc", "rpc", "http", "grpc",
        "sql", "json", "toml", "yaml", "xml", "html", "css", "wasm", "llvm", "rustc", "cargo",
        "nvim", "tmux", "ssh", "gpg", "sudo", "chmod", "chown", "grep", "awk", "sed",
    ];
    const SUFFIXES: &[&str] = &[
        "ctl", "map", "vec", "buf", "ptr", "ref", "mut", "cli", "api", "sdk", "cfg", "env", "tmp",
        "log", "err", "ctx", "idx", "len", "cap", "pos", "off", "val", "key", "src", "dst", "dir",
        "ext", "mgr", "svc", "srv", "req", "res", "ack", "seq", "pid", "uid", "gid", "rpc", "sql",
        "xml", "css", "json", "toml", "yaml", "html",
    ];
    PREFIXES.iter().any(|p| raw_lower.starts_with(p))
        || SUFFIXES.iter().any(|s| raw_lower.ends_with(s))
}

fn try_vn_symspell(
    user_dict: &crate::user_dict::UserDict,
    raw_lower: &str,
    method: InputMethod,
    layout: AutocorrectLayout,
    prev_words: &[&str],
) -> Option<String> {
    if raw_lower.len() < 4 || raw_lower.len() > 10 {
        return None;
    }
    let idx = symspell_index();
    let (vn_exact, vn_deletes) = match method {
        InputMethod::Telex => (&idx.vn_exact, &idx.vn_deletes),
        InputMethod::Vni => (&idx.vni_exact, &idx.vni_deletes),
    };
    let mut best_by_syl: HashMap<&'static str, (u8, i32)> = HashMap::new();

    let mut consider = |syl: &'static str, canon: &str| {
        if let Some((dist, cost)) = weighted_spatial_edit(raw_lower, canon, layout) {
            if dist >= 1 && cost <= 24 {
                best_by_syl
                    .entry(syl)
                    .and_modify(|cur| {
                        if (cost, dist) < (cur.1, cur.0) {
                            *cur = (dist, cost);
                        }
                    })
                    .or_insert((dist, cost));
            }
        }
    };

    // 1. Missing 1 char: `raw_lower` itself is a 1-delete of a canonical word
    if let Some(entries) = vn_deletes.get(raw_lower) {
        for (syl, canon) in entries {
            consider(syl, canon);
        }
    }

    // 2. Extra 1 char / substitution / transposition: 1-deletes of `raw_lower`
    let bytes = raw_lower.as_bytes();
    for i in 0..bytes.len() {
        let mut del = String::with_capacity(bytes.len() - 1);
        del.push_str(&raw_lower[..i]);
        del.push_str(&raw_lower[i + 1..]);
        if let Some(&syl) = vn_exact.get(&del) {
            consider(syl, &del);
        }
        if let Some(entries) = vn_deletes.get(&del) {
            for (syl, canon) in entries {
                consider(syl, canon);
            }
        }
    }

    let mut scored: Vec<(&'static str, u8, i32, i32)> = best_by_syl
        .into_iter()
        .map(|(syl, (dist, cost))| {
            let ctx = context_support_score(user_dict, prev_words, syl, true);
            (syl, dist, cost, ctx)
        })
        .collect();

    if scored.is_empty() {
        return None;
    }

    // Rank by (context_bonus - 6 * spatial_cost) descending
    scored.sort_by(|a, b| {
        let ra = a.3 - 6 * a.2;
        let rb = b.3 - 6 * b.2;
        rb.cmp(&ra)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.0.cmp(b.0))
    });

    let best = scored[0];
    if scored.len() == 1 {
        // Sole unambiguous candidate: accept edit-1 with tight spatial cost (<= 16).
        if best.1 == 1 && best.2 <= 16 {
            return Some(best.0.to_string());
        }
        return None;
    }

    // Multiple candidates:
    // 1. With context: require decisive contextual margin (preserves `tieengd -> None` standalone).
    // 2. Without context: accept only if the sole edit-1 winner has a strictly lower spatial cost
    //    than every runner-up (`best.cost + 4 <= second.cost`, e.g. `khoog -> không` cost 14 vs
    //    non-adjacent extra-char `khô` cost 18).
    let second = scored[1];
    let best_rank = best.3 - 6 * best.2;
    let second_rank = second.3 - 6 * second.2;
    if !prev_words.is_empty() && best.3 >= 140 && best_rank >= second_rank + 100 {
        return Some(best.0.to_string());
    }
    if best.1 == 1 && best.2 <= 14 && best.2 + 4 <= second.2 {
        return Some(best.0.to_string());
    }
    None
}

fn try_vn_missing_space_split(
    raw_lower: &str,
    compose: &impl Fn(&str) -> String,
) -> Option<String> {
    if raw_lower.len() < 6 || raw_lower.len() > 18 || looks_like_dev_identifier(raw_lower) {
        return None;
    }
    let mut best: Option<(i32, String)> = None;
    let mut ambiguous = false;

    for split in 2..=(raw_lower.len() - 2) {
        let left = &raw_lower[..split];
        let right = &raw_lower[split..];
        if left.len() > 9 || right.len() > 9 {
            continue;
        }
        let w1 = compose(left);
        let w2 = compose(right);
        // Require at least one diacritic Vietnamese syllable so pure-ASCII
        // identifiers (`hashmap`, `localhost`, `filename`) are never split.
        if w1.is_ascii() && w2.is_ascii() {
            continue;
        }
        if !crate::spelling::is_valid_vietnamese_syllable(&w1)
            || !crate::spelling::is_in_vietnamese_dictionary(&w1)
            || !crate::spelling::is_valid_vietnamese_syllable(&w2)
            || !crate::spelling::is_in_vietnamese_dictionary(&w2)
        {
            continue;
        }
        let bi_vn =
            crate::stat_lm::lookup_bigram_scores(&w1, &w2).map_or(0, |(p_vn, _)| i32::from(p_vn));
        let pred_hit = crate::ngram_lm::predict_next_words(&[&w1], 3)
            .iter()
            .any(|p| p == &w2);
        let en = crate::engine::english_dict();
        // Never use the `both_diacritic` fallback bonus if either half is a real English
        // word (prevents OOV English compounds like `hostlist`, `testlist`, `lastpost`
        // from splitting into `hót lít`/`tét lít` while preserving `dduowcjrooif -> được rồi`).
        let neither_english = !en.is_english_word(left) && !en.is_english_word(right);
        let both_diacritic = neither_english
            && !w1.is_ascii()
            && !w2.is_ascii()
            && left.len() >= 3
            && right.len() >= 3;
        let score = bi_vn + if pred_hit { 160 } else { 0 } + if both_diacritic { 155 } else { 0 };
        if score < 150 {
            continue;
        }
        let joined = format!("{w1} {w2}");
        match &best {
            None => best = Some((score, joined)),
            Some((prev_score, prev_str)) if prev_str == &joined => {
                if score > *prev_score {
                    best = Some((score, joined));
                }
            }
            Some((prev_score, _)) => {
                if (score - *prev_score).abs() < 30 {
                    ambiguous = true;
                } else if score > *prev_score {
                    best = Some((score, joined));
                    ambiguous = false;
                }
            }
        }
    }

    if ambiguous {
        None
    } else {
        best.map(|(_, s)| s)
    }
}

fn try_core_en_symspell(
    user_dict: &crate::user_dict::UserDict,
    raw_lower: &str,
    layout: AutocorrectLayout,
    prev_words: &[&str],
) -> Option<String> {
    if raw_lower.len() < 5
        || raw_lower.len() > 16
        || !raw_lower.bytes().all(|b| b.is_ascii_lowercase())
        || looks_like_dev_identifier(raw_lower)
    {
        return None;
    }
    let idx = symspell_index();
    let mut seen: HashSet<&'static str> = HashSet::new();
    let mut scored: Vec<(&'static str, i32, i32, bool)> = Vec::new(); // (word, cost, ctx, is_core)

    let mut consider = |word: &'static str| {
        if !seen.insert(word) {
            return;
        }
        if let Some((dist, cost)) = weighted_spatial_edit(raw_lower, word, layout) {
            if dist == 1 && cost <= 16 {
                let ctx = context_support_score(user_dict, prev_words, word, false);
                let is_core = idx.en_core.contains(word);
                scored.push((word, cost, ctx, is_core));
            }
        }
    };

    // Missing 1 char in input
    if let Some(words) = idx.en_deletes.get(raw_lower) {
        for &w in words {
            consider(w);
        }
    }
    // Extra 1 char / substitution / transposition in input
    let bytes = raw_lower.as_bytes();
    for i in 0..bytes.len() {
        let mut del = String::with_capacity(bytes.len() - 1);
        del.push_str(&raw_lower[..i]);
        del.push_str(&raw_lower[i + 1..]);
        if let Some(&w) = idx.en_exact.get(del.as_str()) {
            consider(w);
        }
        if let Some(words) = idx.en_deletes.get(&del) {
            for &w in words {
                consider(w);
            }
        }
    }

    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| {
        let ra = a.2 + if a.3 { 40 } else { 0 } - 6 * a.1;
        let rb = b.2 + if b.3 { 40 } else { 0 } - 6 * b.1;
        rb.cmp(&ra)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.0.cmp(b.0))
    });
    if scored.len() == 1 {
        return Some(scored[0].0.to_string());
    }
    let best = scored[0];
    let second = scored[1];
    // Core dictionary word at edit-1 wins decisively over non-core extended candidates
    // unless context favors the extended candidate.
    if best.3 && !second.3 && best.1 <= 14 && best.2 >= second.2 {
        return Some(best.0.to_string());
    }
    if !prev_words.is_empty()
        && best.2 >= 140
        && (best.2 - 6 * best.1) >= (second.2 - 6 * second.1) + 100
    {
        return Some(best.0.to_string());
    }
    None
}

/// Entry point called by `autocorrect.rs` after its exact single-pass checks.
pub(crate) fn suggest_multi_error(
    user_dict: &crate::user_dict::UserDict,
    raw_lower: &str,
    method: InputMethod,
    layout: AutocorrectLayout,
    prev_ctx: LanguageContext,
    prev_words: &[&str],
    compose: &impl Fn(&str) -> String,
) -> Option<String> {
    let valid_chars = match method {
        InputMethod::Telex => raw_lower.bytes().all(|b| b.is_ascii_lowercase()),
        InputMethod::Vni => {
            raw_lower
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_lowercase())
                && raw_lower
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || matches!(b, b'1'..=b'9'))
                && raw_lower.bytes().filter(|b| b.is_ascii_lowercase()).count() >= 2
        }
    };
    if !valid_chars {
        return None;
    }
    if prev_ctx == LanguageContext::English {
        return try_core_en_symspell(user_dict, raw_lower, layout, prev_words);
    }

    if let Some(vn) = try_vn_symspell(user_dict, raw_lower, method, layout, prev_words) {
        return Some(vn);
    }
    if let Some(en) = try_core_en_symspell(user_dict, raw_lower, layout, prev_words) {
        return Some(en);
    }
    try_vn_missing_space_split(raw_lower, compose)
}
