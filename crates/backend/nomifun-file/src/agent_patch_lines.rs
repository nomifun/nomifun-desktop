//! Logical patch lines retain their source terminators. Only inserted lines
//! use the first observed ending (LF for empty/single-line sources). This is a
//! pure text transform, not an alternate filesystem owner or fuzzy matcher.
use nomifun_common::AppError;

#[derive(Clone, Copy)]
pub(crate) struct PatchLine<'a> {
    pub text: &'a str,
    pub ending: &'static str,
}

pub(crate) struct PatchText<'a> {
    pub lines: Vec<PatchLine<'a>>,
    pub preferred_ending: &'static str,
    prefix: &'a str,
    trailing_newline: bool,
}

impl<'a> PatchText<'a> {
    pub fn parse(original: &'a str, max_lines: usize) -> Result<Self, AppError> {
        let (prefix, text) = match original.strip_prefix('\u{feff}') {
            Some(text) => ("\u{feff}", text),
            None => ("", original),
        };
        let mut lines = Vec::new();
        let mut first_ending = None;
        for (_, line) in logical_lines(text) {
            if lines.len() >= max_lines {
                return Err(line_limit());
            }
            if !line.ending.is_empty() {
                first_ending.get_or_insert(line.ending);
            }
            lines.push(line);
        }
        let trailing_newline = lines.last().is_some_and(|line| !line.ending.is_empty());
        Ok(Self {
            lines,
            preferred_ending: first_ending.unwrap_or("\n"),
            prefix,
            trailing_newline,
        })
    }

    /// Preserve the original EOF-newline policy, including the historical
    /// no-final-newline default for new empty-source patches. An unterminated
    /// source line moved into the middle must acquire a separator.
    pub fn render(&self, lines: &[PatchLine<'_>], max_bytes: usize) -> Result<String, AppError> {
        let mut result = self.prefix.to_owned();
        for (index, line) in lines.iter().enumerate() {
            let ending = if index + 1 == lines.len() && !self.trailing_newline {
                ""
            } else if line.ending.is_empty() {
                self.preferred_ending
            } else {
                line.ending
            };
            if result
                .len()
                .saturating_add(line.text.len())
                .saturating_add(ending.len())
                > max_bytes
            {
                return Err(AppError::BadRequest(format!(
                    "patched file exceeds the {max_bytes} byte per-file limit"
                )));
            }
            result.push_str(line.text);
            result.push_str(ending);
        }
        if result.len() > max_bytes {
            return Err(AppError::BadRequest(
                "patch text prefix exceeds the byte limit".into(),
            ));
        }
        Ok(result)
    }
}

/// Zero-allocation iterator also used by literal search. Offsets always name
/// original source bytes, including a BOM when the caller has not removed it.
pub(crate) fn logical_lines(text: &str) -> impl Iterator<Item = (usize, PatchLine<'_>)> {
    let mut at = 0;
    std::iter::from_fn(move || {
        if at == text.len() {
            return None;
        }
        let start = at;
        while at < text.len() {
            if let Some(ending) = ending_at(text.as_bytes(), at) {
                let line = PatchLine {
                    text: &text[start..at],
                    ending,
                };
                at += ending.len();
                return Some((start, line));
            }
            at += 1;
        }
        Some((
            start,
            PatchLine {
                text: &text[start..],
                ending: "",
            },
        ))
    })
}

/// Byte pages and patch hunks must agree about CR/LF logical line numbers.
/// No line allocation is needed for a page near the end of a large source.
pub(crate) fn line_position(text: &str, offset: usize) -> (usize, usize) {
    let bytes = text.as_bytes();
    let (mut at, mut line, mut line_start) = (0, 1, 0);
    while at < offset {
        if let Some(ending) = ending_at(bytes, at) {
            if at + ending.len() > offset {
                break;
            }
            at += ending.len();
            line += 1;
            line_start = at;
        } else {
            at += 1;
        }
    }
    (line, offset - line_start + 1)
}

fn ending_at(bytes: &[u8], at: usize) -> Option<&'static str> {
    match bytes.get(at) {
        Some(b'\n') => Some("\n"),
        Some(b'\r') if bytes.get(at + 1) == Some(&b'\n') => Some("\r\n"),
        Some(b'\r') => Some("\r"),
        _ => None,
    }
}

fn line_limit() -> AppError {
    AppError::BadRequest("patch source exceeds the bounded logical line count".into())
}
