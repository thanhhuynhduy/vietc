// SPDX-License-Identifier: MIT
//! Quantized Kneser-Ney 3-gram/4-gram Language Model & Next-Word Prediction.
//!
//! Backed by `ngram_lm.bin` (generated deterministically by `scripts/build_lm.py`)
//! and embedded at compile time via `include_bytes!`.
//!
//! Architectural guarantees:
//! - Zero `unsafe` code.
//! - Zero heap allocations on the scoring hot path (`score_context`,
//!   `score_word_in_context`) using stack-only UTF-8 lowercase FNV-1a hashing.
//! - O(1) bounded open-addressing lookup (hard 64-probe cap prevents infinite
//!   loops even on corrupted binaries).
//! - Compile-time embedded binary integrity check (`verify_binary_integrity`).

const NGRAM_BYTES: &[u8] = include_bytes!("ngram_lm.bin");

const MAGIC: &[u8; 4] = b"DBNG";
const EXPECTED_VERSION: u32 = 1;
const HEADER_LEN: usize = 44;
const SCORE_SLOT_LEN: usize = 10; // u64 key + u8 p_vn + u8 p_en
const PRED_SLOT_LEN: usize = 14; // u64 key + 3 * u16 vocab_id
const MAX_PROBES: usize = 64;

const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

#[derive(Clone, Copy, Debug)]
struct NgramHeader {
    trigram_cap: usize,
    fourgram_cap: usize,
    pred_bi_cap: usize,
    pred_tri_cap: usize,
    vocab_count: usize,
    vocab_blob_len: usize,
    checksum: u64,
    // Precomputed section byte offsets inside `NGRAM_BYTES`:
    tri_off: usize,
    four_off: usize,
    pred_bi_off: usize,
    pred_tri_off: usize,
    vocab_offsets_off: usize,
    vocab_blob_off: usize,
}

#[inline]
fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let s = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

