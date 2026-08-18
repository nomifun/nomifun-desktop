//! Text search normalization and query expansion — the shared retrieval
//! front-end for note/document search.
//!
//! # Why this module exists
//!
//! Retrieval used to be `content LIKE '%query%'`: ONE contiguous literal
//! substring, matched against a string a model generated non-deterministically.
//! Any inserted space, case change, full-width character, or rephrasing missed
//! a note that demonstrably existed.
//!
//! Swapping the storage layer for FTS5 does NOT by itself fix that — verified
//! empirically. Feeding a model's raw string to FTS5 as one phrase reproduces
//! the bug exactly: `"NomiFun是什么"` matches, `"NomiFun 是什么"` does not. The
//! fix is on the QUERY side: normalize identically at index and query time,
//! then split the query into terms that are OR-combined. The index only makes
//! that efficient and BM25-rankable.
//!
//! # The three channels
//!
//! [`expand_query`] splits one natural-language string into three term sets,
//! because SQLite's trigram tokenizer cannot MATCH anything shorter than three
//! characters (the same constraint `memory_search.rs` documents):
//!
//! - [`NoteQueryTerms::fts`] — 3+ chars, for `MATCH`.
//! - [`NoteQueryTerms::like`] — 2 chars or short ASCII, for a `LIKE` substring
//!   scan; unreachable by trigram MATCH.
//! - [`NoteQueryTerms::bigrams`] — CJK bigrams, a LAST-RESORT rung only. These
//!   are deliberately segregated rather than merged: high-frequency particles
//!   like 「怎么」 match almost any note, so promoting them to a peer channel
//!   trades the recall bug for a precision bug. Callers must consult them only
//!   when the higher rungs return nothing, and must cap the result count.

use unicode_normalization::UnicodeNormalization;

/// Below this many characters a term cannot produce a trigram MATCH.
/// Mirrors `TRIGRAM_MIN_CHARS` in the companion store's memory search.
pub const TRIGRAM_MIN_CHARS: usize = 3;

/// Cap on expanded terms per query. Long sentences produce O(n) overlapping
/// n-grams; without a cap one rambling message becomes a 100-term OR.
pub const MAX_QUERY_TERMS: usize = 24;

/// Interrogative and filler words that appear in almost every question and so
/// carry no selectivity. Dropping them is what turns 「介绍一下 NomiFun」 into
/// the single high-signal term `nomifun`.
const STOP_WORDS: &[&str] = &[
    // Chinese question frames and fillers.
    "是什么",
    "什么",
    "介绍一下",
    "介绍",
    "怎么样",
    "怎么",
    "如何",
    "请问",
    "一下",
    "有没有",
    "可以",
    "哪些",
    "哪个",
    "为什么",
    "能不能",
    // English question frames and articles.
    "what",
    "which",
    "how",
    "why",
    "who",
    "when",
    "where",
    "is",
    "are",
    "the",
    "a",
    "an",
    "of",
    "to",
    "do",
    "does",
    "can",
    "tell",
    "me",
    "about",
];

/// Sentence-final particles. Stripping them recovers the stem: 「免费吗」 keeps
/// both forms so the stem 「免费」 can match 「开源免费」.
const TRAILING_PARTICLES: &[char] = &['吗', '呢', '吧', '啊', '的', '了', '么', '呀', '嘛'];

/// Characters that separate terms — ASCII punctuation plus the full-width and
/// CJK forms a Chinese user actually types. NFKC folds some of these to ASCII,
/// but not 「，。、」, so they are listed explicitly.
const SEPARATORS: &[char] = &[
    ' ', '\t', '\n', '\r', '\u{3000}', ',', '.', '?', '!', ':', ';', '(', ')', '[', ']', '{', '}',
    '<', '>', '"', '\'', '`', '~', '@', '#', '$', '%', '^', '&', '*', '+', '=', '|', '\\', '/', '-',
    '_', '，', '。', '、', '？', '！', '：', '；', '（', '）', '【', '】', '「', '」', '《', '》',
    '…', '·', '～',
];

/// NFKC + lowercase folding. **Index and query sides must both call this** —
/// a mismatch silently loses recall for exactly the inputs it was meant to fix.
///
/// NFKC folds full-width to ASCII (`Ｎ`→`N`, `？`→`?`, `：`→`:`), so a visitor
/// typing 「ＮomiFun是什么」 on a CJK IME reaches the same index terms as
/// `NomiFun`. `to_lowercase` then folds `NOMIFUN`/`NomiFun`/`nomifun` together
/// (SQLite's own `LIKE` is case-insensitive for ASCII only, and its trigram
/// tokenizer does not apply NFKC at all — hence doing it ourselves).
pub fn normalize_for_search(input: &str) -> String {
    input.nfkc().collect::<String>().to_lowercase()
}

