// SPDX-License-Identifier: MIT
//! Gboard-style Quantized Bigram & Morphological Scorer + Retroactive 2-Word
//! Disambiguator for Vietnamese–English bilingual typing.

use crate::english::LanguageContext;
use crate::ring_buffer::CommittedWordEntry;

/// Forward word-bigram override `(prev_visible_lower, current_raw_lower) -> Option<bool>`:
/// - `Some(true)`  => force auto-restore to English (`"bài test"`, `"xe bus"`, `"bếp gas"`).
/// - `Some(false)` => force keep Vietnamese (`"file đó"`, `"bug gì"`, `"repo tên"`).
/// - `None`        => defer to standard unigram + `LanguageContext` scorer.
pub(crate) fn evaluate_forward_bigram(
    prev_visible: Option<&str>,
    current_composed: &str,
    current_raw: &str,
    prev_context: LanguageContext,
) -> Option<bool> {
    let prev = prev_visible?.to_lowercase();
    let raw_lower = current_raw.to_ascii_lowercase();
    let comp_lower = current_composed.to_lowercase();

    // Sentence-start / English-context idioms starting with a bilingual ASCII homograph
    // (`"me too"`, `"in its"`, `"a car"`), while keeping `"ăn me ít"` / `"máy in ít"` Vietnamese!
    if prev_context != LanguageContext::Vietnamese
        && matches!(
            (prev.as_str(), raw_lower.as_str()),
            ("me", "too") | ("in", "its") | ("a", "car")
        )
    {
        return Some(true);
    }

    // 1. High-frequency Vietnamese -> English loanword collocations where `prev` is
    //    Vietnamese (`prev_context == Vietnamese`) yet the second token is unambiguously English.
    match raw_lower.as_str() {
        "test"
            if matches!(
                prev.as_str(),
                "bài"
                    | "chạy"
                    | "viết"
                    | "làm"
                    | "bộ"
                    | "hàm"
                    | "bản"
                    | "kỳ"
                    | "ca"
                    | "trường"
                    | "quả"
                    | "đội"
                    | "nên"
                    | "cần"
                    | "để"
                    | "tự"
                    | "muốn"
                    | "phải"
                    | "sẽ"
                    | "đang"
                    | "vừa"
                    | "thử"
                    | "dụng"
                    | "rồi"
                    | "và"
                    | "xong"
                    | "vào"
                    | "lại"
                    | "khi"
                    | "mới"
                    | "chưa"
                    | "đã"
            ) =>
        {
            return Some(true);
        }
        "bus" if matches!(prev.as_str(), "xe" | "đi" | "tuyến" | "trạm" | "bắt" | "vé") => {
            return Some(true);
        }
        "gas"
            if matches!(
                prev.as_str(),
                "bếp" | "bình" | "khí" | "ống" | "van" | "nạp" | "đổi" | "ga"
            ) =>
        {
            return Some(true);
        }
        "bar" if matches!(prev.as_str(), "quán" | "thanh" | "quầy") => {
            return Some(true);
        }
        "docs" | "repo" | "demo" | "todo" | "logo" | "macro" | "sudo" | "cargo" | "server"
        | "user"
            if matches!(
                prev.as_str(),
                "xem"
                    | "đọc"
                    | "viết"
                    | "tạo"
                    | "mở"
                    | "chạy"
                    | "bản"
                    | "con"
                    | "trang"
                    | "quyền"
                    | "lệnh"
            ) =>
        {
            return Some(true);
        }
        "rust" if matches!(prev.as_str(), "học" | "ngôn" | "bằng") => {
            return Some(true);
        }
        "gif" if matches!(prev.as_str(), "ảnh" | "hình") => {
            return Some(true);
        }
        "caps" if matches!(prev.as_str(), "phím" | "bật" | "tắt") => {
            return Some(true);
        }
        "tips" if matches!(prev.as_str(), "sẻ" | "mẹo" | "nhiều" | "vài") => {
            return Some(true);
        }
        "maps" if matches!(prev.as_str(), "xem" | "mở" | "tra" | "trên") => {
            return Some(true);
        }
        _ => {}
    }

    // 2. High-frequency English-tech-noun -> Vietnamese grammar particle collocations
    //    where `prev` is English (`"file"`, `"bug"`, `"code"`, `"repo"`, `"server"`, `"app"`)
    //    followed by a Vietnamese demonstrative/interrogative (`"đó"`=`ddos`, `"gì"`=`gif`,
    //    `"tên"`=`teen`, `"bên"`=`been`, `"cả"`=`car`, `"ít"`=`its`).
    match (raw_lower.as_str(), comp_lower.as_str()) {
        ("ddos", "đó")
            if !matches!(
                prev.as_str(),
                "mitigate"
                    | "anti"
                    | "prevent"
                    | "stop"
                    | "under"
                    | "massive"
                    | "distributed"
                    | "layer"
            ) =>
        {
            return Some(false);
        }
        ("gif", "gì")
            if !matches!(
                prev.as_str(),
                "animated" | "file" | "image" | "export" | "send" | "funny" | "meme" | "format"
            ) =>
        {
            return Some(false);
        }
        ("teen", "tên")
            if matches!(
                prev.as_str(),
                "file" | "repo" | "branch" | "app" | "user" | "server" | "table" | "column" | "key"
            ) =>
        {
            return Some(false);
        }
        ("been", "bên")
            if !(matches!(
                prev.as_str(),
                "have"
                    | "has"
                    | "had"
                    | "not"
                    | "never"
                    | "already"
                    | "always"
                    | "just"
                    | "recently"
            ) || (prev_context == LanguageContext::English
                && matches!(prev.as_str(), "ve" | "s" | "d" | "t"))) =>
        {
            return Some(false);
        }
        _ => {}
    }

    // 3. Strong Vietnamese compound bigrams when Word 1 is already Vietnamese
    //    (e.g. `"bánh tét"`, `"cảm ơn"`) or a sentence-start double-homograph (`"Low mow"` -> `"Lơ mơ"`).
    if is_vietnamese_lexical_compound(&prev, &comp_lower) || (prev == "low" && comp_lower == "mơ")
    {
        return Some(false);
    }

    None
}

