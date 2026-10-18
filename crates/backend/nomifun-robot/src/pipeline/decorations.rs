//! Conservative removal of conversational decorations at the device boundary.
//! Code, URLs, ordinary parentheses and mathematical content remain text.

const BARE_FACES: [&str; 10] = [
    "^_^",
    "T_T",
    "Q_Q",
    ">_<",
    "ಠ_ಠ",
    "ಥ_ಥ",
    "QAQ",
    "QwQ",
    "╥﹏╥",
    "ಥ﹏ಥ",
];
const SMILEYS: [&str; 19] = [
    ":-)", ":-D", ";-)", ":-P", ":-p", ":)", ":D", ";)", ":P", ":p", ":-]", ":]", ":/", ":\\", ":(", ":-(",
    ":-O", ":O", "<3",
];
const STICKER_LABELS: [&str; 18] = [
    "微笑", "笑哭", "捂脸", "偷笑", "大笑", "呲牙", "调皮", "眨眼", "比心", "爱心", "害羞", "流泪", "大哭",
    "委屈", "可怜", "doge", "可爱", "开心",
];
const MAX_IMAGE_BYTES: usize = 2048;

fn has_ascii_word(text: &str, words: &[&str]) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| words.contains(&word) || word.strip_suffix('s').is_some_and(|word| words.contains(&word)))
}

fn decorative_image(text: &str) -> bool {
    let Some((alt, target)) = text.strip_prefix("![").and_then(|text| text.split_once(']')) else {
        return false;
    };
    let label = alt.trim().to_ascii_lowercase();
    let target = target.to_ascii_lowercase();
    ["表情", "颜文字", "贴纸", "梗图"]
        .iter()
        .any(|word| label.contains(word))
        || has_ascii_word(
            &label,
            &["emoji", "emote", "emoticon", "sticker", "meme", "kaomoji"],
        )
        || has_ascii_word(&target, &["emoji", "emoticon", "sticker", "meme"])
        || [
            "笑", "微笑", "大笑", "捂脸", "开心", "笑脸", "wink", "smile", "happy", "facepalm",
        ]
        .contains(&label.as_str())
        || alt
            .chars()
            .any(|ch| matches!(ch as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x20E3))
}

fn ascii_identifier(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn face_core_at(text: &str) -> Option<usize> {
    if let Some(face) = BARE_FACES.iter().find(|face| text.starts_with(**face)) {
        return Some(face.len());
    }
    let chars: Vec<(usize, char)> = text.char_indices().take(8).collect();
    let (_, eye) = *chars.first()?;
    let mouth = |ch| matches!(ch, '_' | ' ' | 'ω' | '▽' | 'ᴗ' | '﹏' | '.' | '-');
    let mut end = None;
    if chars.len() >= 3 {
        let middle = chars[1].1;
        let last = chars[2].1;
        let matched = match eye {
            '^' => last == '^' && mouth(middle),
            'T' | 'Q' => last == eye && matches!(middle, '_' | ' ' | '.' | '-'),
            '>' => last == '<' && mouth(middle),
            'o' | 'O' => matches!(last, 'o' | 'O') && matches!(middle, '_' | ' ' | '.' | '-'),
            '≧' => last == '≦' && matches!(middle, '▽' | '∀' | 'ω' | 'ᴗ'),
            '￣' => last == '￣' && matches!(middle, '▽' | 'ω' | '︶' | 'ᴗ'),
            _ => false,
        };
        if matched {
            end = Some(chars[2].0 + last.len_utf8());
        }
    }
    if matches!(eye, '•' | '・' | '･' | '❛' | '◕' | '°') {
        let mut start = 1;
        if chars
            .get(start)
            .is_some_and(|(_, ch)| matches!(ch, '\u{0300}' | '\u{0301}'))
        {
            start += 1;
        }
        for last in start + 1..=(start + 3).min(chars.len().saturating_sub(1)) {
            if chars[start..last]
                .iter()
                .all(|(_, ch)| mouth(*ch) || matches!(ch, '∀' | '‿'))
                && matches!(chars[last].1, '•' | '・' | '･' | '❛' | '◕' | '°')
            {
                let mut size = chars[last].0 + chars[last].1.len_utf8();
                if chars
                    .get(last + 1)
                    .is_some_and(|(_, ch)| matches!(ch, '\u{0300}' | '\u{0301}'))
                {
                    size += chars[last + 1].1.len_utf8();
                }
                end = Some(size);
                break;
            }
        }
    }
    end
}

fn carrier_char(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '(' | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '【'
                | '】'
                | '{'
                | '}'
                | 'つ'
                | 'っ'
                | 'づ'
                | 'ﾉ'
                | 'ノ'
                | 'ヽ'
                | 'ヾ'
                | '੭'
                | 'و'
                | 'ง'
                | '╯'
                | '╰'
                | '┛'
                | '┗'
                | '♡'
                | '♥'
                | '☆'
                | '★'
                | '✧'
                | '✦'
                | '*'
                | '｡'
                | '。'
                | 'ﾟ'
                | '゜'
                | 'ˊ'
                | 'ˋ'
                | '˘'
                | '˙'
                | '˶'
                | 'ฅ'
                | 'ʕ'
                | 'ʔ'
                | '༼'
                | '༽'
                | '彡'
                | '～'
                | '~'
                | '!'
                | '！'
                | '๑'
                | '٩'
                | '۶'
                | '°'
        )
}