#[inline]
fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let s = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[inline]
fn read_u64_le(bytes: &[u8], offset: usize) -> Option<u64> {
    let s = bytes.get(offset..offset + 8)?;
    Some(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn parse_header(bytes: &[u8]) -> Option<NgramHeader> {
    if bytes.len() < HEADER_LEN || bytes.get(0..4)? != MAGIC {
        return None;
    }
    let version = read_u32_le(bytes, 4)?;
    if version != EXPECTED_VERSION {
        return None;
    }
    let trigram_cap = read_u32_le(bytes, 8)? as usize;
    let fourgram_cap = read_u32_le(bytes, 12)? as usize;
    let pred_bi_cap = read_u32_le(bytes, 16)? as usize;
    let pred_tri_cap = read_u32_le(bytes, 20)? as usize;
    let vocab_count = read_u32_le(bytes, 24)? as usize;
    let vocab_offsets_len = read_u32_le(bytes, 28)? as usize;
    let vocab_blob_len = read_u32_le(bytes, 32)? as usize;
    let checksum = read_u64_le(bytes, 36)?;

    if !trigram_cap.is_power_of_two()
        || !fourgram_cap.is_power_of_two()
        || !pred_bi_cap.is_power_of_two()
        || !pred_tri_cap.is_power_of_two()
    {
        return None;
    }
    if vocab_offsets_len != vocab_count.checked_add(1)?.checked_mul(4)? {
        return None;
    }

    let tri_off = HEADER_LEN;
    let four_off = tri_off.checked_add(trigram_cap.checked_mul(SCORE_SLOT_LEN)?)?;
    let pred_bi_off = four_off.checked_add(fourgram_cap.checked_mul(SCORE_SLOT_LEN)?)?;
    let pred_tri_off = pred_bi_off.checked_add(pred_bi_cap.checked_mul(PRED_SLOT_LEN)?)?;
    let vocab_offsets_off = pred_tri_off.checked_add(pred_tri_cap.checked_mul(PRED_SLOT_LEN)?)?;
    let vocab_blob_off = vocab_offsets_off.checked_add(vocab_offsets_len)?;
    let expected_len = vocab_blob_off.checked_add(vocab_blob_len)?;
    if bytes.len() != expected_len {
        return None;
    }

    Some(NgramHeader {
        trigram_cap,
        fourgram_cap,
        pred_bi_cap,
        pred_tri_cap,
        vocab_count,
        vocab_blob_len,
        checksum,
        tri_off,
        four_off,
        pred_bi_off,
        pred_tri_off,
        vocab_offsets_off,
        vocab_blob_off,
    })
}

#[inline]
fn cached_header() -> Option<NgramHeader> {
    static HEADER: std::sync::OnceLock<Option<NgramHeader>> = std::sync::OnceLock::new();
    *HEADER.get_or_init(|| parse_header(NGRAM_BYTES))
}

#[inline]
fn fnv1a_byte(mut h: u64, b: u8) -> u64 {
    h ^= u64::from(b);
    h.wrapping_mul(FNV_PRIME)
}

#[inline]
fn fnv1a_lower_str(mut h: u64, s: &str) -> u64 {
    let mut utf8_buf = [0u8; 4];
    for ch in s.chars() {
        for lower in ch.to_lowercase() {
            let encoded = lower.encode_utf8(&mut utf8_buf);
            for &b in encoded.as_bytes() {
                h = fnv1a_byte(h, b);
            }
        }
    }
    h
}

#[inline]
fn hash_tokens(tokens: &[&str]) -> u64 {
    let mut h = FNV_OFFSET;
    for (idx, &tok) in tokens.iter().enumerate() {
        if idx > 0 {
            h = fnv1a_byte(h, 0x1F);
        }
        h = fnv1a_lower_str(h, tok);
    }
    if h == 0 {
        1
    } else {
        h
    }
}

#[inline]
fn lookup_score_table(table: &[u8], cap: usize, key: u64) -> Option<(u8, u8)> {
    let mask = cap - 1;
    let mut idx = (key as usize) & mask;
    let max_probes = cap.min(MAX_PROBES);
    for _ in 0..max_probes {
        let off = idx * SCORE_SLOT_LEN;
        let slot_key = read_u64_le(table, off)?;
        if slot_key == 0 {
            return None;
        }
        if slot_key == key {
            let p_vn = *table.get(off + 8)?;
            let p_en = *table.get(off + 9)?;
            return Some((p_vn, p_en));
        }
        idx = (idx + 1) & mask;
    }
    None
}

#[inline]
fn lookup_pred_table(table: &[u8], cap: usize, key: u64) -> Option<[u16; 3]> {
    let mask = cap - 1;
    let mut idx = (key as usize) & mask;
    let max_probes = cap.min(MAX_PROBES);
    for _ in 0..max_probes {
        let off = idx * PRED_SLOT_LEN;
        let slot_key = read_u64_le(table, off)?;
        if slot_key == 0 {
            return None;
        }
        if slot_key == key {
            return Some([
                read_u16_le(table, off + 8)?,
                read_u16_le(table, off + 10)?,
                read_u16_le(table, off + 12)?,
            ]);
        }
        idx = (idx + 1) & mask;
    }
    None
}

#[inline]
fn vocab_word(hdr: NgramHeader, id: u16) -> Option<&'static str> {
    if id == 0 {
        return None;
    }
    let idx = (id as usize) - 1;
    if idx >= hdr.vocab_count {
        return None;
    }
    let start = read_u32_le(NGRAM_BYTES, hdr.vocab_offsets_off + idx * 4)? as usize;
    let end = read_u32_le(NGRAM_BYTES, hdr.vocab_offsets_off + (idx + 1) * 4)? as usize;
    if end < start || end > hdr.vocab_blob_len {
        return None;
    }
    let slice = NGRAM_BYTES.get(hdr.vocab_blob_off + start..hdr.vocab_blob_off + end)?;
    std::str::from_utf8(slice).ok()
}

#[inline]
fn lookup_trigram(w1: &str, w2: &str, w3: &str) -> Option<(u8, u8)> {
    let hdr = cached_header()?;
    let table = NGRAM_BYTES.get(hdr.tri_off..hdr.four_off)?;
    lookup_score_table(table, hdr.trigram_cap, hash_tokens(&[w1, w2, w3]))
}

#[inline]
fn lookup_fourgram(w1: &str, w2: &str, w3: &str, w4: &str) -> Option<(u8, u8)> {
    let hdr = cached_header()?;
    let table = NGRAM_BYTES.get(hdr.four_off..hdr.pred_bi_off)?;
    lookup_score_table(table, hdr.fourgram_cap, hash_tokens(&[w1, w2, w3, w4]))
}

/// Returns the embedded n-gram LM binary size in bytes.
pub fn binary_size_bytes() -> usize {
    NGRAM_BYTES.len()
}

/// Verifies magic, version, table geometry, and FNV-1a payload checksum of the
/// embedded `ngram_lm.bin`.
pub fn verify_binary_integrity() -> bool {
    let Some(hdr) = parse_header(NGRAM_BYTES) else {
        return false;
    };
    let mut h = FNV_OFFSET;
    for &b in &NGRAM_BYTES[HEADER_LEN..] {
        h = fnv1a_byte(h, b);
    }
    h == hdr.checksum
}

/// Score a colliding word `(cand_vn, cand_en)` using 4-gram & 3-gram Kneser-Ney
/// statistical context `(prev3, prev2, prev1)` with zero heap allocations.
///
/// - Positive score (`> 0`): strong 3-gram/4-gram evidence for **English** (`cand_en`).
/// - Negative score (`< 0`): strong 3-gram/4-gram evidence for **Vietnamese** (`cand_vn`).
/// - `0`: no 3-gram/4-gram match (caller falls back to 2-gram/unigram in `stat_lm`).
pub fn score_context(prev_words: &[&str], cand_vn: &str, cand_en: &str) -> i32 {
    let n = prev_words.len();
    if n < 2 {
        return 0;
    }
    let p2 = prev_words[n - 2];
    let p1 = prev_words[n - 1];
    let distinct_vn = !cand_vn.eq_ignore_ascii_case(cand_en);
    let mut score: i32 = 0;

    // 1. Highest priority: 4-gram `(p3, p2, p1, cand)` (weight 3x)
    if n >= 3 {
        let p3 = prev_words[n - 3];
        if let Some((p_vn, p_en)) = lookup_fourgram(p3, p2, p1, cand_en) {
            score += 3 * (i32::from(p_en) - i32::from(p_vn));
        }
        if distinct_vn {
            if let Some((p_vn, p_en)) = lookup_fourgram(p3, p2, p1, cand_vn) {
                score += 3 * (i32::from(p_en) - i32::from(p_vn));
            }
        }
    }

    // 2. Trigram `(p2, p1, cand)` (weight 2x)
    if let Some((p_vn, p_en)) = lookup_trigram(p2, p1, cand_en) {
        score += 2 * (i32::from(p_en) - i32::from(p_vn));
    }
    if distinct_vn {
        if let Some((p_vn, p_en)) = lookup_trigram(p2, p1, cand_vn) {
            score += 2 * (i32::from(p_en) - i32::from(p_vn));
        }
    }

    score
}

/// Score a single candidate word's contextual plausibility in `prev_words`
/// (combining 4-gram and 3-gram evidence) for ranking autocorrect candidates.
pub fn score_word_in_context(prev_words: &[&str], candidate: &str) -> u16 {
    let n = prev_words.len();
    let mut total: u16 = 0;
    if n >= 3 {
        if let Some((p_vn, p_en)) = lookup_fourgram(
            prev_words[n - 3],
            prev_words[n - 2],
            prev_words[n - 1],
            candidate,
        ) {
            total = total.saturating_add(u16::from(p_vn.max(p_en)) * 2);
        }
    }
    if n >= 2 {
        if let Some((p_vn, p_en)) = lookup_trigram(prev_words[n - 2], prev_words[n - 1], candidate)
        {
            total = total.saturating_add(u16::from(p_vn.max(p_en)));
        }
    }
    total
}

/// Predict up to `limit` (capped at 3) most likely next words given committed
/// preceding words `prev_words`, backing off cleanly from trigram `(p2, p1)`
/// to bigram `(p1)` in O(1).
pub fn predict_next_words(prev_words: &[&str], limit: usize) -> Vec<String> {
    let cap = limit.min(3);
    if cap == 0 || prev_words.is_empty() {
        return Vec::new();
    }
    let Some(hdr) = cached_header() else {
        return Vec::new();
    };
    let n = prev_words.len();
    let mut out: Vec<String> = Vec::with_capacity(cap);

    // 1. Trigram next-word prediction: `(prev2, prev1) -> top 3`
    if n >= 2 {
        if let Some(table) = NGRAM_BYTES.get(hdr.pred_tri_off..hdr.vocab_offsets_off) {
            let key = hash_tokens(&[prev_words[n - 2], prev_words[n - 1]]);
            if let Some(ids) = lookup_pred_table(table, hdr.pred_tri_cap, key) {
                for id in ids {
                    if out.len() >= cap {
                        break;
                    }
                    if let Some(word) = vocab_word(hdr, id) {
                        if !out.iter().any(|w| w == word) {
                            out.push(word.to_string());
                        }
                    }
                }
            }
        }
    }

    // 2. Backoff to bigram next-word prediction: `(prev1) -> top 3`
    if out.len() < cap {
        if let Some(table) = NGRAM_BYTES.get(hdr.pred_bi_off..hdr.pred_tri_off) {
            let key = hash_tokens(&[prev_words[n - 1]]);
            if let Some(ids) = lookup_pred_table(table, hdr.pred_bi_cap, key) {
                for id in ids {
                    if out.len() >= cap {
                        break;
                    }
                    if let Some(word) = vocab_word(hdr, id) {
                        if !out.iter().any(|w| w == word) {
                            out.push(word.to_string());
                        }
                    }
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_integrity_and_size_budget() {
        assert!(
            verify_binary_integrity(),
            "embedded ngram_lm.bin failed integrity check"
        );
        let size = binary_size_bytes();
        assert!(
            size > 512 * 1024 && size <= 8 * 1024 * 1024,
            "ngram_lm.bin size out of expected range (0.5MB..=8MB): {size} bytes"
        );
        let hdr = cached_header().expect("valid header");
        assert!(
            hdr.vocab_count >= 5_000,
            "vocab too small: {}",
            hdr.vocab_count
        );
    }

    #[test]
    fn predicts_vietnamese_and_english_next_words() {
        let xin = predict_next_words(&["xin"], 3);
        assert!(
            xin.iter().any(|w| w == "chào"),
            "expected 'chào' after 'xin', got {xin:?}"
        );

        let cam_on = predict_next_words(&["cảm", "ơn"], 3);
        assert!(
            cam_on.iter().any(|w| w == "bạn"),
            "expected 'bạn' after 'cảm ơn', got {cam_on:?}"
        );

        let thank_you = predict_next_words(&["thank", "you"], 3);
        assert!(
            thank_you.iter().any(|w| w == "for"),
            "expected 'for' after 'thank you', got {thank_you:?}"
        );

        // Case-insensitive lookup works identically
        assert_eq!(predict_next_words(&["Xin"], 3), xin);
    }

    #[test]
    fn scores_trigram_and_fourgram_context() {
        // Compositional VN + EN dev trigram -> strongly positive (English)
        let cs = score_context(&["đang", "chạy"], "tét", "test");
        assert!(
            cs >= 250,
            "expected strong EN score for 'đang chạy test', got {cs}"
        );

        // Natural English trigram -> positive
        let en = score_context(&["in", "the"], "pát", "past");
        assert!(
            en >= 250,
            "expected strong EN score for 'in the past', got {en}"
        );

        // Contextual word plausibility scoring (for autocorrect ranking)
        assert!(score_word_in_context(&["việt", "nam"], "là") > 0);
    }
}