/// Checks whether `(prev_entry, current_final_word, current_raw)` warrants a
/// Gboard-style **Retroactive 2-Word Correction** of `prev_entry`!
///
/// Returns `Some((corrected_first_word, new_was_auto_restored))` when `prev_entry`
/// should be rewritten alongside the current word in one atomic replacement.
pub(crate) fn evaluate_retroactive_first_word(
    prev_entry: &CommittedWordEntry,
    current_final_word: &str,
    current_raw: &str,
    method_compose_vn: impl FnOnce(&str) -> String,
) -> Option<(String, bool)> {
    // Only retroactively adjust across a single space when the user did not manually override.
    if prev_entry.user_override_vn || prev_entry.trailing_delimiters != " " {
        return None;
    }

    let w1_raw_lower = prev_entry.raw_keys.to_ascii_lowercase();
    let w2_final_lower = current_final_word.to_lowercase();
    let w2_raw_lower = current_raw.to_ascii_lowercase();

    if !prev_entry.was_auto_restored {
        // Direction A: Word 1 was kept as Vietnamese at sentence start (e.g. `"Lots "` -> `"Lót "`,
        // `"Sets "` -> `"Sét "`, `"Too "` -> `"Tô "`, `"Been "` -> `"Bên "`, `"Its "` -> `"Ít "`),
        // and Word 2 completes an unambiguous English grammatical phrase (`"Lots of"`, `"Too much"`).
        if is_english_retroactive_phrase(&w1_raw_lower, &w2_final_lower, &w2_raw_lower)
            && prev_entry.visible_word != prev_entry.raw_keys
        {
            return Some((prev_entry.raw_keys.clone(), true));
        }
    } else {
        // Direction B: Word 1 was auto-restored to English at sentence start because `prev_context`
        // was `None` (e.g. `"Last "` for `"Lát"`, `"Nest "` for `"Nét"`, `"Cast "` for `"Cát"`,
        // `"Max "` for `"Mã"`, `"Rust "` for `"Rút"`, `"This "` for `"Thí"`, `"Six "` for `"Sĩ"`),
        // and Word 2 forms a genuine Vietnamese 2-syllable lexical compound (`"Lát nữa"`, `"Mã số"`,
        // `"Rút tiền"`, `"Thí nghiệm"`, `"Sĩ quan"`).
        let w1_vn = method_compose_vn(&prev_entry.raw_keys);
        let w1_vn_lower = w1_vn.to_lowercase();
        if w1_vn != prev_entry.visible_word
            && crate::spelling::is_valid_vietnamese_syllable(&w1_vn)
            && crate::spelling::is_in_vietnamese_dictionary(&w1_vn)
            && is_vietnamese_lexical_compound(&w1_vn_lower, &w2_final_lower)
        {
            return Some((w1_vn, false));
        }
    }

    None
}

