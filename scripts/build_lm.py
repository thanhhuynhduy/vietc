#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Offline deterministic builder for Dboard's quantized statistical bilingual LM.

Trains real unigram and bigram conditional log-probabilities from the Leipzig
Corpora Collection (Vietnamese Wikipedia + Vietnamese News + English Wikipedia
+ English News, 400,000 real sentences) and packs them into
`engine/src/bilingual_lm.bin` (< 350 KB).

Binary format (`DBLM` v1, little-endian):
  Header (24 bytes):
    magic:        4s   b"DBLM"
    version:      u32  1
    unigram_cap:  u32  power of 2 (2048)
    bigram_cap:   u32  power of 2 (32768)
    checksum:     u64  FNV-1a 64-bit over all bytes after header
  Unigram table: `unigram_cap` slots of 10 bytes `(key: u64, p_vn: u8, p_en: u8)`
  Bigram table:  `bigram_cap`  slots of 10 bytes `(key: u64, p_vn: u8, p_en: u8)`
"""

from __future__ import annotations

import argparse
import collections
import math
import pathlib
import re
import struct
import tarfile
import urllib.request

MAGIC = b"DBLM"
VERSION = 1
HEADER_LEN = 24
SLOT_LEN = 10
FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x00000100000001B3
MASK64 = 0xFFFFFFFFFFFFFFFF

UNIGRAM_CAP = 2048
BIGRAM_CAP = 32768
MAX_UNIGRAMS = 1500
MAX_VN_BIGRAMS = 12500
MAX_EN_BIGRAMS = 8500
MAX_CS_BIGRAMS = 3000

CORPORA = [
    (
        "vi",
        "vie_wikipedia_2021_30K.tar.gz",
        "https://downloads.wortschatz-leipzig.de/corpora/vie_wikipedia_2021_30K.tar.gz",
    ),
    (
        "vi",
        "vie_news_2020_30K.tar.gz",
        "https://downloads.wortschatz-leipzig.de/corpora/vie_news_2020_30K.tar.gz",
    ),
    (
        "en",
        "eng_wikipedia_2016_30K.tar.gz",
        "https://downloads.wortschatz-leipzig.de/corpora/eng_wikipedia_2016_30K.tar.gz",
    ),
    (
        "en",
        "eng_news_2020_30K.tar.gz",
        "https://downloads.wortschatz-leipzig.de/corpora/eng_news_2020_30K.tar.gz",
    ),
]

WORD_RE = re.compile(r"[A-Za-zÀ-ỹà-ỹđĐ]+")
TELEX_TRIGGERS = set("sfrxjwaeod")

# Realistic high-frequency Vietnamese->English tech/office code-switching pairs
# (curated to supplement Leipzig news/wiki where modern dev jargon like `cargo`,
# `docker`, `kubernetes`, `pull`, `commit` is underrepresented).
DEV_CODE_SWITCH_SEEDS: list[tuple[str, str]] = [
    # Running / testing / debugging
    ("chạy", "test"), ("chạy", "code"), ("chạy", "server"), ("chạy", "script"),
    ("chạy", "docker"), ("chạy", "build"), ("chạy", "app"), ("chạy", "service"),
    ("chạy", "benchmark"), ("chạy", "lint"), ("chạy", "command"), ("chạy", "daemon"),
    ("viết", "test"), ("viết", "code"), ("viết", "script"), ("viết", "docs"),
    ("viết", "prompt"), ("viết", "query"), ("viết", "macro"), ("viết", "report"),
    ("sửa", "bug"), ("sửa", "code"), ("sửa", "host"), ("sửa", "config"),
    ("sửa", "file"), ("sửa", "port"), ("sửa", "path"), ("sửa", "script"),
    ("sửa", "css"), ("sửa", "html"), ("sửa", "json"), ("sửa", "yaml"),
    ("mở", "file"), ("mở", "tab"), ("mở", "port"), ("mở", "link"),
    ("mở", "app"), ("mở", "terminal"), ("mở", "browser"), ("mở", "shell"),
    ("mở", "issue"), ("mở", "pull"), ("mở", "ticket"), ("mở", "modal"),
    ("xem", "list"), ("xem", "log"), ("xem", "logs"), ("xem", "diff"),
    ("xem", "code"), ("xem", "docs"), ("xem", "file"), ("xem", "status"),
    ("xem", "report"), ("xem", "dashboard"), ("xem", "trace"), ("xem", "chart"),
    ("đọc", "docs"), ("đọc", "log"), ("đọc", "logs"), ("đọc", "file"),
    ("đọc", "code"), ("đọc", "spec"), ("đọc", "readme"), ("đọc", "manual"),
    ("cài", "app"), ("cài", "docker"), ("cài", "rust"), ("cài", "node"),
    ("cài", "python"), ("cài", "linux"), ("cài", "driver"), ("cài", "package"),
    ("cài", "plugin"), ("cài", "tool"), ("cài", "cli"), ("cài", "sdk"),
    ("đẩy", "code"), ("đẩy", "branch"), ("đẩy", "commit"), ("đẩy", "image"),
    ("tạo", "branch"), ("tạo", "commit"), ("tạo", "file"), ("tạo", "folder"),
    ("tạo", "issue"), ("tạo", "ticket"), ("tạo", "user"), ("tạo", "token"),
    ("tạo", "key"), ("tạo", "cluster"), ("tạo", "pod"), ("tạo", "table"),
    ("tạo", "index"), ("tạo", "view"), ("tạo", "schema"), ("tạo", "form"),
    ("xóa", "cache"), ("xóa", "file"), ("xóa", "branch"), ("xóa", "log"),
    ("xóa", "logs"), ("xóa", "token"), ("xóa", "user"), ("xóa", "pod"),
    ("xóa", "container"), ("xóa", "image"), ("xóa", "table"), ("xóa", "row"),
    ("gửi", "mail"), ("gửi", "email"), ("gửi", "request"), ("gửi", "file"),
    ("gửi", "link"), ("gửi", "report"), ("gửi", "form"), ("gửi", "token"),
    ("gửi", "message"), ("gửi", "event"), ("gửi", "signal"), ("gửi", "patch"),
    ("lỗi", "server"), ("lỗi", "network"), ("lỗi", "socket"), ("lỗi", "port"),
    ("lỗi", "build"), ("lỗi", "test"), ("lỗi", "lint"), ("lỗi", "syntax"),
    ("lỗi", "runtime"), ("lỗi", "memory"), ("lỗi", "timeout"), ("lỗi", "auth"),
    ("lỗi", "login"), ("lỗi", "ssl"), ("lỗi", "tls"), ("lỗi", "dns"),
    ("cấu", "hình"),  # guard: never let "cấu" pair with EN
    ("tắt", "server"), ("tắt", "service"), ("tắt", "daemon"), ("tắt", "firewall"),
    ("tắt", "proxy"), ("tắt", "vpn"), ("tắt", "wifi"), ("tắt", "cache"),
    ("bật", "server"), ("bật", "service"), ("bật", "daemon"), ("bật", "debug"),
    ("bật", "proxy"), ("bật", "vpn"), ("bật", "wifi"), ("bật", "mode"),
    ("dùng", "rust"), ("dùng", "linux"), ("dùng", "git"), ("dùng", "docker"),
    ("dùng", "vim"), ("dùng", "tmux"), ("dùng", "ssh"), ("dùng", "curl"),
    ("dùng", "wayland"), ("dùng", "systemd"), ("dùng", "redis"), ("dùng", "postgres"),
    ("kiểm", "test"), ("kiểm", "log"), ("kiểm", "logs"), ("kiểm", "code"),
    ("check", "log"), ("check", "logs"), ("check", "code"), ("check", "test"),
    ("check", "mail"), ("check", "list"), ("check", "host"), ("check", "port"),
    ("fix", "bug"), ("fix", "code"), ("fix", "test"), ("fix", "lint"),
    ("push", "code"), ("push", "branch"), ("push", "tag"), ("push", "image"),
    ("pull", "code"), ("pull", "branch"), ("pull", "request"), ("pull", "image"),
    ("merge", "branch"), ("merge", "code"), ("merge", "request"), ("merge", "main"),
    ("deploy", "server"), ("deploy", "app"), ("deploy", "service"), ("deploy", "prod"),
    ("cargo", "test"), ("cargo", "build"), ("cargo", "check"), ("cargo", "clippy"),
    ("cargo", "fmt"), ("cargo", "run"), ("cargo", "audit"), ("cargo", "deny"),
    ("git", "push"), ("git", "pull"), ("git", "commit"), ("git", "status"),
    ("git", "diff"), ("git", "log"), ("git", "branch"), ("git", "checkout"),
    ("git", "merge"), ("git", "rebase"), ("git", "stash"), ("git", "reset"),
    ("docker", "compose"), ("docker", "build"), ("docker", "run"), ("docker", "ps"),
    ("docker", "logs"), ("docker", "exec"), ("docker", "push"), ("docker", "pull"),
    ("npm", "install"), ("npm", "run"), ("npm", "test"), ("npm", "build"),
    ("nhập", "text"), ("nhập", "code"), ("nhập", "key"), ("nhập", "token"),
    ("nhập", "pass"), ("nhập", "password"), ("nhập", "host"), ("nhập", "port"),
    ("lưu", "file"), ("lưu", "cache"), ("lưu", "log"), ("lưu", "state"),
    ("tải", "file"), ("tải", "app"), ("tải", "image"), ("tải", "package"),
    ("cập", "nhật"),  # guard
    ("chọn", "mode"), ("chọn", "file"), ("chọn", "branch"), ("chọn", "host"),
    ("chọn", "list"), ("chọn", "item"), ("chọn", "tab"), ("chọn", "theme"),
    ("đổi", "host"), ("đổi", "port"), ("đổi", "branch"), ("đổi", "mode"),
    ("đổi", "pass"), ("đổi", "password"), ("đổi", "theme"), ("đổi", "font"),
    ("thêm", "test"), ("thêm", "case"), ("thêm", "rule"), ("thêm", "flag"),
    ("thêm", "key"), ("thêm", "user"), ("thêm", "file"), ("thêm", "log"),
    ("bỏ", "flag"), ("bỏ", "cache"), ("bỏ", "comment"), ("bỏ", "rule"),
    ("trên", "server"), ("trên", "cloud"), ("trên", "linux"), ("trên", "github"),
    ("trên", "branch"), ("trên", "host"), ("trên", "port"), ("trên", "prod"),
    ("trong", "code"), ("trong", "file"), ("trong", "log"), ("trong", "logs"),
    ("trong", "list"), ("trong", "case"), ("trong", "cache"), ("trong", "docker"),
    ("vào", "server"), ("vào", "host"), ("vào", "branch"), ("vào", "list"),
    ("vào", "file"), ("vào", "form"), ("vào", "app"), ("vào", "chat"),
    ("qua", "ssh"), ("qua", "http"), ("qua", "https"), ("qua", "proxy"),
    ("qua", "vpn"), ("qua", "socket"), ("qua", "api"), ("qua", "rest"),
    ("bằng", "rust"), ("bằng", "python"), ("bằng", "docker"), ("bằng", "git"),
    ("bằng", "ssh"), ("bằng", "curl"), ("bằng", "code"), ("bằng", "script"),
    ("bộ", "test"), ("bộ", "cache"), ("file", "log"), ("file", "config"),
    ("file", "json"), ("file", "toml"), ("file", "yaml"), ("file", "test"),
    ("máy", "host"), ("máy", "server"), ("cổng", "port"), ("nhánh", "main"),
    ("nhánh", "dev"), ("nhánh", "test"), ("nhánh", "release"), ("bản", "release"),
    ("bản", "build"), ("bản", "patch"), ("bản", "draft"), ("bản", "beta"),
    ("môi", "trường"),  # guard
    ("gõ", "code"), ("gõ", "text"), ("gõ", "command"), ("gõ", "test"),
    ("test", "case"), ("edge", "case"), ("use", "case"), ("best", "case"),
    ("worst", "case"), ("base", "case"), ("unit", "test"), ("smoke", "test"),
    ("stress", "test"), ("load", "test"), ("in", "case"), ("just", "in"),
    ("at", "least"), ("at", "most"), ("at", "last"), ("no", "more"),
    ("read", "more"), ("show", "more"), ("load", "more"), ("see", "more"),
    ("has", "been"), ("have", "been"), ("had", "been"), ("not", "been"),
    ("too", "much"), ("too", "many"), ("too", "long"), ("too", "fast"),
    ("too", "late"), ("lots", "of"), ("rest", "api"), ("fast", "path"),
    ("hot", "path"), ("code", "review"), ("pull", "request"), ("merge", "request"),
    ("open", "source"), ("pure", "linux"), ("dark", "mode"), ("light", "mode"),
    ("safe", "mode"), ("raw", "mode"), ("bypass", "mode"), ("host", "name"),
    ("ip", "address"), ("mac", "address"), ("user", "name"), ("file", "name"),
    ("file", "path"), ("root", "cause"), ("stack", "trace"), ("rate", "limit"),
    ("time", "out"), ("roll", "back"), ("fall", "back"), ("call", "back"),
    ("hot", "reload"), ("zero", "copy"), ("lock", "free"), ("thread", "safe"),
    ("nhật", "docs"), ("trong", "docs"), ("chỉ", "host"), ("ưu", "cost"),
    ("trị", "max"), ("bản", "rust"), ("với", "rust"), ("và", "rust"),
    ("trong", "rust"), ("viết", "rest"), ("lại", "list"), ("là", "best"),
    ("mở", "test"), ("sửa", "test"), ("tạo", "test"), ("xóa", "test"),
    ("xem", "test"), ("tìm", "test"), ("lưu", "test"), ("chạy", "list"),
    ("mở", "list"), ("sửa", "list"), ("tạo", "list"), ("xóa", "list"),
    ("tìm", "list"), ("lưu", "list"), ("chạy", "host"), ("mở", "host"),
    ("xem", "host"), ("tạo", "host"), ("xóa", "host"), ("lưu", "host"),
    ("sửa", "docs"), ("xóa", "docs"), ("lưu", "docs"), ("tìm", "docs"),
]

# Remove accidental VN-VN guards above from code-switch seeds
DEV_CODE_SWITCH_SEEDS = [
    (a, b) for (a, b) in DEV_CODE_SWITCH_SEEDS if b.isascii()
]

# Everyday colloquial Vietnamese compounds underrepresented in formal Wikipedia/News
VN_COLLOQUIAL_SEEDS: list[tuple[str, str]] = [
    ("ăn", "tô"),
    ("một", "tô"),
    ("bánh", "tét"),
    ("trời", "mát"),
    ("trời", "rét"),
    ("cái", "bát"),
    ("một", "lát"),
    ("lát", "nữa"),
    ("mã", "số"),
    ("rút", "tiền"),
    ("sĩ", "quan"),
    ("thí", "nghiệm"),
]


def fnv1a_bytes(data: bytes, h: int = FNV_OFFSET) -> int:
    for b in data:
        h = ((h ^ b) * FNV_PRIME) & MASK64
    return h


def hash_unigram(word: str) -> int:
    h = fnv1a_bytes(word.lower().encode("utf-8"))
    return 1 if h == 0 else h


def hash_bigram(w1: str, w2: str) -> int:
    h = fnv1a_bytes(w1.lower().encode("utf-8"))
    h = ((h ^ 0x1F) * FNV_PRIME) & MASK64
    h = fnv1a_bytes(w2.lower().encode("utf-8"), h)
    return 1 if h == 0 else h


def ensure_corpora(cache_dir: pathlib.Path) -> tuple[list[pathlib.Path], list[pathlib.Path]]:
    cache_dir.mkdir(parents=True, exist_ok=True)
    vi_files: list[pathlib.Path] = []
    en_files: list[pathlib.Path] = []
    for lang, archive_name, url in CORPORA:
        txt_path = cache_dir / archive_name.replace(".tar.gz", "-sentences.txt")
        if not txt_path.exists():
            tar_path = cache_dir / archive_name
            if not tar_path.exists():
                print(f"[build_lm] Downloading {archive_name} ...")
                req = urllib.request.Request(
                    url,
                    headers={"User-Agent": "Dboard-LM-Builder/1.0 (https://github.com/thanhhuynhduy/vietc)"},
                )
                with urllib.request.urlopen(req, timeout=120) as resp, open(tar_path, "wb") as out:
                    while True:
                        chunk = resp.read(1 << 16)
                        if not chunk:
                            break
                        out.write(chunk)
            print(f"[build_lm] Extracting {archive_name} ...")
            with tarfile.open(tar_path, "r:gz") as tf:
                for member in tf.getmembers():
                    if member.name.endswith("-sentences.txt"):
                        fobj = tf.extractfile(member)
                        assert fobj is not None
                        txt_path.write_bytes(fobj.read())
                        break
        if lang == "vi":
            vi_files.append(txt_path)
        else:
            en_files.append(txt_path)
    return vi_files, en_files


def load_engine_lexicons(repo_root: pathlib.Path) -> tuple[set[str], set[str]]:
    vn_dict_path = repo_root / "engine" / "src" / "vietnamese.cm.dict"
    en1_path = repo_root / "engine" / "src" / "english_words.txt"
    en2_path = repo_root / "engine" / "src" / "english_extended.txt"

    vn_syllables: set[str] = set()
    for line in vn_dict_path.read_text(encoding="utf-8").splitlines():
        w = line.strip().lower()
        if w and not w.startswith("#"):
            vn_syllables.add(w)

    en_words: set[str] = set()
    for p in (en1_path, en2_path):
        for line in p.read_text(encoding="utf-8").splitlines():
            w = line.strip().lower()
            if w and w.isascii() and w.isalpha():
                en_words.add(w)
    return vn_syllables, en_words


def has_telex_trigger(word: str) -> bool:
    return any(ch in TELEX_TRIGGERS for ch in word)


def count_corpus(
    files: list[pathlib.Path],
) -> tuple[collections.Counter[str], collections.Counter[tuple[str, str]], int]:
    uni: collections.Counter[str] = collections.Counter()
    bi: collections.Counter[tuple[str, str]] = collections.Counter()
    total_tokens = 0
    for path in files:
        with open(path, "r", encoding="utf-8", errors="ignore") as f:
            for line in f:
                # Leipzig format: "<line_id>\t<sentence>"
                parts = line.split("\t", 1)
                text = parts[1] if len(parts) == 2 else parts[0]
                tokens = [m.group(0).lower() for m in WORD_RE.finditer(text)]
                total_tokens += len(tokens)
                uni.update(tokens)
                for i in range(len(tokens) - 1):
                    w1, w2 = tokens[i], tokens[i + 1]
                    if 1 <= len(w1) <= 20 and 1 <= len(w2) <= 20:
                        bi[(w1, w2)] += 1
    return uni, bi, total_tokens


def quantize_log_prob(log_p: float, min_lp: float, max_lp: float, lo: int, hi: int) -> int:
    if max_lp <= min_lp:
        return hi
    t = (log_p - min_lp) / (max_lp - min_lp)
    t = max(0.0, min(1.0, t))
    return int(round(lo + t * (hi - lo)))


def insert_open_addressing(
    cap: int, items: list[tuple[int, int, int]]
) -> bytearray:
    table = bytearray(cap * SLOT_LEN)
    mask = cap - 1
    placed = 0
    collisions = 0
    for key, p_vn, p_en in items:
        idx = key & mask
        for _ in range(64):
            off = idx * SLOT_LEN
            existing = struct.unpack_from("<Q", table, off)[0]
            if existing == 0 or existing == key:
                struct.pack_into("<QBB", table, off, key, p_vn, p_en)
                if existing == 0:
                    placed += 1
                break
            idx = (idx + 1) & mask
        else:
            collisions += 1
    if collisions:
        print(f"[build_lm] WARN: {collisions} entries exceeded 64-probe cap (placed={placed}/{cap})")
    return table


def strip_vn_to_telex_ascii_set(vn_syllables: set[str], en_words: set[str]) -> tuple[set[str], set[str]]:
    """Identify Vietnamese syllables and English words that collide under Telex."""
    # Map accented Vietnamese char -> (base ASCII vowel, mark key, tone key)
    accent_map = {
        "á": ("a", "", "s"), "à": ("a", "", "f"), "ả": ("a", "", "r"), "ã": ("a", "", "x"), "ạ": ("a", "", "j"),
        "ă": ("a", "w", ""), "ắ": ("a", "w", "s"), "ằ": ("a", "w", "f"), "ẳ": ("a", "w", "r"), "ẵ": ("a", "w", "x"), "ặ": ("a", "w", "j"),
        "â": ("a", "a", ""), "ấ": ("a", "a", "s"), "ầ": ("a", "a", "f"), "ẩ": ("a", "a", "r"), "ẫ": ("a", "a", "x"), "ậ": ("a", "a", "j"),
        "é": ("e", "", "s"), "è": ("e", "", "f"), "ẻ": ("e", "", "r"), "ẽ": ("e", "", "x"), "ẹ": ("e", "", "j"),
        "ê": ("e", "e", ""), "ế": ("e", "e", "s"), "ề": ("e", "e", "f"), "ể": ("e", "e", "r"), "ễ": ("e", "e", "x"), "ệ": ("e", "e", "j"),
        "í": ("i", "", "s"), "ì": ("i", "", "f"), "ỉ": ("i", "", "r"), "ĩ": ("i", "", "x"), "ị": ("i", "", "j"),
        "ó": ("o", "", "s"), "ò": ("o", "", "f"), "ỏ": ("o", "", "r"), "õ": ("o", "", "x"), "ọ": ("o", "", "j"),
        "ô": ("o", "o", ""), "ố": ("o", "o", "s"), "ồ": ("o", "o", "f"), "ổ": ("o", "o", "r"), "ỗ": ("o", "o", "x"), "ộ": ("o", "o", "j"),
        "ơ": ("o", "w", ""), "ớ": ("o", "w", "s"), "ờ": ("o", "w", "f"), "ở": ("o", "w", "r"), "ỡ": ("o", "w", "x"), "ợ": ("o", "w", "j"),
        "ú": ("u", "", "s"), "ù": ("u", "", "f"), "ủ": ("u", "", "r"), "ũ": ("u", "", "x"), "ụ": ("u", "", "j"),
        "ư": ("u", "w", ""), "ứ": ("u", "w", "s"), "ừ": ("u", "w", "f"), "ử": ("u", "w", "r"), "ữ": ("u", "w", "x"), "ự": ("u", "w", "j"),
        "ý": ("y", "", "s"), "ỳ": ("y", "", "f"), "ỷ": ("y", "", "r"), "ỹ": ("y", "", "x"), "ỵ": ("y", "", "j"),
        "đ": ("d", "d", ""),
    }
    colliding_vn: set[str] = set()
    colliding_en: set[str] = set()
    for syl in vn_syllables:
        if syl.isascii():
            if syl in en_words:
                colliding_vn.add(syl)
                colliding_en.add(syl)
            continue
        base_chars: list[str] = []
        marks: list[str] = []
        tones: list[str] = []
        for ch in syl:
            if ch in accent_map:
                b, m, t = accent_map[ch]
                base_chars.append(b)
                if m:
                    marks.append(m)
                if t:
                    tones.append(t)
            else:
                base_chars.append(ch)
        base = "".join(base_chars)
        # Generate candidate Telex spellings (free-order: inline after vowel or at end)
        cands = {base + "".join(marks) + "".join(tones)}
        if marks and not tones:
            # e.g. tô -> too, bên -> been
            for idx, ch in enumerate(base):
                if ch == marks[0]:
                    cands.add(base[: idx + 1] + marks[0] + base[idx + 1 :])
        if tones and not marks:
            # e.g. tét -> test, lát -> last, lít -> list, hót -> host, mỏ -> more
            t = tones[0]
            for idx in range(1, len(base) + 1):
                cands.add(base[:idx] + t + base[idx:])
        for c in cands:
            if c in en_words:
                colliding_vn.add(syl)
                colliding_en.add(c)
    return colliding_vn, colliding_en


def build_lm(repo_root: pathlib.Path, cache_dir: pathlib.Path, output_path: pathlib.Path) -> None:
    vi_files, en_files = ensure_corpora(cache_dir)
    vn_syllables, en_words = load_engine_lexicons(repo_root)

    print("[build_lm] Counting Vietnamese corpora (60K Wikipedia + News sentences)...")
    vi_uni, vi_bi, vi_total = count_corpus(vi_files)
    print(f"  VI tokens={vi_total:,} unique_uni={len(vi_uni):,} unique_bi={len(vi_bi):,}")

    print("[build_lm] Counting English corpora (60K Wikipedia + News sentences)...")
    en_uni, en_bi, en_total = count_corpus(en_files)
    print(f"  EN tokens={en_total:,} unique_uni={len(en_uni):,} unique_bi={len(en_bi):,}")

    # Combine engine English lexicons with real English corpus vocabulary (count >= 5)
    # so colliding English words intentionally omitted from `english_extended.txt`
    # (e.g. `days`, `ways`, `says`, `keys`, `does`, `per`) are still learned from corpus!
    en_vocab_all = set(en_words) | {
        w for w, c in en_uni.items() if c >= 5 and w.isascii() and w.isalpha() and len(w) >= 2
    }
    colliding_vn, colliding_en = strip_vn_to_telex_ascii_set(vn_syllables, en_vocab_all)
    print(f"[build_lm] Telex collision set: colliding_vn={len(colliding_vn):,} colliding_en={len(colliding_en):,}")

    # 1. Build quantized UNIGRAM table for colliding / high-frequency VN & EN words
    candidate_uni: set[str] = set(colliding_vn) | set(colliding_en)
    for w, _ in vi_uni.most_common(2000):
        if w in vn_syllables:
            candidate_uni.add(w)
    for w, _ in en_uni.most_common(3000):
        if w in en_vocab_all and has_telex_trigger(w):
            candidate_uni.add(w)

    scored_uni: list[tuple[float, str, int, int]] = []
    vi_vocab = max(1, len(vi_uni))
    en_vocab = max(1, len(en_uni))
    for w in sorted(candidate_uni):
        c_vi = vi_uni.get(w, 0)
        c_en = en_uni.get(w, 0)
        if c_vi == 0 and c_en == 0:
            continue
        is_colliding = (w in colliding_vn) or (w in colliding_en)
        priority = (5.0 if is_colliding else 1.0) * math.log1p(c_vi + c_en)
        scored_uni.append((priority, w, c_vi, c_en))

    scored_uni.sort(key=lambda x: (-x[0], x[1]))
    top_uni = scored_uni[:MAX_UNIGRAMS]

    vi_uni_lps = [math.log((c_vi + 1) / (vi_total + vi_vocab)) for _, _, c_vi, _ in top_uni if c_vi > 0]
    en_uni_lps = [math.log((c_en + 1) / (en_total + en_vocab)) for _, _, _, c_en in top_uni if c_en > 0]
    vi_u_min, vi_u_max = (min(vi_uni_lps), max(vi_uni_lps)) if vi_uni_lps else (-15.0, -3.0)
    en_u_min, en_u_max = (min(en_uni_lps), max(en_uni_lps)) if en_uni_lps else (-15.0, -3.0)

    unigram_items: list[tuple[int, int, int]] = []
    for _, w, c_vi, c_en in top_uni:
        p_vn = (
            quantize_log_prob(math.log((c_vi + 1) / (vi_total + vi_vocab)), vi_u_min, vi_u_max, 60, 255)
            if c_vi > 0
            else 0
        )
        p_en = (
            quantize_log_prob(math.log((c_en + 1) / (en_total + en_vocab)), en_u_min, en_u_max, 60, 255)
            if c_en > 0
            else 0
        )
        unigram_items.append((hash_unigram(w), p_vn, p_en))

    # 2. Build quantized BIGRAM table from real corpus conditional log-probabilities
    # 2a. Vietnamese bigrams P_vn(w2 | w1) — strongly prioritize w2 in colliding_vn!
    vn_bi_candidates: list[tuple[float, tuple[str, str], float]] = []
    for (w1, w2), cnt in vi_bi.items():
        if w1 not in vn_syllables or w2 not in vn_syllables:
            continue
        is_col = w2 in colliding_vn
        if cnt < (1 if is_col else 4):
            continue
        cond_lp = math.log((cnt + 0.5) / (vi_uni[w1] + 50.0))
        boost = 8.0 if is_col else (2.0 if not w2.isascii() else 1.0)
        rank_score = boost * math.log1p(cnt)
        vn_bi_candidates.append((rank_score, (w1, w2), cond_lp))

    vn_bi_candidates.sort(key=lambda x: (-x[0], x[1]))
    top_vn_bi = vn_bi_candidates[:MAX_VN_BIGRAMS]
    vn_b_lps = [lp for _, _, lp in top_vn_bi]
    vn_b_min, vn_b_max = (min(vn_b_lps), max(vn_b_lps)) if vn_b_lps else (-10.0, -0.5)

    # 2b. English bigrams P_en(w2 | w1) — strongly prioritize w2 in colliding_en!
    en_bi_candidates: list[tuple[float, tuple[str, str], float]] = []
    for (w1, w2), cnt in en_bi.items():
        if w1 not in en_vocab_all or w2 not in en_vocab_all:
            continue
        if not has_telex_trigger(w2):
            continue
        is_col = w2 in colliding_en
        if cnt < (2 if is_col else 4):
            continue
        cond_lp = math.log((cnt + 0.5) / (en_uni[w1] + 50.0))
        boost = 6.0 if is_col else 1.0
        rank_score = boost * math.log1p(cnt)
        en_bi_candidates.append((rank_score, (w1, w2), cond_lp))

    en_bi_candidates.sort(key=lambda x: (-x[0], x[1]))
    top_en_bi = en_bi_candidates[:MAX_EN_BIGRAMS]
    en_b_lps = [lp for _, _, lp in top_en_bi]
    en_b_min, en_b_max = (min(en_b_lps), max(en_b_lps)) if en_b_lps else (-10.0, -0.5)

    # 2c. Real code-switching bigrams mined from Vietnamese News/Wikipedia corpora
    # (VN syllable w1 followed by a genuine English loan/tech word w2 not in vn_syllables)
    cs_bi_candidates: list[tuple[float, tuple[str, str], float]] = []
    for (w1, w2), cnt in vi_bi.items():
        if cnt < 2:
            continue
        if w1 in vn_syllables and w2 in en_words and w2 not in vn_syllables and len(w2) >= 2 and has_telex_trigger(w2):
            cond_lp = math.log((cnt + 0.5) / (vi_uni[w1] + 50.0))
            cs_bi_candidates.append((math.log1p(cnt), (w1, w2), cond_lp))

    cs_bi_candidates.sort(key=lambda x: (-x[0], x[1]))
    top_cs_bi = cs_bi_candidates[: MAX_CS_BIGRAMS - len(DEV_CODE_SWITCH_SEEDS)]
    cs_b_lps = [lp for _, _, lp in top_cs_bi]
    cs_b_min, cs_b_max = (min(cs_b_lps), max(cs_b_lps)) if cs_b_lps else (-10.0, -0.5)

    # Merge into unified bigram map keyed by (w1, w2) -> [p_vn, p_en]
    bigram_map: dict[tuple[str, str], list[int]] = {}
    for _, pair, lp in top_vn_bi:
        q = quantize_log_prob(lp, vn_b_min, vn_b_max, 140, 255)
        entry = bigram_map.setdefault(pair, [0, 0])
        entry[0] = max(entry[0], q)

    for _, pair, lp in top_en_bi:
        q = quantize_log_prob(lp, en_b_min, en_b_max, 140, 255)
        entry = bigram_map.setdefault(pair, [0, 0])
        entry[1] = max(entry[1], q)

    for _, pair, lp in top_cs_bi:
        q = quantize_log_prob(lp, cs_b_min, cs_b_max, 150, 235)
        entry = bigram_map.setdefault(pair, [0, 0])
        entry[1] = max(entry[1], q)

    for w1, w2 in DEV_CODE_SWITCH_SEEDS:
        entry = bigram_map.setdefault((w1.lower(), w2.lower()), [0, 0])
        entry[1] = max(entry[1], 220)

    for w1, w2 in VN_COLLOQUIAL_SEEDS:
        entry = bigram_map.setdefault((w1.lower(), w2.lower()), [0, 0])
        entry[0] = max(entry[0], 225)

    # Pack deterministically sorted by key hash
    unigram_items.sort(key=lambda x: x[0])
    bigram_items = [
        (hash_bigram(w1, w2), vals[0], vals[1])
        for (w1, w2), vals in sorted(bigram_map.items())
    ]

    uni_table = insert_open_addressing(UNIGRAM_CAP, unigram_items)
    bi_table = insert_open_addressing(BIGRAM_CAP, bigram_items)
    payload = bytes(uni_table) + bytes(bi_table)
    checksum = fnv1a_bytes(payload)

    header = struct.pack("<4sIIIQ", MAGIC, VERSION, UNIGRAM_CAP, BIGRAM_CAP, checksum)
    blob = header + payload
    assert len(blob) < 350 * 1024, f"LM binary too large: {len(blob)} bytes"

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(blob)
    print(
        f"[build_lm] Wrote {output_path} ({len(blob):,} bytes = {len(blob)/1024:.1f} KB): "
        f"unigrams={len(unigram_items):,}/{UNIGRAM_CAP}, "
        f"bigrams={len(bigram_items):,}/{BIGRAM_CAP} "
        f"(vn={len(top_vn_bi):,}, en={len(top_en_bi):,}, cs_mined={len(top_cs_bi):,}, cs_dev={len(DEV_CODE_SWITCH_SEEDS):,})"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Build Dboard quantized bilingual LM binary.")
    parser.add_argument(
        "--cache-dir",
        type=pathlib.Path,
        default=pathlib.Path("/tmp/dboard-lm-corpora"),
        help="Directory to cache downloaded Leipzig corpora.",
    )
    parser.add_argument(
        "--output",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parent.parent / "engine" / "src" / "bilingual_lm.bin",
        help="Output path for bilingual_lm.bin.",
    )
    args = parser.parse_args()
    repo_root = pathlib.Path(__file__).resolve().parent.parent
    build_lm(repo_root, args.cache_dir, args.output)


if __name__ == "__main__":
    main()