/// The three retrieval channels for one expanded query.
///
/// Empty in all three fields means the input carried no searchable signal
/// (blank, pure punctuation, or only stopwords) — callers must treat that as
/// "no query" rather than "match everything".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteQueryTerms {
    /// Terms of 3+ chars, for FTS5 `MATCH`.
    pub fts: Vec<String>,
    /// Sub-trigram terms (2 chars, or 1-2 char ASCII), for a `LIKE` scan.
    pub like: Vec<String>,
    /// CJK bigrams — last-resort rung only. See the module docs.
    pub bigrams: Vec<String>,
}

impl NoteQueryTerms {
    /// True when no channel carries a term, i.e. nothing to search for.
    pub fn is_empty(&self) -> bool {
        self.fts.is_empty() && self.like.is_empty() && self.bigrams.is_empty()
    }

    /// True when a primary (non-last-resort) channel has a term.
    pub fn has_primary(&self) -> bool {
        !self.fts.is_empty() || !self.like.is_empty()
    }
}

/// True for a run of ASCII alphanumerics — the shape latin/digit terms take
/// after [`normalize_for_search`].
fn is_ascii_word(term: &str) -> bool {
    !term.is_empty() && term.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_stop_word(term: &str) -> bool {
    STOP_WORDS.contains(&term)
}

/// Split a normalized string into maximal runs, breaking at separators and at
/// every ASCII/non-ASCII boundary.
///
/// The boundary split is the single most important step: it turns the fused
/// token `nomifun是什么` into `["nomifun", "是什么"]`, so the product name
/// survives as its own term. Every one of the originally reported failures was
/// a variation on that fusion.
fn split_runs(normalized: &str) -> Vec<String> {
    let mut runs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_ascii: Option<bool> = None;

    for ch in normalized.chars() {
        if SEPARATORS.contains(&ch) {
            if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
            current_ascii = None;
            continue;
        }
        let ascii = ch.is_ascii_alphanumeric();
        // Drop anything that is neither a separator nor alphanumeric-or-CJK
        // (stray control characters, emoji, residual symbols).
        if !ascii && ch.is_ascii() {
            if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
            current_ascii = None;
            continue;
        }
        if current_ascii != Some(ascii) && !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
        current_ascii = Some(ascii);
        current.push(ch);
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// Strip a leading `@mention`, which channel messages carry as addressing
/// noise: `"@support NomiFun是什么"` must search for the question, not the
/// bot's handle.
fn strip_leading_mention(normalized: &str) -> &str {
    let trimmed = normalized.trim_start();
    if !trimmed.starts_with('@') {
        return trimmed;
    }
    match trimmed.find(char::is_whitespace) {
        Some(idx) => trimmed[idx..].trim_start(),
        // A bare "@handle" with no following text carries no query.
        None => "",
    }
}

/// Overlapping n-grams of a CJK run.
fn ngrams(run: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = run.chars().collect();
    if chars.len() < n {
        return Vec::new();
    }
    chars.windows(n).map(|w| w.iter().collect()).collect()
}

/// Push `term` unless already present, keeping first-seen order.
fn push_unique(target: &mut Vec<String>, term: String) {
    if !target.contains(&term) {
        target.push(term);
    }
}

/// Expand one natural-language query into the three retrieval channels.
///
/// Accepts either a model-generated query or a visitor's raw message, so the
/// same function backs both the search tool and turn-level pre-retrieval.
///
/// ```
/// use nomifun_common::text_search::expand_query;
/// // The fused product name is split out as its own term.
/// let terms = expand_query("@support  NomiFun 是什么");
/// assert!(terms.fts.contains(&"nomifun".to_owned()));
/// ```
pub fn expand_query(raw: &str) -> NoteQueryTerms {
    let normalized = normalize_for_search(raw);
    let body = strip_leading_mention(&normalized);

    let mut terms = NoteQueryTerms::default();
    for run in split_runs(body) {
        if is_stop_word(&run) {
            continue;
        }
        if is_ascii_word(&run) {
            if run.chars().count() >= TRIGRAM_MIN_CHARS {
                push_unique(&mut terms.fts, run);
            } else {
                push_unique(&mut terms.like, run);
            }
            continue;
        }

        // CJK run: index both the run as typed and its particle-stripped stem,
        // so 「免费吗」 also searches 「免费」.
        let stem: String = run.trim_end_matches(|c| TRAILING_PARTICLES.contains(&c)).to_owned();
        let mut variants = vec![run.clone()];
        if !stem.is_empty() && stem != run {
            variants.push(stem);
        }

        for variant in variants {
            let len = variant.chars().count();
            if len < 2 || is_stop_word(&variant) {
                continue;
            }
            if len < TRIGRAM_MIN_CHARS {
                push_unique(&mut terms.like, variant);
                continue;
            }
            if len == TRIGRAM_MIN_CHARS {
                push_unique(&mut terms.fts, variant.clone());
            } else {
                for gram in ngrams(&variant, TRIGRAM_MIN_CHARS) {
                    if !is_stop_word(&gram) {
                        push_unique(&mut terms.fts, gram);
                    }
                }
            }
            // Bigrams are collected for every CJK run but stay in the
            // last-resort channel; a 2-char overlap is often the only shared
            // signal between a paraphrase and a note (「访客」 in
            // 「访客很生气怎么办」), yet on its own it is too weak to rank.
            for gram in ngrams(&variant, 2) {
                if !is_stop_word(&gram) {
                    push_unique(&mut terms.bigrams, gram);
                }
            }
        }
    }

    // Longest-first truncation: longer terms are more selective, so when a
    // rambling message overflows the cap the discriminating terms survive.
    for channel in [&mut terms.fts, &mut terms.like, &mut terms.bigrams] {
        if channel.len() > MAX_QUERY_TERMS {
            channel.sort_by(|a, b| {
                b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b))
            });
            channel.truncate(MAX_QUERY_TERMS);
        }
    }
    terms
}

/// Quote one term as an FTS5 string literal: wrap in double quotes and double
/// any embedded quote.
///
/// **Mandatory on every term.** Raw text reaching `MATCH` is a syntax-error
/// hazard, not merely a bad-results hazard — verified: `AND`, `a OR`, `"`, `*`,
/// `(`, and `收费吗?` all raise SQLite errors, which would surface as a failed
/// customer-service reply.
pub fn fts_phrase(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Build an OR-combined FTS5 MATCH expression, or `None` when there are no
/// terms (an empty expression is a syntax error).
pub fn fts_match_expression(terms: &[String]) -> Option<String> {
    if terms.is_empty() {
        return None;
    }
    Some(terms.iter().map(|term| fts_phrase(term)).collect::<Vec<_>>().join(" OR "))
}

/// `LIKE` pattern for a substring match, escaping the metacharacters with
/// `ESCAPE '\'` semantics so a term containing `%` or `_` stays literal.
pub fn like_pattern(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    out.push('%');
    for c in term.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fts_of(raw: &str) -> Vec<String> {
        expand_query(raw).fts
    }

    /// NFKC + case folding is what makes the lowercase and full-width variants
    /// of the same question converge on one term.
    #[test]
    fn normalization_folds_case_and_full_width() {
        assert_eq!(normalize_for_search("NomiFun"), "nomifun");
        assert_eq!(normalize_for_search("NOMIFUN"), "nomifun");
        // Full-width latin letter and full-width question mark.
        assert_eq!(normalize_for_search("ＮomiFun？"), "nomifun?");
        assert_eq!(normalize_for_search("ＡＩ"), "ai");
    }

    /// The core fix: a fused CJK+ASCII token splits so the product name
    /// survives as its own term. Every originally reported failure is here.
    #[test]
    fn reported_failures_all_yield_the_product_name() {
        for raw in [
            "@xxx  NomiFun是什么",
            "@xxx  NomiFun 是什么",
            "@xxx  nomifun是什么",
            "@xxx 介绍一下 NomiFun",
            "@xxx ＮomiFun是什么？",
            "@xxx NOMIFUN 能干什么？",
        ] {
            assert!(
                fts_of(raw).contains(&"nomifun".to_owned()),
                "{raw:?} must expand to the bare product name, got {:?}",
                fts_of(raw)
            );
        }
    }

    #[test]
    fn mention_prefix_is_stripped_but_inner_text_survives() {
        let terms = expand_query("@support 怎么安装");
        assert!(!terms.fts.iter().any(|t| t.contains("support")), "{terms:?}");
        assert!(terms.fts.contains(&"怎么安".to_owned()), "{terms:?}");
        // A bare mention with no question carries no searchable signal.
        assert!(expand_query("@support").is_empty());
        assert!(expand_query("@support   ").is_empty());
    }

    #[test]
    fn stopwords_and_particles_are_stripped_to_stems() {
        // 是什么 / 介绍一下 are pure noise and must not become terms.
        let terms = expand_query("介绍一下 NomiFun");
        assert_eq!(terms.fts, vec!["nomifun".to_owned()], "{terms:?}");
        // Particle stripping keeps both the typed form and the stem, so
        // 「免费吗」 can reach a note that says 「开源免费」.
        let terms = expand_query("免费吗");
        assert!(terms.fts.contains(&"免费吗".to_owned()), "{terms:?}");
        assert!(terms.like.contains(&"免费".to_owned()), "{terms:?}");
    }

    #[test]
    fn short_terms_route_to_the_like_channel() {
        // "ai" is 2 chars: below the trigram floor, so MATCH can never find it.
        let terms = expand_query("AI 工作空间");
        assert!(terms.like.contains(&"ai".to_owned()), "{terms:?}");
        assert!(terms.fts.contains(&"工作空".to_owned()), "{terms:?}");
    }

    #[test]
    fn long_cjk_runs_become_overlapping_trigrams() {
        let terms = expand_query("安装包在哪下载");
        for gram in ["安装包", "装包在", "包在哪", "在哪下", "哪下载"] {
            assert!(terms.fts.contains(&gram.to_owned()), "missing {gram}: {terms:?}");
        }
    }

    /// Bigrams must stay segregated: they are the only signal for some
    /// paraphrases, but merging them into `fts` would wreck precision.
    #[test]
    fn bigrams_are_collected_but_kept_out_of_primary_channels() {
        let terms = expand_query("访客很生气怎么办");
        assert!(terms.bigrams.contains(&"访客".to_owned()), "{terms:?}");
        assert!(!terms.fts.contains(&"访客".to_owned()), "{terms:?}");
        assert!(terms.has_primary(), "trigrams still populate the primary channel");
    }

    #[test]
    fn empty_and_noise_only_input_yields_no_terms() {
        for raw in ["", "   ", "\n\t", "？？？", "，。！", "是什么", "what is the"] {
            assert!(expand_query(raw).is_empty(), "{raw:?} must expand to nothing");
        }
    }

    #[test]
    fn term_count_is_capped_longest_first() {
        let long = "这是一个非常冗长的客服问题".repeat(12);
        let terms = expand_query(&long);
        assert!(terms.fts.len() <= MAX_QUERY_TERMS, "{}", terms.fts.len());
        assert!(terms.bigrams.len() <= MAX_QUERY_TERMS, "{}", terms.bigrams.len());
    }

    /// Raw text in a MATCH expression is a syntax-error hazard, so quoting is
    /// not cosmetic. These inputs all raise SQLite errors unquoted.
    #[test]
    fn fts_phrase_quotes_and_escapes_operators() {
        assert_eq!(fts_phrase("AND"), "\"AND\"");
        assert_eq!(fts_phrase("a-b"), "\"a-b\"");
        assert_eq!(fts_phrase("he said \"hi\""), "\"he said \"\"hi\"\"\"");
        assert_eq!(fts_phrase("*"), "\"*\"");
    }

    #[test]
    fn match_expression_is_none_when_no_terms() {
        assert!(fts_match_expression(&[]).is_none());
        let expr = fts_match_expression(&["nomifun".to_owned(), "怎么安".to_owned()]).unwrap();
        assert_eq!(expr, "\"nomifun\" OR \"怎么安\"");
    }

    #[test]
    fn like_pattern_escapes_metacharacters() {
        assert_eq!(like_pattern("abc"), "%abc%");
        assert_eq!(like_pattern("50%"), "%50\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("c:\\x"), "%c:\\\\x%");
    }

    /// FTS5 operator words must never leak through as bare terms — they are
    /// stripped as stopwords or quoted, never interpolated raw.
    #[test]
    fn adversarial_input_does_not_panic_and_produces_safe_terms() {
        for raw in ["@xxx \" OR *", "@xxx AND", "@xxx NEAR(a b)", "@xxx ((("] {
            let terms = expand_query(raw);
            for term in terms.fts.iter().chain(&terms.like).chain(&terms.bigrams) {
                assert!(!term.contains('"'), "{raw:?} leaked a quote: {term:?}");
            }
        }
    }
}
