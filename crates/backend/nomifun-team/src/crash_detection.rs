//! Classify fatal agent stream events into recoverable (rate-limited) vs crash.
//!
//! Paired with W4-D20a `detect_crash`. Rate-limit errors are surfaced as
//! `TeammateStatus::Failed` without going through crash recovery (no kill,
//! no testament) — see interface-contracts §23.

use nomifun_ai_agent::protocol::events::AgentStreamEvent;
use regex::Regex;
use std::sync::OnceLock;

fn rate_limit_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)429|rate.?limit|quota|too many requests").expect("rate-limit regex must compile")
    })
}

/// Returns true when an [`AgentStreamEvent::Error`] message looks like an
/// upstream rate-limit / quota response.
pub fn is_rate_limited(event: &AgentStreamEvent) -> bool {
    match event {
        AgentStreamEvent::Error(data) => rate_limit_regex().is_match(&data.message),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_ai_agent::protocol::events::{ErrorEventData, StartEventData};

    fn error_event(message: &str) -> AgentStreamEvent {
        AgentStreamEvent::Error(ErrorEventData::legacy(message, None))
    }

    #[test]
    fn http_429_is_rate_limited() {
        assert!(is_rate_limited(&error_event("HTTP 429 Too Many Requests")));
    }

    #[test]
    fn rate_limit_phrase_is_rate_limited() {
        assert!(is_rate_limited(&error_event(
            "Anthropic API: rate limit exceeded, retry later"
        )));
    }

    #[test]
    fn plain_error_is_not_rate_limited() {
        assert!(!is_rate_limited(&error_event("syntax error at line 42")));
    }

    #[test]
    fn non_error_event_is_not_rate_limited() {
        assert!(!is_rate_limited(&AgentStreamEvent::Start(StartEventData::default())));
    }
}

// ---------------------------------------------------------------------------
// Crash detection (W4-D20a)
// ---------------------------------------------------------------------------

/// Reason an agent was classified as crashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrashReason {
    ProcessExited,
    SessionNotFound,
    Unknown(String),
}

/// Detect crash from an agent stream event.
/// Returns Some(reason) if the event indicates a crash, None otherwise.
pub fn detect_crash(event: &AgentStreamEvent) -> Option<CrashReason> {
    match event {
        AgentStreamEvent::Error(data) => {
            let msg = &data.message;
            if msg.contains("process exited unexpectedly") || msg.contains("process exited") {
                Some(CrashReason::ProcessExited)
            } else if msg.contains("Session not found") || msg.contains("session not found") {
                Some(CrashReason::SessionNotFound)
            } else {
                Some(CrashReason::Unknown(msg.clone()))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod crash_tests {
    use super::*;
    use nomifun_ai_agent::protocol::events::ErrorEventData;

    #[test]
    fn detect_crash_process_exited() {
        let event = AgentStreamEvent::Error(ErrorEventData::legacy("process exited unexpectedly", None));
        assert_eq!(detect_crash(&event), Some(CrashReason::ProcessExited));
    }

    #[test]
    fn detect_crash_session_not_found() {
        let event = AgentStreamEvent::Error(ErrorEventData::legacy("Session not found", None));
        assert_eq!(detect_crash(&event), Some(CrashReason::SessionNotFound));
    }

    #[test]
    fn detect_crash_other_error() {
        let event = AgentStreamEvent::Error(ErrorEventData::legacy("something else broke", None));
        assert_eq!(
            detect_crash(&event),
            Some(CrashReason::Unknown("something else broke".into()))
        );
    }

    #[test]
    fn detect_crash_non_error_returns_none() {
        let event = AgentStreamEvent::Start(nomifun_ai_agent::protocol::events::StartEventData { session_id: None });
        assert_eq!(detect_crash(&event), None);
    }
}
