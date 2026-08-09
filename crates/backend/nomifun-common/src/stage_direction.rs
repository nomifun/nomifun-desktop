//! Stage-direction removal: a content guard, not an emotion channel.
//!
//! The robot system prompt used to ask the model to lead every sentence with an
//! `[emotion:name]` marker, and two rounds of code were built on that syntax.
//! The model emitted `[winking]` instead — the bare name — so every stripper
//! keyed on the literal `"[emotion:"` matched nothing: the marker was read aloud
//! by TTS, printed in the desktop transcript, and drove no face at all. A syntax
//! contract with an LLM is not enforceable, so the contract is gone: the prompt
//! now FORBIDS brackets, stage directions, emoji and markdown outright, and no
//! marker channel exists in either direction.
//!
//! A prohibition is far more enforceable than a syntax contract — but it is
//! still not a guarantee, and the requirement is absolute: 要么展示正常内容，要么
//! 别展示. So this module is the backstop that implements it, and it is
//! deliberately **format-agnostic**: it knows nothing about emotion names,
//! nothing about an `emotion:` prefix, and nothing about any vocabulary. It only
//! knows the *shape* of a stage direction — a short bracketed run of ASCII
//! word characters — which is what `[winking]`, `[laughs]`, `[emotion:happy]`
//! and `[pause]` all share and what `[1]`, `[2026]` and `[附录2]` all lack.
//!
//! It lives in `nomifun-common` because two independent readers of the same
//! `AgentStreamEvent` stream need it: the desktop relay (`stream_relay.rs`, for
//! the live stream and the persisted `messages` row) and the device path
//! (`nomifun-robot`'s `sanitize_for_speech` / `sanitize_for_display`, for TTS
//! and the OLED). Neither may depend on the other's crate. Only this guard is
//! shared; the device sanitisers also drop emoji and collapse whitespace, which
//! stays device-only because it would be data loss in a desktop transcript.

/// The delimiter pairs a stage direction arrives in. The full-width CJK pair is
/// listed because a Chinese-speaking model reaches for `【…】` as readily as for
/// `[…]`, and the prompt forbids both.
const DELIMITERS: [(char, char); 2] = [('[', ']'), ('【', '】')];

/// Longest inner text, in bytes, that can still read as a stage direction.
///
/// Real bracketed content in a transcript is either short and numeric (`[1]`,
/// `[2026]`) or long and prosaic; a stage direction is short and wordy. 24 bytes
/// admits `[smiling softly]` and `[emotion: happy]` while a genuine bracketed
/// clause outruns it.
const MAX_INNER_BYTES: usize = 24;

/// Upper bound on what [`StageDirectionFilter`] withholds: the widest opener
/// (`【`, 3 bytes) plus the longest inner text that can still qualify. This is
/// what bounds the filter's latency and its memory.
const MAX_WITHHELD_BYTES: usize = 3 + MAX_INNER_BYTES;

/// The closing delimiter for `open`, or `None` when `open` opens nothing.
fn closer_for(open: char) -> Option<char> {
    DELIMITERS
        .iter()
        .find(|(opener, _)| *opener == open)
        .map(|(_, closer)| *closer)
}

/// Byte offset of the next opening delimiter, with the closer it expects.
fn next_opener(text: &str) -> Option<(usize, char, char)> {
    text.char_indices()
        .find_map(|(at, ch)| closer_for(ch).map(|closer| (at, ch, closer)))
}

/// Does `inner` (the text between a delimiter pair) read as a stage direction?
///
/// Three conditions, and all three earn their keep:
/// - **short** — a long bracketed run is prose, not an annotation;
/// - **at least one ASCII letter** — this is what saves `[1]` and `[2026]`, the
///   footnote and year references a transcript legitimately contains;
/// - **nothing but ASCII letters, digits, spaces, `_`, `-`, `:`** — this is what
///   saves `[附录2]`, `[见图3]` and `[TODO 中文]`: any CJK, punctuation or symbol
///   means a human wrote it as content.
fn is_stage_direction(inner: &str) -> bool {
    inner.len() <= MAX_INNER_BYTES
        && inner.bytes().any(|byte| byte.is_ascii_alphabetic())
        && inner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_' | b'-' | b':'))
}

