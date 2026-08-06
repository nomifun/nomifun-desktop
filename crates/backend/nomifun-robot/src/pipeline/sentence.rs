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
}