fn face_carrier(text: &str) -> bool {
    let mut rest = text;
    let mut core = false;
    while !rest.is_empty() {
        if let Some(bytes) = face_core_at(rest) {
            core = true;
            rest = &rest[bytes..];
            continue;
        }
        let ch = rest.chars().next().expect("non-empty");
        if !carrier_char(ch) {
            return false;
        }
        rest = &rest[ch.len_utf8()..];
    }
    core
}

/// Complete Markdown image length, including nested parentheses in its target.
/// `None` preserves malformed or incomplete text for the next stream fragment.
fn image_len(text: &str) -> Option<usize> {
    if !text.starts_with("![") {
        return None;
    }
    let label_end = text[2..].find(']')? + 2;
    if text[label_end + 1..].starts_with('[') {
        return text[label_end + 2..].find(']').map(|end| label_end + 2 + end + 1);
    }
    if !text[label_end + 1..].starts_with('(') {
        return None;
    }
    let mut depth = 1usize;
    for (at, ch) in text[label_end + 2..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(label_end + 2 + at + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn protected_len(text: &str) -> Option<usize> {
    if text.starts_with("http://") || text.starts_with("https://") {
        return Some(
            text.find(|ch: char| {
                ch.is_whitespace() || matches!(ch, '。' | '，' | '！' | '？' | '；' | '<' | '>')
            })
            .unwrap_or(text.len()),
        );
    }
    let first = text.chars().next()?;
    if matches!(first, '`' | '$' | '~') {
        let count = text.chars().take_while(|ch| *ch == first).count();
        if first == '~' && count < 3 {
            return None;
        }
        // Currency is prose. Only paired inline math is literal; unlike a
        // fenced block, an unmatched `$` must not hold subsequent sentences.
        if first == '$' && count == 1 && text[1..].chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let marker = first.to_string().repeat(count);
        return text[count..]
            .find(&marker)
            .map(|at| count + at + count)
            .or_else(|| {
                ((first == '$' && count >= 2) || (matches!(first, '`' | '~') && count >= 3))
                    .then_some(text.len())
            });
    }
    None
}

pub(super) fn strip_decorations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if let Some(bytes) = protected_len(rest) {
            out.push_str(&rest[..bytes]);
            at += bytes;
            continue;
        }
        if let Some(bytes) = image_len(rest) {
            if !decorative_image(&rest[..bytes]) {
                out.push_str(&rest[..bytes]);
            }
            at += bytes;
            continue;
        }
        if rest.starts_with("o(*￣▽￣*)o") {
            at += "o(*￣▽￣*)o".len();
            continue;
        }
        let ch = rest.chars().next().expect("non-empty");
        let prefix = if matches!(ch, '╰' | '٩' | 'ヾ') && rest[ch.len_utf8()..].starts_with(['(', '（'])
        {
            ch.len_utf8()
        } else {
            0
        };
        let opener = rest[prefix..].chars().next().expect("non-empty");
        let closer = match opener {
            '(' => Some(')'),
            '（' => Some('）'),
            '[' => Some(']'),
            '【' => Some('】'),
            _ => None,
        };
        if let Some(closer) = closer {
            if let Some((end, _)) = rest.char_indices().take(50).find(|(_, ch)| *ch == closer) {
                let inner = &rest[prefix + opener.len_utf8()..end];
                let sticker = matches!(opener, '[' | '【')
                    && (STICKER_LABELS.contains(&inner)
                        || [
                            "表情包",
                            "表情",
                            "颜文字",
                            "贴纸",
                            "sticker",
                            "emoji",
                            "emoticon",
                            "emote",
                            "meme",
                        ]
                        .iter()
                        .any(|prefix| {
                            inner
                                .to_ascii_lowercase()
                                .strip_prefix(prefix)
                                .is_some_and(|suffix| suffix.starts_with([':', '：']))
                        }));
                if sticker || face_carrier(&rest[..end + closer.len_utf8()]) {
                    let mut bytes = end + closer.len_utf8();
                    // A conventional arm directly following the closed face is
                    // part of that decoration, rather than a word to pronounce.
                    while rest[bytes..]
                        .chars()
                        .next()
                        .is_some_and(|ch| matches!(ch, 'و' | 'ﾉ' | 'ノ' | '੭' | '╯' | '۶' | 'づ' | 'ง' | '✧'))
                    {
                        bytes += rest[bytes..].chars().next().expect("checked").len_utf8();
                    }
                    at += bytes;
                    continue;
                }
            }
        }
        let before = text[..at].chars().next_back();
        if let Some(face) = BARE_FACES.iter().find(|face| {
            rest.starts_with(**face)
                && before.is_none_or(|ch| !ascii_identifier(ch) && !matches!(ch, '.' | '/' | '\\'))
                && rest[face.len()..]
                    .chars()
                    .next()
                    .is_none_or(|ch| !ascii_identifier(ch) && !matches!(ch, '/' | '\\'))
                && !(rest[face.len()..].starts_with('.')
                    && rest[face.len() + 1..]
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_alphanumeric()))
        }) {
            at += face.len();
            continue;
        }
        if let Some(face) = SMILEYS.iter().find(|face| {
            rest.starts_with(**face)
                && before.is_none_or(|ch| !ascii_identifier(ch))
                && rest[face.len()..]
                    .chars()
                    .next()
                    .is_none_or(|ch| !ascii_identifier(ch))
        }) {
            at += face.len();
            continue;
        }
        out.push(ch);
        at += ch.len_utf8();
    }
    out
}