/// Remove every stage direction from a whole string.
///
/// An opening delimiter that is not part of one — unclosed, too long, or holding
/// real content — is emitted literally, and scanning continues behind it, so no
/// input ever loses text it did not consist of. Never swallows the rest of a
/// line.
pub fn strip_stage_directions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some((at, open, closer)) = next_opener(rest) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        // `at` and the opener's width are both char boundaries, so is this.
        let after_open = &rest[at + open.len_utf8()..];
        match after_open.find(closer) {
            Some(end) if is_stage_direction(&after_open[..end]) => {
                rest = &after_open[end + closer.len_utf8()..];
            }
            // Not a stage direction: the delimiter is literal text. Emit it and
            // rescan from just behind it — the bytes it precedes may still hold
            // a real one (`a[b[winking]`).
            _ => {
                out.push(open);
                rest = after_open;
            }
        }
    }
}

/// Streaming form of [`strip_stage_directions`] for token-level deltas.
///
/// `AgentStreamEvent::Text` is a delta, so a stage direction straddles chunk
/// boundaries (`[wink` + `ing]`) and a naive per-delta call leaks both halves.
/// This withholds at most [`MAX_WITHHELD_BYTES`] trailing bytes until the closer
/// arrives or the bound proves nothing can qualify. It is a transport adapter,
/// NOT a second policy: `filter_matches_the_whole_string_form` pins the two
/// together over every case in the table.
///
/// The bound is safe to state as equivalence — unlike the marker filter this
/// replaces, which had to document a divergence — because the length limit is
/// part of the *policy* here, not just of the transport: an inner text past
/// [`MAX_INNER_BYTES`] is content to both forms.
///
/// Withheld bytes are never dropped: whoever ends the text run must call
/// [`StageDirectionFilter::flush`], which releases them verbatim.
#[derive(Debug, Default)]
pub struct StageDirectionFilter {
    /// A possible stage direction under construction, always at most
    /// [`MAX_WITHHELD_BYTES`] long and always valid UTF-8 — deltas arrive as
    /// `&str` and every slice below is taken at a char boundary, so a partial
    /// multi-byte sequence can never be emitted.
    pending: String,
}

impl StageDirectionFilter {
    /// Feed one delta; returns the bytes that are safe to emit now.
    pub fn push(&mut self, delta: &str) -> String {
        let mut work = std::mem::take(&mut self.pending);
        work.push_str(delta);
        let mut out = String::with_capacity(work.len());
        let mut rest = work.as_str();
        loop {
            let Some((at, open, closer)) = next_opener(rest) else {
                out.push_str(rest);
                rest = "";
                break;
            };
            out.push_str(&rest[..at]);
            let candidate = &rest[at..];
            let after_open = &rest[at + open.len_utf8()..];
            match after_open.find(closer) {
                // A complete stage direction: drop it and keep scanning.
                Some(end) if is_stage_direction(&after_open[..end]) => {
                    rest = &after_open[end + closer.len_utf8()..];
                }
                // Closed, but it is content. Same reading as the whole-string
                // form: the delimiter is text.
                Some(_) => {
                    out.push(open);
                    rest = after_open;
                }
                // Unclosed so far, and already longer than any inner text that
                // could qualify — so no closer arriving later can save it and
                // the delimiter is text. Emit it and rescan, rather than
                // buffering without bound.
                None if after_open.len() > MAX_INNER_BYTES => {
                    out.push(open);
                    rest = after_open;
                }
                // Still could become a stage direction; wait for more input.
                None => {
                    rest = candidate;
                    break;
                }
            }
        }
        self.pending = rest.to_owned();
        debug_assert!(
            self.pending.len() <= MAX_WITHHELD_BYTES,
            "withheld text must stay bounded"
        );
        out
    }

