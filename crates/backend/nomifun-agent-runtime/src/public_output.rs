const TOOL_TAG: &str = "<tool_call>";
const FUNCTION_TAG: &str = "<function=";

/// Keep a split protocol marker out of public output until it is classified.
/// Native ToolCall events bypass this text-only guard and retain their normal
/// admission path. Fenced documentation examples remain ordinary text.
#[derive(Default)]
pub(crate) struct PublicOutputGuard {
    pending: String,
    published: String,
}

pub(crate) struct PublicOutputChunk {
    pub text: String,
    pub invalid_tool_call: bool,
    /// Name only, never parsed arguments or executable text.
    pub tool_hint: Option<String>,
}

fn in_code_fence(text: &str) -> bool {
    let mut fence: Option<(u8, usize)> = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 3 { continue; }
        let line = &line[indent..];
        let Some(marker @ (b'`' | b'~')) = line.as_bytes().first().copied() else { continue; };
        let count = line.bytes().take_while(|value| *value == marker).count();
        if count < 3 { continue; }
        match fence {
            None => fence = Some((marker, count)),
            Some((open, size)) if marker == open && count >= size => fence = None,
            _ => {}
        }
    }
    fence.is_some()
}

impl PublicOutputGuard {
    pub fn push(&mut self, text: &str) -> PublicOutputChunk {
        self.pending.push_str(text);
        let mut output = String::new();
        let mut invalid_tool_call = false;
        let mut tool_hint = None;
        loop {
            let lower = self.pending.to_ascii_lowercase();
            if let Some(start) = lower.find(TOOL_TAG) {
                let prefix = self.pending[..start].to_owned();
                let fenced = in_code_fence(&format!("{}{}{}", self.published, output, prefix));
                let after = lower[start + TOOL_TAG.len()..].trim_start();
                if !fenced && after.starts_with(FUNCTION_TAG) {
                    let raw = self.pending[start + TOOL_TAG.len()..].trim_start();
                    let name = &raw[FUNCTION_TAG.len()..];
                    // Retain only the short name prefix across stream frames;
                    // never wait for or interpret XML arguments.
                    if !name.contains('>') && name.len() <= 256 {
                        output.push_str(&prefix);
                        self.pending.drain(..start);
                        break;
                    }
                    tool_hint = name.split_once('>').map(|(name,_)| name.trim())
                        .filter(|name| !name.is_empty() && name.len() <= 256
                            && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c)))
                        .map(str::to_owned);
                    output.push_str(&prefix);
                    self.pending.clear();
                    invalid_tool_call = true;
                    break;
                }
                if !fenced && (after.is_empty() || FUNCTION_TAG.starts_with(after)) {
                    output.push_str(&prefix);
                    self.pending.drain(..start);
                    break;
                }
                output.push_str(&self.pending.drain(..start + TOOL_TAG.len()).collect::<String>());
                continue;
            }
            let retain = (1..TOOL_TAG.len()).rev()
                .find(|length| lower.ends_with(&TOOL_TAG[..*length])).unwrap_or(0);
            let publish_len = self.pending.len() - retain;
            output.push_str(&self.pending.drain(..publish_len).collect::<String>());
            break;
        }
        self.published.push_str(&output);
        PublicOutputChunk { text: output, invalid_tool_call, tool_hint }
    }

    pub fn has_pending_native_call(&self) -> bool {
        self.pending.to_ascii_lowercase().trim_start().strip_prefix(TOOL_TAG)
            .is_some_and(|after| after.trim_start().starts_with(FUNCTION_TAG))
    }

    pub fn finish(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_pseudo_tool_call_fails_before_arguments_reach_the_transcript() {
        let mut guard = PublicOutputGuard::default();
        let mut visible = String::new();
        for part in ["I will write it. <to", "ol_call>", "\n<fun", "ction=write_file>\n<parameter=content>SECRET_FILE_BODY"] {
            let chunk = guard.push(part);
            visible.push_str(&chunk.text);
            if chunk.invalid_tool_call {
                assert_eq!(visible, "I will write it. ");
                assert_eq!(chunk.tool_hint.as_deref(),Some("write_file"));
                return;
            }
        }
        panic!("pseudo tool call was accepted");
    }

    #[test]
    fn ordinary_progress_is_immediate_and_fenced_protocol_examples_survive() {
        let mut guard = PublicOutputGuard::default();
        assert_eq!(guard.push("Checking the result.").text, "Checking the result.");
        let example = "\n```xml\n<tool_call>\n<function=write_file>\n</tool_call>\n```";
        let mut visible = String::new();
        for part in example.as_bytes().chunks(3) {
            let chunk = guard.push(std::str::from_utf8(part).unwrap());
            assert!(!chunk.invalid_tool_call);
            visible.push_str(&chunk.text);
        }
        visible.push_str(&guard.finish());
        assert_eq!(visible, example);
    }

    #[test]
    fn streaming_name_is_retained_without_publishing_or_parsing_arguments() {
        let mut guard = PublicOutputGuard::default();
        for part in ["<tool_call>","<function=","write_","file"] {
            let chunk=guard.push(part);
            assert!(chunk.text.is_empty());
            assert!(!chunk.invalid_tool_call);
        }
        assert!(guard.has_pending_native_call());
        let chunk=guard.push("><parameter=content>PRIVATE_BODY");
        assert!(chunk.invalid_tool_call);
        assert_eq!(chunk.tool_hint.as_deref(),Some("write_file"));
        assert!(chunk.text.is_empty());
    }
}
