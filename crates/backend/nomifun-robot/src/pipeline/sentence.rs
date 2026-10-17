//! Incremental sentence splitting and the text cleaning the device needs.
//!
//! The model streams text; the device needs whole sentences (one `sentence_start`
//! plus its audio at a time). Splitting eagerly is what keeps first-audio latency
//! low, so this runs on every stream chunk.
//!
//! This consumes canonical reply text without changing the source facts. The
//! speaker and device display drop conversational decorations, stage directions
//! and unsupported symbols. Expressions use the Session's granted device tools;
//! there is no expression channel hidden in the spoken text.

pub use nomifun_common::stage_direction::strip_stage_directions;
use super::decorations::{guard_stages, sentence_protected_len, strip_decorations};

/// Terminators that end a sentence. `\n` counts: the model uses it as a beat.
const TERMINATORS: [char; 9] = ['。', '！', '？', '；', '!', '?', ';', '\n', '.'];

/// A character a speech engine cannot voice: emoji, pictographs, dingbats,
/// arrows, technical symbols, variation selectors and (non-whitespace) control
/// characters. Deliberately a blocklist of symbol ranges rather than a letter
/// whitelist, so it never strips real language — CJK, kana, latin, digits and
/// both ASCII and full-width punctuation all pass through.
fn is_non_speech_char(ch: char) -> bool {
    let c = ch as u32;
    if ch.is_control() {
        return true;
    }
    matches!(
        c,
        0x1F000..=0x1FAFF   // emoticons, pictographs, transport, supplemental symbols, flags
        | 0x2600..=0x27BF   // miscellaneous symbols + dingbats
        | 0x2B00..=0x2BFF   // miscellaneous symbols and arrows
        | 0x2300..=0x23FF   // miscellaneous technical (⌚ ⏰ ⏳ …)
        | 0x2190..=0x21FF   // arrows
        | 0xFE00..=0xFE0F   // variation selectors
        | 0x200D            // zero-width joiner (glues emoji sequences)
        | 0x20E3            // combining enclosing keycap
        | 0xE0020..=0xE007F // emoji tag sequences, including subdivision flags
        | 0x2122 | 0x2139   // ™ ℹ
        | 0x3030 | 0x303D | 0x3297 | 0x3299 // isolated CJK emoji symbols
        | 0x203C | 0x2049   // ‼ ⁉
        | 0xFFFC | 0xFFFD   // object-replacement / replacement character
    )
}

/// The text that is safe to hand a TTS engine: stage directions removed, emoji
/// and other non-speech symbols dropped, runs of whitespace collapsed to one
/// space, trimmed. Returns an empty string when nothing speakable remains (a
/// sentence that was only an emoji or only a stage direction) — the caller then
/// skips synthesis instead of letting the engine 400 and cut the voice off.
pub fn sanitize_for_speech(text: &str) -> String {
    // An image must be recognized before its [alt] can be mistaken for a stage
    // direction, leaving the URL behind for the speech engine.
    let decorated = strip_decorations(text);
    let guarded = guard_stages(&decorated, strip_stage_directions);
    let mut out = String::with_capacity(guarded.len());
    let mut last_was_space = false;
    let mut chars = guarded.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '©' | '®') && chars.peek() == Some(&'\u{FE0F}') {
            chars.next();
            continue;
        }
        if ch.is_ascii_digit() || matches!(ch, '#' | '*') {
            let mut sequence = chars.clone();
            if sequence.peek() == Some(&'\u{FE0F}') { sequence.next(); }
            if sequence.next() == Some('\u{20E3}') { chars = sequence; continue; }
        }
        if ch.is_whitespace() {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
            }
            last_was_space = true;
            continue;
        }
        if is_non_speech_char(ch) {
            continue;
        }
        out.push(ch);
        last_was_space = false;
    }
    let out = out.trim();
    if !out.chars().any(char::is_alphanumeric) {
        String::new()
    } else {
        out.to_owned()
    }
}


/// The text the device's OLED can actually render.
///
/// Deliberately the same cleaning as [`sanitize_for_speech`]: every class of
/// character a speech engine cannot voice — emoji, pictographs, dingbats,
/// arrows, technical symbols — is also missing from the 14px CJK font the
/// firmware draws with, where it comes out as a hollow box. Removing bracketed
/// runs alone (which is what this used to do) left `🌱` on the screen and pushed
/// the firmware into carrying its own duplicate sanitiser; the gateway owns the
/// wire format, so it cleans both copies.
///
/// Kept as a separate name from the speech copy so the two call sites read as
/// what they mean, and so they can diverge if a device ever ships an emoji font.
pub fn sanitize_for_display(text: &str) -> String {
    sanitize_for_speech(text)
}

