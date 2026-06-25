pub mod fold;
pub mod json;
pub mod level;
pub mod sanitize;
pub mod toon;

pub use level::CompactionLevel;
pub use toon::toon_format_instructions;

pub fn compact_output(text: &str, level: CompactionLevel) -> String {
    match level {
        CompactionLevel::Off => text.to_string(),
        CompactionLevel::Safe => sanitize::sanitize(text),
        CompactionLevel::Full => {
            let text = sanitize::sanitize(text);
            let text = fold::fold_repeated_lines(&text);
            json::compact_json(&text)
        }
    }
}

pub fn compact_output_toon(text: &str) -> String {
    toon::try_toon_encode(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_returns_unchanged() {
        let input = "hello\x1b[31m world\n\n\nfoo";
        assert_eq!(compact_output(input, CompactionLevel::Off), input);
    }

    #[test]
    fn safe_strips_ansi() {
        let input = "\x1b[32mOK\x1b[0m done";
        let result = compact_output(input, CompactionLevel::Safe);
        assert_eq!(result, "OK done");
    }

    #[test]
    fn safe_merges_blank_lines() {
        let input = "a\n\n\n\nb";
        let result = compact_output(input, CompactionLevel::Safe);
        assert_eq!(result, "a\n\nb");
    }

    #[test]
    fn safe_collapses_cr() {
        let input = "50%\r100%\nDone";
        let result = compact_output(input, CompactionLevel::Safe);
        assert_eq!(result, "100%\nDone");
    }

    #[test]
    fn full_folds_repeated_lines() {
        let lines: Vec<String> = (0..6)
            .map(|i| format!("Compiling dep-{i} v0.1.0"))
            .collect();
        let input = lines.join("\n");
        let result = compact_output(&input, CompactionLevel::Full);
        assert!(result.contains("[... 4 similar lines]"));
    }

    #[test]
    fn full_compacts_json() {
        let input = "{\n    \"id\": 1,\n    \"name\": \"Alice\"\n}";
        let result = compact_output(input, CompactionLevel::Full);
        assert!(result.len() < input.len());
    }

    #[test]
    fn safe_does_not_fold_lines() {
        let lines: Vec<String> = (0..6)
            .map(|i| format!("Compiling dep-{i} v0.1.0"))
            .collect();
        let input = lines.join("\n");
        let result = compact_output(&input, CompactionLevel::Safe);
        assert!(!result.contains("[..."), "Safe level should not fold lines");
    }
}