pub(crate) fn is_english_retroactive_phrase(w1_raw: &str, w2_final: &str, w2_raw: &str) -> bool {
    let w2 = if w2_final.is_ascii() {
        w2_final
    } else if matches!(
        w2_raw,
        "of" | "own" | "soon" | "low" | "here" | "there" | "more" | "far" | "hard" | "first"
    ) {
        w2_raw
    } else {
        return false;
    };
    match w1_raw {
        "lots" | "sets" | "tops" | "cups" | "cuts" | "nuts" | "bats" | "cats" | "hats" | "mats"
        | "rats" | "nets" => {
            matches!(w2, "of" | "for" | "from" | "with" | "and" | "are" | "were")
                || (w1_raw == "cuts" && matches!(w2, "off" | "down" | "through" | "into"))
        }
        "maps" => matches!(w2, "of" | "from" | "for"),
        "tips" => matches!(w2, "for" | "from" | "and"),
        "hits" => matches!(w2, "the" | "from"),
        "test" => matches!(
            w2,
            "suite"
                | "case"
                | "cases"
                | "plan"
                | "run"
                | "runner"
                | "file"
                | "data"
                | "env"
                | "tool"
                | "tools"
                | "result"
                | "results"
                | "report"
                | "coverage"
        ),
        "too" => matches!(
            w2,
            "much"
                | "many"
                | "late"
                | "far"
                | "fast"
                | "slow"
                | "big"
                | "small"
                | "good"
                | "bad"
                | "hard"
                | "easy"
                | "high"
                | "low"
                | "old"
                | "new"
                | "early"
                | "soon"
                | "often"
                | "short"
                | "heavy"
                | "complex"
        ),
        "its" => matches!(
            w2,
            "own"
                | "best"
                | "first"
                | "main"
                | "new"
                | "current"
                | "original"
                | "value"
                | "type"
                | "name"
                | "size"
                | "state"
                | "status"
                | "data"
                | "performance"
        ),
        "been" => matches!(
            w2,
            "the"
                | "there"
                | "here"
                | "done"
                | "waiting"
                | "working"
                | "updated"
                | "fixed"
                | "merged"
                | "deployed"
                | "tested"
                | "able"
        ),
        "car" => matches!(
            w2,
            "is" | "was" | "park" | "parking" | "rental" | "insurance" | "keys"
        ),
        _ => false,
    }
}

