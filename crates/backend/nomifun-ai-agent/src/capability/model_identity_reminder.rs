//! Renders a `<system-reminder>` block that tells the CLI-hosted LLM
//! the actual model it is currently running as. Used when
//! `BehaviorPolicy::self_identity_sticky` is set, because those CLIs
//! cache model identity in the session system prompt at launch and
//! `session/set_model` does not refresh it.

/// Produce the reminder prefix to prepend before the next user prompt.
/// The returned string ends with `\n\n` so the user content can be
/// appended directly.
pub fn render_model_identity_reminder(model_label: &str) -> String {
    format!(
        "<system-reminder>\n\
         Model switch: The active model has been changed to {label} via the /model command. \
         You are now running as {label}. \
         The earlier \"You are powered by\" text in the system prompt is cached from session start and no longer reflects the actual model. \
         When asked which model you are, answer {label}.\n\
         </system-reminder>\n\n",
        label = model_label,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_label() {
        let out = render_model_identity_reminder("us.anthropic.claude-opus-4-6-v1");
        assert!(out.starts_with("<system-reminder>"));
        assert!(out.contains("running as us.anthropic.claude-opus-4-6-v1"));
        assert!(out.ends_with("</system-reminder>\n\n"));
    }

    #[test]
    fn output_is_safely_composable_before_user_content() {
        let prefix = render_model_identity_reminder("opus");
        let combined = format!("{prefix}hello");
        assert!(combined.ends_with("\n\nhello"));
    }
}