    /// Release anything still withheld, verbatim. Call when the text run ends.
    ///
    /// Verbatim is correct, not lazy: nothing is ever withheld once it has a
    /// closer, so an unterminated candidate is exactly what
    /// [`strip_stage_directions`] would also emit unchanged.
    pub fn flush(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reported bug, verbatim: the model emits the bare name, and no
    /// `emotion:`-keyed stripper ever matched it.
    #[test]
    fn strips_the_bare_name_the_model_actually_emits() {
        assert_eq!(strip_stage_directions("[winking]你好呀"), "你好呀");
        assert_eq!(strip_stage_directions("【winking】你好呀"), "你好呀");
        assert_eq!(strip_stage_directions("[Winking] 你好呀"), " 你好呀");
    }

    /// The dead syntax still has to go — a transcript already carries it, and a
    /// model told to stop may not stop at once.
    #[test]
    fn strips_the_dead_emotion_syntax_too_without_knowing_it() {
        assert_eq!(strip_stage_directions("[emotion:winking]你好"), "你好");
        assert_eq!(strip_stage_directions("[emotion: happy]你好"), "你好");
        assert_eq!(
            strip_stage_directions("[emotion:ecstatic]太好了"),
            "太好了",
            "no vocabulary is consulted, so an invented name strips identically"
        );
    }

    #[test]
    fn strips_ordinary_stage_directions_and_action_annotations() {
        assert_eq!(strip_stage_directions("[laughs]真好玩"), "真好玩");
        assert_eq!(strip_stage_directions("[pause]然后呢"), "然后呢");
        assert_eq!(strip_stage_directions("[smiling softly]嗯"), "嗯");
    }

    /// The other half of the contract: real bracketed content is not ours to
    /// delete. `[附录2]` is the case the user named.
    #[test]
    fn keeps_real_bracketed_content() {
        for kept in [
            "见附录[1]",
            "写于[2026]",
            "见附录[附录2]",
            "参考[见图3]",
            "[TODO 中文]",
            "【附录2】",
        ] {
            assert_eq!(
                strip_stage_directions(kept),
                kept,
                "{kept} is content a human wrote"
            );
        }
    }

    #[test]
    fn an_unclosed_delimiter_is_emitted_and_swallows_nothing() {
        assert_eq!(strip_stage_directions("未闭合 [winking 保留"), "未闭合 [winking 保留");
        assert_eq!(strip_stage_directions("["), "[");
        assert_eq!(strip_stage_directions("【"), "【");
        assert_eq!(
            strip_stage_directions("[winking 后面还有很多正文，一个字都不能吞"),
            "[winking 后面还有很多正文，一个字都不能吞"
        );
    }

    #[test]
    fn a_long_inner_run_is_prose_not_an_annotation() {
        let long = "a".repeat(MAX_INNER_BYTES + 1);
        let input = format!("[{long}]");
        assert_eq!(strip_stage_directions(&input), input);
        let limit = "a".repeat(MAX_INNER_BYTES);
        assert_eq!(strip_stage_directions(&format!("[{limit}]")), "");
    }

    #[test]
    fn a_stage_direction_is_stripped_anywhere_in_the_line() {
        assert_eq!(strip_stage_directions("我很好[winking]你呢"), "我很好你呢");
        assert_eq!(
            strip_stage_directions("[happy]你好。[sad]再见。"),
            "你好。再见。",
            "several per line, which a 3-sentence reply produces"
        );
    }

    /// Feed `input` one `char` at a time — the worst case a token stream can
    /// produce — and return everything the filter emitted.
    fn drain_char_by_char(input: &str) -> String {
        let mut filter = StageDirectionFilter::default();
        let mut out = String::new();
        for ch in input.chars() {
            out.push_str(&filter.push(&ch.to_string()));
        }
        out.push_str(&filter.flush());
        out
    }

    #[test]
    fn filter_strips_a_stage_direction_split_across_deltas() {
        let mut filter = StageDirectionFilter::default();
        let mut out = String::new();
        for delta in ["[wink", "ing]", "你好"] {
            out.push_str(&filter.push(delta));
        }
        out.push_str(&filter.flush());
        assert_eq!(out, "你好", "a bracket straddling a chunk boundary is still one");
    }

    #[test]
    fn filter_flush_releases_a_truncated_candidate() {
        let mut filter = StageDirectionFilter::default();
        let emitted = filter.push("hi[wink");
        assert_eq!(emitted, "hi", "the possible stage direction is withheld");
        assert_eq!(
            format!("{emitted}{}", filter.flush()),
            "hi[wink",
            "flush must never lose withheld text"
        );
    }

    #[test]
    fn filter_passes_real_content_through() {
        assert_eq!(drain_char_by_char("见附录[1]和[附录2]"), "见附录[1]和[附录2]");
    }

    #[test]
    fn filter_withhold_is_bounded() {
        let input = format!("[{}", "x".repeat(60));
        let mut filter = StageDirectionFilter::default();
        let mut out = String::new();
        for ch in input.chars() {
            out.push_str(&filter.push(&ch.to_string()));
            assert!(
                filter.pending.len() <= MAX_WITHHELD_BYTES,
                "an unterminated candidate must not buffer without bound"
            );
        }
        out.push_str(&filter.flush());
        assert_eq!(
            out, input,
            "a candidate that never closes is text, and every byte of it is emitted"
        );
    }

    /// The bound is measured in BYTES while the text is scanned by `char`, so
    /// multi-byte content is where an off-by-one would corrupt a transcript: it
    /// must neither split a character nor let the withhold buffer grow.
    #[test]
    fn multibyte_content_survives_whole_and_still_respects_the_bound() {
        let mut filter = StageDirectionFilter::default();
        let mut out = String::new();
        // A full-width pair around a name, then CJK long enough to blow past the
        // 24-byte inner bound (each char is 3 bytes).
        for ch in "【winking】你好呀，【这一段中文远远超过二十四个字节的上限】完".chars() {
            out.push_str(&filter.push(&ch.to_string()));
            assert!(
                filter.pending.len() <= MAX_WITHHELD_BYTES,
                "CJK inside a candidate must not grow the withhold buffer"
            );
            assert!(
                filter.pending.chars().count() * 4 >= filter.pending.len(),
                "the withheld tail is whole characters, never a split sequence"
            );
        }
        out.push_str(&filter.flush());
        assert_eq!(
            out, "你好呀，【这一段中文远远超过二十四个字节的上限】完",
            "the short annotation goes, every CJK byte of the long one stays"
        );
    }

    /// The load-bearing test: the filter is an adapter over
    /// [`strip_stage_directions`], not a second policy. Same shape as the test
    /// the deleted `EmotionMarkerFilter` carried, minus its documented
    /// divergence — the length bound is policy here, so the two agree
    /// everywhere.
    #[test]
    fn filter_matches_the_whole_string_form() {
        let long = "x".repeat(MAX_INNER_BYTES + 6);
        for input in [
            "",
            "你好",
            "[winking]你好",
            "[emotion:winking]你好",
            "【winking】你好",
            "[happy]你好。[sad]再见。",
            "我很好[winking]你呢",
            "见附录[1]和[winking]备注",
            "见附录[附录2]",
            "[TODO 中文]",
            "[Winking]",
            "[smiling softly]嗯",
            "[emotion: happy]嗯",
            "[]空的",
            "[1]",
            "[2026]",
            "hi[wink",
            "未闭合 [winking 保留",
            "[",
            "【",
            "[[winking]]",
            "a[b[c[winking]d",
            "[winking】混搭",
            &long,
            &format!("[{long}]"),
            &format!("[{long}[winking]hi"),
        ] {
            assert_eq!(
                drain_char_by_char(input),
                strip_stage_directions(input),
                "streaming and whole-string must agree on {input:?}"
            );
        }
    }
}