pub(crate) fn is_vietnamese_lexical_compound(w1_vn_lower: &str, w2_vn_lower: &str) -> bool {
    match w1_vn_lower {
        "lát" => matches!(
            w2_vn_lower,
            "nữa" | "sau" | "gạch" | "đá" | "cắt" | "bánh" | "thịt" | "mỏng" | "sàn" | "gỗ"
        ),
        "nét" => matches!(
            w2_vn_lower,
            "đẹp" | "chữ" | "mặt" | "vẽ" | "bút" | "riêng" | "mới" | "cổ" | "thanh" | "duyên"
        ),
        "cát" => matches!(
            w2_vn_lower,
            "bụi" | "trắng" | "vàng" | "biển" | "tường" | "lún" | "sỏi" | "mịn"
        ),
        "mã" => matches!(
            w2_vn_lower,
            "số" | "nguồn"
                | "hóa"
                | "hoá"
                | "vạch"
                | "độc"
                | "vùng"
                | "lực"
                | "nhận"
                | "xác"
                | "bảo"
                | "ưu"
                | "giảm"
                | "thẻ"
                | "định"
                | "hàng"
        ),
        "rút" => matches!(
            w2_vn_lower,
            "tiền"
                | "gọn"
                | "ngắn"
                | "lui"
                | "khỏi"
                | "cuộc"
                | "kinh"
                | "đơn"
                | "bớt"
                | "dây"
                | "quân"
                | "hồ"
                | "thăm"
                | "lời"
                | "vốn"
        ),
        "rét" => matches!(
            w2_vn_lower,
            "đậm" | "hại" | "buốt" | "mướt" | "quá" | "run" | "căm" | "nàng"
        ),
        "lơ" => matches!(w2_vn_lower, "mơ" | "là" | "đãng" | "ngơ" | "lửng"),
        "nơ" => matches!(
            w2_vn_lower,
            "đỏ" | "hồng" | "trắng" | "đen" | "bướm" | "xinh"
        ),
        "mốt" => matches!(w2_vn_lower, "mới" | "cũ" | "thời" | "hiện"),
        "hót" => matches!(w2_vn_lower, "líu" | "vang" | "hay" | "rộn"),
        "bú" => matches!(w2_vn_lower, "sữa" | "mẹ" | "bình" | "mớm"),
        "á" => matches!(w2_vn_lower, "hậu" | "quân" | "đông" | "âu" | "khôi"),
        "ú" => matches!(w2_vn_lower, "òa" | "oà" | "tim" | "ớ" | "nụ"),
        "ì" => matches!(w2_vn_lower, "ạch" | "xèo" | "ầm"),
        "thí" => matches!(
            w2_vn_lower,
            "sinh" | "điểm" | "nghiệm" | "dụ" | "chủ" | "mạng"
        ),
        "sĩ" => matches!(
            w2_vn_lower,
            "quan" | "diện" | "tử" | "khí" | "số" | "phu" | "tốt"
        ),
        "mĩ" => matches!(
            w2_vn_lower,
            "thuật" | "mãn" | "nhân" | "quan" | "lệ" | "miều" | "ẩm" | "tục"
        ),
        "hơ" => matches!(w2_vn_lower, "nóng" | "lửa" | "ấm" | "tay" | "khô"),
        "ơn" => matches!(
            w2_vn_lower,
            "trời" | "giời" | "nghĩa" | "đời" | "huệ" | "trên"
        ),
        "lít" => matches!(
            w2_vn_lower,
            "nước" | "xăng" | "dầu" | "sữa" | "rượu" | "bia"
        ),
        "lót" => matches!(
            w2_vn_lower,
            "chuột" | "dạ" | "đường" | "gạch" | "sàn" | "giày" | "nệm" | "ổ" | "lòng"
        ),
        "lăn" => matches!(
            w2_vn_lower,
            "bánh" | "lộn" | "tăn" | "đùng" | "khử" | "bột" | "tròn"
        ),
        "nôn" => matches!(w2_vn_lower, "nóng" | "nao" | "mửa" | "ói" | "trớ"),
        "bủn" => matches!(w2_vn_lower, "rủn" | "xỉn"),
        "bỏn" => w2_vn_lower == "xẻn",
        "cỏn" => w2_vn_lower == "con",
        "hí" => matches!(w2_vn_lower, "hửng" | "hoáy"),
        "tét" => matches!(w2_vn_lower, "mông" | "mắt" | "đít"),
        "tá" => matches!(w2_vn_lower, "hỏa" | "hoả" | "lả" | "túc"),
        "bả" => matches!(w2_vn_lower, "vai" | "chó" | "mồi"),
        "bánh" => w2_vn_lower == "tét",
        "cảm" | "biết" | "mang" | "cám" => w2_vn_lower == "ơn",
        _ => false,
    }
}

/// Morphological English compound detector for OOV words whose double letters
/// triggered Telex undo (`composed.is_ascii() && composed != raw`, e.g.
/// `"serverless"`, `"treesitter"`, `"storybook"`, `"glassware"`, `"bedroom"`).
pub(crate) fn has_english_compound_morphology(raw_lower: &str) -> bool {
    if raw_lower.len() < 6 || !raw_lower.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    // Never treat intentional triple-letter Telex escapes (`boooks`, `tesst` len 5) as compounds.
    let bytes = raw_lower.as_bytes();
    for w in bytes.windows(3) {
        if w[0] == w[1] && w[1] == w[2] {
            return false;
        }
    }
    const ROOTS_AND_AFFIXES: &[&str] = &[
        "less", "ness", "ment", "able", "ible", "tion", "sion", "ship", "ward", "ware", "work",
        "book", "room", "wood", "pool", "feed", "week", "keep", "look", "tool", "boot", "loop",
        "root", "foot", "good", "hood", "food", "mood", "need", "seed", "speed", "free", "tree",
        "screen", "green", "sheet", "street", "fleet", "pass", "class", "glass", "grass", "press",
        "cross", "loss", "boss", "house", "stone", "board", "line", "time", "side", "site", "code",
    ];
    ROOTS_AND_AFFIXES
        .iter()
        .any(|affix| raw_lower.starts_with(affix) || raw_lower.ends_with(affix))
}