/// Sentence splitting must not cut an image or smiley into separately spoken
/// fragments. A short unfinished image is retained until its close or turn end.
pub(super) fn sentence_protected_len(text: &str) -> Option<usize> {
    if let Some(bytes) = protected_len(text).filter(|bytes| *bytes <= MAX_IMAGE_BYTES) {
        return Some(bytes);
    }
    if text.starts_with("![") {
        return image_len(text).or_else(|| (text.len() <= MAX_IMAGE_BYTES).then_some(text.len()));
    }
    if text == "!" || text == ";" || text == ";-" {
        return Some(text.len());
    }
    SMILEYS
        .iter()
        .find(|face| text.starts_with(**face))
        .map(|face| face.len())
}

/// Keep literal code, URLs and surviving Markdown images out of the stage
/// direction heuristic (for example, `[diagram]` is an image label).
pub(super) fn guard_stages(text: &str, guard: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut plain_start = 0;
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if let Some(bytes) = protected_len(rest).or_else(|| image_len(rest)) {
            out.push_str(&guard(&text[plain_start..at]));
            out.push_str(&rest[..bytes]);
            at += bytes;
            plain_start = at;
        } else {
            at += rest.chars().next().expect("non-empty").len_utf8();
        }
    }
    out.push_str(&guard(&text[plain_start..]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_recognizable_faces_and_explicit_stickers() {
        for face in [
            "(^_^)",
            "（T_T）",
            "(≧▽≦)",
            "(•̀ᴗ•́)و",
            "ಠ_ಠ",
            "^_^",
            "Q_Q",
            ">_<",
            "QAQ",
            "QwQ",
            "╥﹏╥",
            "ಥ﹏ಥ",
            "o(*￣▽￣*)o",
            "╰(*°▽°*)╯",
            "(ง •̀_•́)ง",
            "٩(๑❛ᴗ❛๑)۶",
            ":)",
            ":-)",
            ":D",
            ";)",
            ":P",
            ":/",
            ":\\",
            "[表情包：开心]",
            "【贴纸:开心】",
            "[微笑]",
            "[笑哭]",
            "![开心表情](https://example.test/happy.png)",
            "![笑][face]",
            "![😊](https://example.test/happy.png)",
        ] {
            assert_eq!(strip_decorations(face), "", "{face}");
        }
        for text in [
            "保留（括号内容）和[附录2]",
            "(x > y) 与 (a + b)",
            "$T_T + Q_Q$",
            "`T_T`",
            "```rust\nlet T_T = 1;\n```",
            "https://example.test/T_T?q=:)",
            "C:\\tmp",
            "CLASS_T_T",
            "T_T.rs",
            "/T_T/",
            "![架构图](https://example.test/diagram.png)",
            "![开心果结构图](https://example.test/happy-dataset.png)",
            "![Memento 设计模式](https://example.test/memento.png)",
            "![Remote architecture](https://example.test/remote.png)",
        ] {
            assert_eq!(strip_decorations(text), text, "{text}");
        }
    }
}
