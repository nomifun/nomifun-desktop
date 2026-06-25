//! Extracts a user-facing error message from a CLI subprocess's recent stderr.
//!
//! ACP child processes (codex-acp, claude-acp, …) often emit important error
//! context as `tracing` events to stderr **without** including it in the
//! JSON-RPC error response we receive. This module turns the last few stderr
//! lines into a single human-readable message, but only if the content
//! matches an allowlisted error keyword — stderr is not a trusted source.
//!
//! Returns `None` whenever no allowlisted line is found; callers must keep
//! their existing error string in that case.

/// Allowlisted lowercase keywords. A stderr line is considered "user-relevant"
/// only if it contains at least one of these (case-insensitive).
///
/// Order does not matter for matching, but keep this short and audited — every
/// new entry expands what stderr content can reach end-users.
const ERROR_KEYWORDS: &[&str] = &[
    "usage limit",
    "rate limit",
    "exceeded",
    "unauthorized",
    "forbidden",
    "network",
    "timeout",
    "connection refused",
    "connection reset",
    "tls",
    "dns",
    // HIGH-RISK: may surface token/key values from stderr (e.g. "credentials:
    // Bearer eyJ..."). The 240-char cap does not protect a 40-60 char secret.
    // Keep matched lines out of operator logs that get exfiltrated.
    "credentials",
    "api key",
    "quota",
    "billing",
];

const MAX_MESSAGE_CHARS: usize = 240;

