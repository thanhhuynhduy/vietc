// SPDX-License-Identifier: MIT
//! Quantized statistical unigram/bigram language model for bilingual VN/EN scoring.
//!
//! Backed by `bilingual_lm.bin` (generated deterministically by `scripts/build_lm.py`)
//! and embedded at compile time via `include_bytes!`.
//!
//! Architectural guarantees:
//! - Zero `unsafe` code.
//! - Zero heap allocations on the hot path (stack-only UTF-8 lowercase FNV-1a).
//! - O(1) bounded open-addressing lookup (hard probe cap prevents infinite loops
//!   even on corrupted binaries).
//! - Compile-time embedded binary integrity check (`verify_binary_integrity`).

const LM_BYTES: &[u8] = include_bytes!("bilingual_lm.bin");

const MAGIC: &[u8; 4] = b"DBLM";
const EXPECTED_VERSION: u32 = 1;
const HEADER_LEN: usize = 24;
const SLOT_LEN: usize = 10;
const MAX_PROBES: usize = 64;

const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

#[derive(Clone, Copy, Debug)]
struct LmHeader {
    unigram_cap: usize,
    bigram_cap: usize,
    checksum: u64,
}

#[inline]
fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[inline]
fn read_u64_le(bytes: &[u8], offset: usize) -> Option<u64> {
    let slice = bytes.get(offset..offset + 8)?;
    Some(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

#[inline]
fn parse_header(bytes: &[u8]) -> Option<LmHeader> {
    if bytes.len() < HEADER_LEN || bytes.get(0..4)? != MAGIC {
        return None;
    }
    let version = read_u32_le(bytes, 4)?;
    if version != EXPECTED_VERSION {
        return None;
    }
    let unigram_cap = read_u32_le(bytes, 8)? as usize;
    let bigram_cap = read_u32_le(bytes, 12)? as usize;
    if !unigram_cap.is_power_of_two() || !bigram_cap.is_power_of_two() {
        return None;
    }
    let checksum = read_u64_le(bytes, 16)?;
    let expected_len = HEADER_LEN
        .checked_add(unigram_cap.checked_mul(SLOT_LEN)?)?
        .checked_add(bigram_cap.checked_mul(SLOT_LEN)?)?;
    if bytes.len() != expected_len {
        return None;
    }
    Some(LmHeader {
        unigram_cap,
        bigram_cap,
        checksum,
    })
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
fn hash_unigram(word: &str) -> u64 {
    let h = fnv1a_lower_str(FNV_OFFSET, word);
    if h == 0 {
        1
    } else {
        h
    }
}

#[inline]
fn hash_bigram(w1: &str, w2: &str) -> u64 {
    let mut h = fnv1a_lower_str(FNV_OFFSET, w1);
    h = fnv1a_byte(h, 0x1F);
    h = fnv1a_lower_str(h, w2);
    if h == 0 {
        1
    } else {
        h
    }
}

#[inline]
fn lookup_table(table: &[u8], cap: usize, key: u64) -> Option<(u8, u8)> {
    let mask = cap - 1;
    let mut idx = (key as usize) & mask;
    let max_probes = cap.min(MAX_PROBES);
    for _ in 0..max_probes {
        let off = idx * SLOT_LEN;
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
fn cached_header() -> Option<LmHeader> {
    static HEADER: std::sync::OnceLock<Option<LmHeader>> = std::sync::OnceLock::new();
    *HEADER.get_or_init(|| parse_header(LM_BYTES))
}

#[inline]
fn lookup_unigram(word: &str) -> Option<(u8, u8)> {
    let hdr = cached_header()?;
    let start = HEADER_LEN;
    let end = start + hdr.unigram_cap * SLOT_LEN;
    let table = LM_BYTES.get(start..end)?;
    lookup_table(table, hdr.unigram_cap, hash_unigram(word))
}

#[inline]
fn lookup_bigram(w1: &str, w2: &str) -> Option<(u8, u8)> {
    let hdr = cached_header()?;
    let start = HEADER_LEN + hdr.unigram_cap * SLOT_LEN;
    let end = start + hdr.bigram_cap * SLOT_LEN;
    let table = LM_BYTES.get(start..end)?;
    lookup_table(table, hdr.bigram_cap, hash_bigram(w1, w2))
}

#[inline]
pub(crate) fn lookup_bigram_scores(w1: &str, w2: &str) -> Option<(u8, u8)> {
    lookup_bigram(w1, w2)
}

/// Returns the embedded LM binary size in bytes.
pub fn binary_size_bytes() -> usize {
    LM_BYTES.len()
}

/// Verifies magic, version, table geometry, and FNV-1a payload checksum of the
/// embedded `bilingual_lm.bin`.
pub fn verify_binary_integrity() -> bool {
    let Some(hdr) = parse_header(LM_BYTES) else {
        return false;
    };
    let mut h = FNV_OFFSET;
    for &b in &LM_BYTES[HEADER_LEN..] {
        h = fnv1a_byte(h, b);
    }
    h == hdr.checksum
}

/// Returns `(populated_unigrams, populated_bigrams)` in the embedded LM binary.
#[cfg(test)]
fn populated_entry_counts() -> (usize, usize) {
    let Some(hdr) = parse_header(LM_BYTES) else {
        return (0, 0);
    };
    let count_nonzero = |start: usize, cap: usize| -> usize {
        (0..cap)
            .filter(|&i| read_u64_le(LM_BYTES, start + i * SLOT_LEN).unwrap_or(0) != 0)
            .count()
    };
    let uni_start = HEADER_LEN;
    let bi_start = HEADER_LEN + hdr.unigram_cap * SLOT_LEN;
    (
        count_nonzero(uni_start, hdr.unigram_cap),
        count_nonzero(bi_start, hdr.bigram_cap),
    )
}

/// Score a colliding word `(cand_vn, cand_en)` in bilingual context `(prev2, prev1)`.
///
/// - Positive score (`> 0`): statistical evidence favors **English** (`cand_en`, restore).
/// - Negative score (`< 0`): statistical evidence favors **Vietnamese** (`cand_vn`, keep).
/// - `0`: no statistical evidence (defer to morphological/context fallback).
///
/// Bigram hits contribute `2 * (p_en - p_vn)` (range roughly `±510`), while
/// standalone unigram priors are bounded within `±63` so callers can apply a
/// clean confidence threshold (`|score| >= 250`) without overriding standalone
/// word heuristics.
pub(crate) fn score_bilingual(
    prev2: Option<&str>,
    prev1: Option<&str>,
    cand_vn: &str,
    cand_en: &str,
) -> i32 {
    let mut score: i32 = 0;
    let mut has_bigram = false;
    let distinct_vn = !cand_vn.eq_ignore_ascii_case(cand_en);

    if let Some(p1) = prev1 {
        if let Some((p_vn, p_en)) = lookup_bigram(p1, cand_en) {
            score += 2 * (i32::from(p_en) - i32::from(p_vn));
            has_bigram = true;
        }
        if distinct_vn {
            if let Some((p_vn, p_en)) = lookup_bigram(p1, cand_vn) {
                score += 2 * (i32::from(p_en) - i32::from(p_vn));
                has_bigram = true;
            }
        }
    }

    if !has_bigram {
        if let Some(p2) = prev2 {
            if let Some((p_vn, p_en)) = lookup_bigram(p2, cand_en) {
                score += (i32::from(p_en) - i32::from(p_vn)) / 2;
            }
            if distinct_vn {
                if let Some((p_vn, p_en)) = lookup_bigram(p2, cand_vn) {
                    score += (i32::from(p_en) - i32::from(p_vn)) / 2;
                }
            }
        }
    }

    // Add gentle unigram prior (scaled down to <= ±63 so unigrams break ties
    // without crossing the strong bigram decision threshold alone).
    if let Some((p_vn, p_en)) = lookup_unigram(cand_en) {
        score += (i32::from(p_en) - i32::from(p_vn)) / 4;
    }
    if distinct_vn {
        if let Some((p_vn, p_en)) = lookup_unigram(cand_vn) {
            score += (i32::from(p_en) - i32::from(p_vn)) / 4;
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_integrity_and_size_budget() {
        assert!(
            verify_binary_integrity(),
            "embedded bilingual_lm.bin failed integrity check"
        );
        assert!(
            binary_size_bytes() < 350 * 1024,
            "bilingual_lm.bin exceeds 350KB budget: {} bytes",
            binary_size_bytes()
        );
        let (unigrams, bigrams) = populated_entry_counts();
        assert!(
            unigrams >= 1_000,
            "expected >= 1,000 corpus unigrams, got {unigrams}"
        );
        assert!(
            bigrams >= 15_000,
            "expected >= 15,000 corpus bigrams, got {bigrams}"
        );
    }

    #[test]
    fn scores_code_switching_and_compounds_accurately() {
        // VN verb + EN dev object -> strongly positive (English)
        assert!(score_bilingual(None, Some("chạy"), "tét", "test") >= 250);
        assert!(score_bilingual(None, Some("mở"), "file", "file") >= 250);
        assert!(score_bilingual(None, Some("sửa"), "hót", "host") >= 250);
        assert!(score_bilingual(None, Some("xem"), "lít", "list") >= 250);

        // EN bigram -> strongly positive (English)
        assert!(score_bilingual(None, Some("cargo"), "tét", "test") >= 250);
        assert!(score_bilingual(None, Some("in"), "cáe", "case") >= 250);
        assert!(score_bilingual(None, Some("has"), "bên", "been") >= 250);

        // VN compound -> strongly negative (Vietnamese)
        assert!(score_bilingual(None, Some("bánh"), "tét", "test") <= -250);
        assert!(score_bilingual(None, Some("một"), "lát", "last") <= -250);
        assert!(score_bilingual(None, Some("cái"), "bát", "bast") <= -250);
        assert!(score_bilingual(None, Some("trời"), "mát", "mast") <= -250);
        assert!(score_bilingual(None, Some("ăn"), "tô", "too") <= -250);

        // Case-insensitive lookup works without heap allocation
        assert!(score_bilingual(None, Some("Chạy"), "Tét", "Test") >= 250);
        assert!(score_bilingual(None, Some("Bánh"), "Tét", "Test") <= -250);
    }
}
