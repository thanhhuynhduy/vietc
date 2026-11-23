// SPDX-License-Identifier: MIT
//! Retroactive restoration of dot-separated filenames (`test.py`, `list.txt`,
//! `host.conf`, `test.tar.gz`) typed mid-sentence without breaking Vietnamese
//! sentence-ending periods (`"ăn bánh tét. hôm nay"`).

use crate::engine_helpers::is_sentence_boundary;
use crate::english::{is_alphanumeric_tech_token, is_tech_token, EnglishDict, LanguageContext};

#[derive(Debug, Clone)]
pub(crate) struct DotStemEntry {
    pub raw_keys: String,
    pub visible_word: String,
}

/// Known development, configuration, document, archive, and media file
/// extensions used to disambiguate `<stem>.<ext>` from Vietnamese prose.
pub(crate) fn is_known_file_extension(ext_lower: &str) -> bool {
    matches!(
        ext_lower,
        "py" | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "txt"
            | "md"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "conf"
            | "cfg"
            | "ini"
            | "sh"
            | "bash"
            | "zsh"
            | "css"
            | "html"
            | "xml"
            | "sql"
            | "env"
            | "lock"
            | "log"
            | "csv"
            | "pdf"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "svg"
            | "gif"
            | "mp4"
            | "mp3"
            | "zip"
            | "tar"
            | "gz"
            | "deb"
            | "rpm"
            | "exe"
            | "so"
            | "dll"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "go"
            | "java"
            | "kt"
            | "rb"
            | "php"
            | "docx"
            | "doc"
            | "xlsx"
            | "xls"
            | "pptx"
            | "ppt"
            | "odt"
            | "ods"
            | "lua"
            | "zig"
            | "nix"
            | "vue"
            | "svelte"
            | "7z"
            | "rar"
    )
}

/// Evaluate whether flushing `raw` after `prev_boundary == Some('.')` completes
/// a dot-separated filename (`<stem>.<ext>`).
///
/// Returns `Some((total_delete_chars, replacement_insert, updated_context))`
/// when retroactive restoration should replace `<visible_stem>.<visible_ext>`
/// with `<raw_stem>.<raw_ext>` atomically.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_dot_filename_restore(
    last_dot_stem: &mut Option<DotStemEntry>,
    dict: &EnglishDict,
    prev_boundary: Option<char>,
    flush_ch: char,
    raw: &str,
    previous: &str,
    final_word: &str,
    prev_len: usize,
    user_override: bool,
    had_pasted_prefix: bool,
) -> Option<(usize, String, LanguageContext)> {
    let prior_stem = last_dot_stem.take();

    if prev_boundary == Some('.') && !user_override && !had_pasted_prefix {
        if let Some(stem) = prior_stem {
            let ext_lower = raw.to_ascii_lowercase();
            let stem_lower = stem.raw_keys.to_ascii_lowercase();
            let stem_looks_tech_or_en = dict.is_english_word(&stem_lower)
                || is_tech_token(&stem_lower)
                || is_alphanumeric_tech_token(&stem.raw_keys);
            if is_known_file_extension(&ext_lower)
                && stem_looks_tech_or_en
                && (stem.visible_word != stem.raw_keys || previous != raw)
            {
                let total_delete = stem.visible_word.chars().count() + 1 + prev_len;
                let insert = format!("{}.{}", stem.raw_keys, raw);
                if flush_ch == '.' {
                    // Multi-dot filename (e.g., `test.tar.gz`): chain restored ext as next stem.
                    *last_dot_stem = Some(DotStemEntry {
                        raw_keys: raw.to_string(),
                        visible_word: raw.to_string(),
                    });
                }
                let updated_ctx = if is_sentence_boundary(flush_ch) {
                    LanguageContext::None
                } else {
                    LanguageContext::English
                };
                return Some((total_delete, insert, updated_ctx));
            }
        }
    }

    if flush_ch == '.' && !user_override && !had_pasted_prefix {
        *last_dot_stem = Some(DotStemEntry {
            raw_keys: raw.to_string(),
            visible_word: final_word.to_string(),
        });
    }
    None
}
