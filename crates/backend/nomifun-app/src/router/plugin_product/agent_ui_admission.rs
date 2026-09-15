//! Host-only rollout policy. Catalog visibility is not a Session access grant.
use nomifun_plugin_platform::runtime::PluginRuntimeApplicationError;

pub(crate) const DISABLED_CODE: &str = "EXPERIMENTAL_AGENT_UI_DISABLED";
pub(crate) const DISABLED_MESSAGE: &str =
    "Plugin Agent UI is experimental; the host must set NOMIFUN_ALLOW_EXPERIMENTAL_AGENT_UI=1";

pub(crate) fn enabled() -> bool {
    opted_in(
        std::env::var("NOMIFUN_ALLOW_EXPERIMENTAL_AGENT_UI")
            .ok()
            .as_deref(),
    )
}

fn opted_in(value: Option<&str>) -> bool {
    value == Some("1")
}

pub(crate) fn require_enabled() -> Result<(), PluginRuntimeApplicationError> {
    if enabled() {
        Ok(())
    } else {
        Err(PluginRuntimeApplicationError::AgentSession {
            status: 403,
            code: DISABLED_CODE.into(),
            message: DISABLED_MESSAGE.into(),
            details: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experimental_agent_ui_requires_exact_host_opt_in() {
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("true"),
            Some("yes"),
            Some(" 1"),
            Some("1 "),
        ] {
            assert!(!opted_in(value), "must default deny: {value:?}");
        }
        assert!(opted_in(Some("1")));
    }
}