/// Strip ANSI CSI escape sequences (`\u{1b}[...m` and similar) from `s`.
///
/// Minimal implementation — handles the SGR (`m`) terminator that `tracing`'s
/// ANSI subscriber uses. Other CSI commands (cursor moves etc.) are stripped
/// the same way as long as they end in `[A-Za-z]`.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.chars().peekable();
    while let Some(c) = iter.next() {
        if c == '\u{1b}' && matches!(iter.peek(), Some('[')) {
            iter.next(); // consume '['
            for ch in iter.by_ref() {
                if ch.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Extract the user-relevant message tail from a single stripped tracing line.
///
/// Tracing format roughly: `<ts> <LEVEL> <target>{<fields>}: <message>`.
/// We split on `": "` and take the last segment — that's almost always the
/// message. Returns the trimmed segment.
fn message_tail(stripped_line: &str) -> &str {
    stripped_line
        .rsplit_once(": ")
        .map(|(_, tail)| tail)
        .unwrap_or(stripped_line)
        .trim()
}

fn matches_allowlist(line_lower: &str) -> bool {
    ERROR_KEYWORDS.iter().any(|kw| line_lower.contains(kw))
}

/// Pick the most informative error line from a chunk of recent stderr.
///
/// Returns `None` if nothing matches the allowlist — caller should keep its
/// existing error message rather than substitute an empty string.
pub(super) fn extract_error_message(stderr_tail: &str) -> Option<String> {
    if stderr_tail.trim().is_empty() {
        return None;
    }

    let lines: Vec<String> = stderr_tail
        .lines()
        .map(strip_ansi)
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();

    // Pass 1: latest line whose stripped form contains "ERROR" AND matches the allowlist.
    let mut chosen: Option<&str> = None;
    for line in lines.iter().rev() {
        let lower = line.to_lowercase();
        if lower.contains("error") && matches_allowlist(&lower) {
            chosen = Some(line.as_str());
            break;
        }
    }
    // Pass 2: latest line that matches the allowlist regardless of level (e.g. WARN).
    if chosen.is_none() {
        for line in lines.iter().rev() {
            let lower = line.to_lowercase();
            if matches_allowlist(&lower) {
                chosen = Some(line.as_str());
                break;
            }
        }
    }

    let line = chosen?;
    let tail = message_tail(line);
    let truncated: String = if tail.chars().count() > MAX_MESSAGE_CHARS {
        let mut buf: String = tail.chars().take(MAX_MESSAGE_CHARS - 1).collect();
        buf.push('…');
        buf
    } else {
        tail.to_owned()
    };
    Some(truncated)
}

#[cfg(test)]
mod tests {
    use super::extract_error_message;

    const STDERR_USAGE_LIMIT: &str = "\u{1b}[2m2026-05-13T20:01:21.330370Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m \u{1b}[2mcodex_acp::thread\u{1b}[0m\u{1b}[2m:\u{1b}[0m Unhandled error during turn: You've hit your usage limit. To get more access now, send a request to your admin or try again at May 14th, 2026 8:16 PM. Some(UsageLimitExceeded)";

    #[test]
    fn extracts_usage_limit_message() {
        let result = extract_error_message(STDERR_USAGE_LIMIT);
        let msg = result.expect("usage limit must match");
        assert!(msg.contains("usage limit"), "got {msg}");
        assert!(!msg.contains("\u{1b}["), "ANSI escapes must be stripped; got {msg:?}");
        assert!(
            !msg.contains("ERROR"),
            "tracing level prefix should not leak into user message; got {msg}"
        );
    }

    #[test]
    fn returns_none_for_unrelated_stderr() {
        // No allowlisted keywords → return None, do not pretend.
        let stderr = "ERROR widget_loader: failed to load widget xyz";
        assert!(extract_error_message(stderr).is_none());
    }

    #[test]
    fn returns_none_for_empty_input() {
        assert!(extract_error_message("").is_none());
        assert!(extract_error_message("   \n\n  ").is_none());
    }

    #[test]
    fn truncates_overlong_message_to_240_chars() {
        let mut long = String::from("ERROR upstream: usage limit exceeded ");
        long.push_str(&"x".repeat(500));
        let result = extract_error_message(&long).expect("matched on usage limit");
        assert!(
            result.chars().count() <= 240,
            "result must be ≤240 chars; got {} chars",
            result.chars().count()
        );
    }

    #[test]
    fn prefers_error_over_warn_lines() {
        let stderr = "WARN widget: usage limit warning happens\n\
                      ERROR upstream: network connection refused\n\
                      WARN cleanup: usage limit cleanup ran";
        let result = extract_error_message(stderr).expect("ERROR line must match");
        assert!(
            result.contains("network connection refused"),
            "ERROR line should win over WARN; got {result}"
        );
    }

    #[test]
    fn falls_back_to_warn_when_no_matching_error() {
        let stderr = "ERROR widget: something unrelated happened\n\
                      WARN upstream: rate limit exceeded for token xyz";
        let result = extract_error_message(stderr).expect("WARN match must surface");
        assert!(result.contains("rate limit"), "got {result}");
    }

    #[test]
    fn picks_latest_matching_line_when_multiple() {
        let stderr = "ERROR upstream: connection timeout 1\n\
                      ERROR upstream: connection timeout 2 (latest)";
        let result = extract_error_message(stderr).expect("must match");
        assert!(
            result.contains("(latest)"),
            "must pick the most recent matching line; got {result}"
        );
    }

    #[test]
    fn strips_ansi_then_keeps_only_message_after_last_colon() {
        // tracing format: "<timestamp> <LEVEL> <target>: <fields>: <message>"
        // We want just the message tail.
        let stderr =
            "\u{1b}[2m2026-05-13T20:01:21Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m foo::bar: ctx=abc: usage limit exceeded";
        let result = extract_error_message(stderr).expect("must match");
        assert_eq!(result, "usage limit exceeded");
    }

    #[test]
    fn handles_line_without_colon_separator() {
        // Some logs aren't tracing-formatted; we should still surface the line
        // when it matches the allowlist, instead of silently mangling it.
        let stderr = "usage limit exceeded no colon here";
        let result = extract_error_message(stderr).expect("should match");
        assert_eq!(result, "usage limit exceeded no colon here");
    }
}
