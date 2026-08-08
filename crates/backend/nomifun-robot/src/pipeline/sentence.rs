//! Incremental sentence splitting and emotion markers.
//!
//! The model streams text; the device needs whole sentences (one `sentence_start`
//! plus its audio at a time). Splitting eagerly is what keeps first-audio latency
//! low, so this runs on every stream chunk.
//!
//! Emotion travels as a leading `[emotion:name]` marker the system prompt asks
//! for. It is stripped before display and TTS, and mapped onto the 21 names the
//! firmware understands (anything else would silently become `neutral` on-device
//! anyway, so we normalise here and log nothing).

/// The exact emotion vocabulary the firmware maps to eye animations and gimbal
/// moves. Any other value degrades to `neutral` on-device.
pub const EMOTIONS: [&str; 21] = [
    "neutral",
    "happy",
    "laughing",
    "funny",
    "sad",
    "angry",
    "crying",
    "loving",
    "embarrassed",
    "surprised",
    "shocked",
    "thinking",
    "winking",
    "cool",
    "relaxed",
    "delicious",
    "kissy",
    "confident",
    "sleepy",
    "silly",
    "confused",
];

/// Map any name onto the firmware vocabulary, defaulting to `neutral`.
pub fn normalize_emotion(name: &str) -> &'static str {
    let needle = name.trim().to_ascii_lowercase();
    EMOTIONS
        .iter()
        .copied()
        .find(|known| *known == needle)
        .unwrap_or("neutral")
}

/// Split a leading `[emotion:name]` marker off a sentence.
///
/// Returns the normalised emotion (only when a marker was present) and the
/// remaining text. A marker anywhere but the start is left alone — the model was
/// asked to lead with it, and rewriting mid-sentence text would mangle content.
pub fn strip_emotion(sentence: &str) -> (Option<&'static str>, String) {
    let trimmed = sentence.trim_start();
    let Some(rest) = trimmed.strip_prefix("[emotion:") else {
        return (None, sentence.to_owned());
    };
    let Some(end) = rest.find(']') else {
        return (None, sentence.to_owned());
    };
    let name = normalize_emotion(&rest[..end]);
    (Some(name), rest[end + 1..].trim_start().to_owned())
}

/// Terminators that end a sentence. `\n` counts: the model uses it as a beat.
const TERMINATORS: [char; 9] = ['。', '！', '？', '；', '!', '?', ';', '\n', '.'];

/// Remove every `[emotion:name]` marker anywhere in the text, not just a leading
/// one. `strip_emotion` consumes the leading marker to drive the face; a model
/// that emits one mid-line (against instructions) would otherwise have it shown
/// on the OLED and read aloud verbatim.
pub fn strip_emotion_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[emotion:") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.find(']') {
            Some(end) => rest = &after[end + 1..],
            // No closing bracket: not a real marker, keep the remainder as text.
            None => {
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

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
        | 0x2122 | 0x2139   // ™ ℹ
        | 0x203C | 0x2049   // ‼ ⁉
        | 0xFFFC | 0xFFFD   // object-replacement / replacement character
    )
}

/// The text that is safe to hand a TTS engine: emotion markers removed, emoji
/// and other non-speech symbols dropped, runs of whitespace collapsed to one
/// space, trimmed. Returns an empty string when nothing speakable remains (a
/// sentence that was only an emoji or a stray marker) — the caller then skips
/// synthesis instead of letting the engine 400 and cut the voice off.
pub fn sanitize_for_speech(text: &str) -> String {
    let without_markers = strip_emotion_markers(text);
    let mut out = String::with_capacity(without_markers.len());
    let mut last_was_space = false;
    for ch in without_markers.chars() {
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
    out.trim().to_owned()
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
        for (index, ch) in self.buf.char_indices() {
            if !TERMINATORS.contains(&ch) {
                continue;
            }
            let end = index + ch.len_utf8();
            // An ASCII '.' surrounded by digits is a decimal point, not a full
            // stop, and one that trails a digit at the very end of the buffer may
            // still become one ("3." + "5"), so that case waits for more input.
            if ch == '.' {
                let before = self.buf[..index].chars().next_back();
                let after = if end < bytes_len {
                    self.buf[end..].chars().next()
                } else {
                    None
                };
                if before.is_some_and(|c| c.is_ascii_digit()) {
                    if after.is_some_and(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    if after.is_none() {
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
    fn strips_a_leading_emotion_marker() {
        let (emotion, text) = strip_emotion("[emotion:happy] 你好呀");
        assert_eq!(emotion, Some("happy"));
        assert_eq!(text, "你好呀");
    }

    #[test]
    fn unknown_emotion_name_falls_back_to_neutral() {
        let (emotion, text) = strip_emotion("[emotion:ecstatic]太好了");
        assert_eq!(emotion, Some("neutral"), "the firmware only knows 21 names");
        assert_eq!(text, "太好了");
    }

    #[test]
    fn sentence_without_marker_is_untouched() {
        let (emotion, text) = strip_emotion("就这样");
        assert_eq!(emotion, None);
        assert_eq!(text, "就这样");
    }

    #[test]
    fn marker_must_be_at_the_start_to_count() {
        let (emotion, text) = strip_emotion("我觉得 [emotion:sad] 不太好");
        assert_eq!(emotion, None);
        assert_eq!(text, "我觉得 [emotion:sad] 不太好");
    }

    #[test]
    fn normalize_accepts_all_21_firmware_names() {
        assert_eq!(EMOTIONS.len(), 21);
        for name in EMOTIONS {
            assert_eq!(
                normalize_emotion(name),
                name,
                "{name} must survive normalisation"
            );
        }
        assert_eq!(normalize_emotion("HAPPY"), "happy", "case-insensitive");
        assert_eq!(normalize_emotion(" happy "), "happy", "trimmed");
        assert_eq!(normalize_emotion("nonsense"), "neutral");
        assert_eq!(normalize_emotion(""), "neutral");
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
    fn sanitize_strips_emotion_markers_anywhere() {
        assert_eq!(sanitize_for_speech("[emotion:happy]你好"), "你好");
        assert_eq!(
            sanitize_for_speech("我觉得 [emotion:sad] 不太好"),
            "我觉得 不太好",
            "a mid-line marker is removed too, not spoken verbatim"
        );
        assert_eq!(sanitize_for_speech("[emotion:relaxed] 北京现在是 🌤️"), "北京现在是");
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

    #[test]
    fn strip_emotion_markers_leaves_ordinary_brackets_alone() {
        assert_eq!(strip_emotion_markers("见附录[1]和[emotion:cool]备注"), "见附录[1]和备注");
        assert_eq!(
            strip_emotion_markers("未闭合 [emotion:happy 保留"),
            "未闭合 [emotion:happy 保留",
            "an unclosed marker is not a marker"
        );
    }
}
