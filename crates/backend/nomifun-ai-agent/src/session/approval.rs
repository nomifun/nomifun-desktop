//! Approval-memory key construction shared across Agent implementations.
//!
//! Keeps the `always_allow` memory key format identical between Agents
//! so the session-level approval cache stays source-of-truth-unique.

/// Build the approval memory key from action and optional command type.
pub fn approval_key(action: Option<&str>, command_type: Option<&str>) -> String {
    match (action, command_type) {
        (Some(a), Some(ct)) => format!("{a}:{ct}"),
        (Some(a), None) => a.to_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_key_formats_matches_previous_contract() {
        assert_eq!(approval_key(Some("exec"), Some("curl")), "exec:curl");
        assert_eq!(approval_key(Some("exec"), None), "exec");
        assert_eq!(approval_key(None, Some("curl")), "");
        assert_eq!(approval_key(None, None), "");
    }
}