/// Buffers streamed text and hands back whole sentences.
#[derive(Debug, Default)]
pub struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    /// Feed a stream chunk; returns every sentence it completed.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        loop {
            let Some(cut) = self.find_terminator() else { break };
            let sentence: String = self.buf.drain(..cut).collect();
            // A newline is a beat, not punctuation the listener should hear or
            // read, so it does not travel with the sentence it ended.
            let sentence = sentence.trim_end_matches(['\n', '\r']);
            if !sentence.trim().is_empty() {
                out.push(sentence.to_owned());
            }
        }
        // A buffer holding only whitespace is not worth carrying.
        if self.buf.trim().is_empty() {
            self.buf.clear();
        }
        out
    }

    /// Byte index just past the first real terminator, if any.
    fn find_terminator(&self) -> Option<usize> {
        let bytes_len = self.buf.len();
        let mut protected_until = 0;
        for (index, ch) in self.buf.char_indices() {
            if index < protected_until { continue; }
            if let Some(bytes) = sentence_protected_len(&self.buf[index..]) {
                protected_until = index + bytes;
                continue;
            }
            if !TERMINATORS.contains(&ch) {
                continue;
            }
            let end = index + ch.len_utf8();
            // A dot inside a decimal or filename is content. A trailing dot
            // after a number, path or underscored token waits so `T_T.rs`
            // cannot become a face followed by a separately spoken extension.
            if ch == '.' {
                let before = self.buf[..index].chars().next_back();
                let after = if end < bytes_len {
                    self.buf[end..].chars().next()
                } else {
                    None
                };
                if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                    if after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                        continue;
                    }
                    let token = self.buf[..index].split_whitespace().next_back().unwrap_or("");
                    if after.is_none() && (before.is_some_and(|ch| ch.is_ascii_digit())
                        || token.contains(['_', '/', '\\', '.'])) {
                        return None;
                    }
                }
            }
            return Some(end);
        }
        None
    }

    /// Emit whatever is left (end of turn), if it is not blank.
    pub fn flush(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        if rest.trim().is_empty() { None } else { Some(rest) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_chinese_and_ascii_terminators() {
        let mut s = SentenceSplitter::default();
        assert_eq!(s.push("你好。今天"), vec!["你好。"]);
        assert_eq!(s.push("天气不错！"), vec!["今天天气不错！"]);
        assert_eq!(s.push("Hi there. Bye?"), vec!["Hi there.", " Bye?"]);
        assert!(s.push("no terminator yet").is_empty());
        assert_eq!(s.flush().as_deref(), Some("no terminator yet"));
        assert!(s.flush().is_none(), "flush drains");
    }

    #[test]
    fn newline_ends_a_sentence_too() {
        let mut s = SentenceSplitter::default();
        assert_eq!(s.push("第一行\n第二行"), vec!["第一行"]);
    }

    #[test]
    fn decimal_points_do_not_split_english_numbers() {
        let mut s = SentenceSplitter::default();
        assert!(
            s.push("It is 3.5 degrees").is_empty(),
            "3.5 is not a sentence end"
        );
        assert_eq!(s.push(" outside.").len(), 1);
        assert_eq!(SentenceSplitter::default().push("价格$5。下一句。"), vec!["价格$5。", "下一句。"]);
    }

    #[test]
    fn whitespace_only_output_is_never_emitted() {
        let mut s = SentenceSplitter::default();
        assert!(
            s.push("   \n  ").is_empty(),
            "blank lines are not sentences"
        );
        assert!(s.flush().is_none());
    }

    #[test]
    fn sanitize_drops_emoji_but_keeps_the_words() {
        // The reported case: an emoji in the middle of a reply must not reach the
        // TTS engine, but the sentence around it must still be spoken.
        assert_eq!(sanitize_for_speech("北京现在是 🌤️ 挺舒服的天气呢~"), "北京现在是 挺舒服的天气呢~");
        assert_eq!(sanitize_for_speech("好耶！🎉🎉"), "好耶！");
        assert_eq!(sanitize_for_speech("👍"), "", "a lone emoji leaves nothing to say");
    }

    #[test]
    fn sanitize_removes_complete_emoji_sequences_and_faces() {
        for decoration in ["1️⃣", "2\u{20E3}", "#️⃣", "*️⃣", "🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}", "👩🏽‍💻", "🇨🇳", "©️", "®️", "(^_^)", "(^_^)！", "👍。", "[笑哭]！", "(•̀ᴗ•́)و", ";-)", "QAQ!", "[微笑]", "[表情包：开心]", "![smile](https://example.test/sticker.png)"] {
            assert_eq!(sanitize_for_speech(decoration), "", "{decoration}");
            assert_eq!(sanitize_for_display(decoration), "", "{decoration}");
        }
        let plain = "第1项、#标题、a * b、（正常括号）、(x > y)、https://example.test/T_T、`T_T`、![diagram](https://example.test/diagram.png)";
        assert_eq!(sanitize_for_speech(plain), plain);
        assert_eq!(sanitize_for_speech("完成 (^_^) ，请看结果。"), "完成 ，请看结果。");
    }

    #[test]
    fn splitting_keeps_chunked_faces_and_images_whole_for_the_guard() {
        let mut splitter = SentenceSplitter::default();
        for chunk in ["(", "^_", "^)", "\n", "!", "[sm", "ile](https://example.test/sticker", ".png)", "\n", ";", "-)", "\n", "正文。"] {
            for sentence in splitter.push(chunk) {
                let cleaned = sanitize_for_speech(&sentence);
                assert!(cleaned.is_empty() || cleaned == "正文。", "{cleaned:?}");
            }
        }
        assert!(splitter.flush().is_none());
        // Malformed image input cannot hold back unlimited later speech.
        let malformed = format!("![{}正文。", "a".repeat(2100));
        assert_eq!(SentenceSplitter::default().push(&malformed), vec!["!", &malformed[1..]]);
        let mut literal = SentenceSplitter::default();
        assert!(literal.push("T_T.").is_empty());
        assert!(literal.push("rs https://example.test/T_T").is_empty());
        assert_eq!(sanitize_for_speech(&literal.flush().unwrap()), "T_T.rs https://example.test/T_T");
    }

    #[test]
    fn sanitize_strips_stage_directions_anywhere() {
        assert_eq!(sanitize_for_speech("[winking]你好"), "你好");
        assert_eq!(
            sanitize_for_speech("我觉得 [sighs] 不太好"),
            "我觉得 不太好",
            "a mid-line stage direction is removed too, not spoken verbatim"
        );
        assert_eq!(
            sanitize_for_speech("[emotion:relaxed] 北京现在是 🌤️"),
            "北京现在是",
            "the dead marker syntax is only one shape of the same guard"
        );
        assert_eq!(
            sanitize_for_speech("见附录[2]和[附录3]"),
            "见附录[2]和[附录3]",
            "real bracketed content is spoken, not silently deleted"
        );
    }

    #[test]
    fn sanitize_keeps_cjk_latin_digits_and_punctuation() {
        let s = "天气 25℃，很好！Hello, world. 3.5 倍——真的？";
        // ℃ (0x2103) is technical-symbol-adjacent but outside our blocklist; the
        // point is that letters, digits, CJK and punctuation all survive.
        let out = sanitize_for_speech(s);
        assert!(out.contains("天气"));
        assert!(out.contains("Hello, world."));
        assert!(out.contains("3.5"));
        assert!(out.contains("真的？"));
    }

    /// The screen copy used to remove bracketed runs and nothing else, which left
    /// emoji on a 14px CJK font that has no glyph for them — the device drew a
    /// hollow box, and the firmware grew its own duplicate sanitiser to
    /// compensate. The gateway owns the wire format, so the screen copy is
    /// cleaned too.
    #[test]
    fn the_screen_copy_drops_emoji_not_just_stage_directions() {
        assert_eq!(
            sanitize_for_display("[loving] 往往藏着最明亮的开始～ 🌱"),
            "往往藏着最明亮的开始～",
            "no emoji reaches the OLED, and the decorative ～ it CAN render survives"
        );
        assert_eq!(
            sanitize_for_display("我觉得 【sad】 不太好"),
            "我觉得 不太好",
            "a full-width bracketed annotation never reaches the screen either"
        );
        assert_eq!(
            sanitize_for_display("突然「哗啦」一声——真的？"),
            "突然「哗啦」一声——真的？",
            "CJK quotes and em-dashes are in the font and must not be stripped"
        );
        assert_eq!(sanitize_for_display("👍"), "", "an emoji-only line leaves nothing to show");
    }
}
