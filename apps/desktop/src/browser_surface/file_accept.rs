//! HTML accept is a picker hint, not file authority or server validation.
//! Only normalized extensions cross into the native Shell filter grammar.
use std::collections::BTreeSet;

pub(super) fn valid_extension(extension: &str) -> bool {
    !extension.is_empty()
        && extension.len() <= 64
        && extension.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '+'))
        })
}
pub(super) fn valid_extensions(extensions: &[String]) -> bool {
    extensions.len() <= 256
        && extensions
            .iter()
            .all(|extension| valid_extension(extension))
        && extensions
            .iter()
            .map(|extension| extension.len() + 3)
            .sum::<usize>()
            <= 4096
}
pub(super) fn extensions(accept: &str) -> Vec<String> {
    if accept.len() > 4096 {
        return vec![];
    }
    let mut extensions = BTreeSet::new();
    for token in accept.split(',') {
        let token = token
            .trim_matches(|ch: char| ch.is_ascii_whitespace())
            .to_ascii_lowercase();
        if let Some(extension) = token.strip_prefix('.') {
            if valid_extension(extension) {
                extensions.insert(extension.to_owned());
            }
        } else if matches!(token.as_str(), "image/*" | "audio/*" | "video/*")
            || (!token.contains(['*', ';']) && token.split('/').count() == 2)
        {
            if let Some(known) = mime_guess::get_mime_extensions_str(&token) {
                extensions.extend(
                    known
                        .iter()
                        .filter(|extension| valid_extension(extension))
                        .map(|extension| (*extension).to_owned()),
                );
            }
        }
        if extensions.len() > 256 {
            return vec![];
        }
    }
    let extensions = extensions.into_iter().collect::<Vec<_>>();
    // A very broad hint remains unrestricted, rather than silently omitting
    // accepted types to fit an arbitrary partial list. All-files stays available.
    if valid_extensions(&extensions) {
        extensions
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extensions_and_mime_types_are_normalized_and_deduplicated() {
        let accepted = extensions(" .TXT, text/plain, .txt, image/png, .tar.gz, .文档 ");
        for expected in ["txt", "png", "tar.gz", "文档"] {
            assert!(accepted.contains(&expected.into()));
        }
        assert_eq!(
            accepted
                .iter()
                .filter(|extension| extension.as_str() == "txt")
                .count(),
            1
        );
        for (mime, extension) in [("image/*", "png"), ("audio/*", "mp3"), ("video/*", "mp4")] {
            assert!(extensions(mime).contains(&extension.into()));
        }
    }
    #[test]
    fn web_hints_cannot_inject_shell_patterns_or_paths() {
        for accept in [
            ".*",
            ".txt;*.*",
            ".a/b",
            ".a\\b",
            ".c:txt",
            ".a?",
            ".a\0b",
            "..txt",
            "application/*",
            "text/plain;charset=utf-8",
        ] {
            assert!(extensions(accept).is_empty(), "{accept:?}");
        }
        assert_eq!(extensions("unknown,application/unknown,.csv"), vec!["csv"]);
        assert!(extensions(&"x".repeat(4097)).is_empty());
        assert!(!valid_extensions(&vec!["txt".into(); 257]));
    }
}
