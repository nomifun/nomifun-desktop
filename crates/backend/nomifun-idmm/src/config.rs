//! Config-derived classification helpers + validation. The persisted config DTOs
//! themselves live in `nomifun_api_types::idmm`; this module adds runtime logic
//! over them.

use nomifun_api_types::{AgentErrorCode, IdmmConfig, WatchTier};
use nomifun_common::ProviderId;

/// Classify an `AgentErrorCode` as a provider fault IDMM should supervise
/// (i.e. a single-vendor failure a backup model or a retry might overcome).
pub fn is_provider_fault(code: AgentErrorCode) -> bool {
    use AgentErrorCode::*;
    matches!(
        code,
        UserLlmProviderAuthFailed
            | UserLlmProviderPermissionDenied
            | UserLlmProviderBillingRequired
            | UserLlmProviderConfigError
            | UserLlmProviderModelNotFound
            | UserLlmProviderUnsupportedModel
            | UserLlmProviderEndpointNotFound
            | UserLlmProviderInvalidRequest
            | UserLlmProviderInvalidToolSchema
            | UserLlmProviderContextTooLarge
            | UserLlmProviderRateLimited
            | UserLlmProviderTimeout
            | UserLlmProviderNetworkError
            | UserLlmProviderEmptyResponse
            | UserLlmProviderGatewayError
            | UnknownUpstreamError
    )
}

/// Validate a config for the given backup resolvability. Returns `Err(reason)`
/// to map to a 400 / inline UI error.
///
/// Phase 2 (plan D4 / config.rs validate): validation is **per-watch**. A watch
/// only carries operational requirements when it is enabled. The single hard
/// prerequisite is the `RulePlusModel` tier's resolvable backup model — and the
/// caller computes `backup_resolvable` inclusively (per-watch override → global
/// default → the conversation's own model), so a plain desktop chat satisfies it
/// with zero extra config ("智能值守、全托管" is one click). The strategy's
/// steering / freeform text is OPTIONAL: when empty, the sidecar prompt falls
/// back to a conservative built-in policy ([`crate::prompt::build_user_prompt`]),
/// so requiring it only added friction. A disabled watch never runs and so
/// carries no requirements (users must always be able to turn a watch off, even
/// from a half-filled model form).
pub fn validate(cfg: &IdmmConfig, backup_resolvable: bool) -> Result<(), String> {
    for (label, provider_id) in [
        (
            "fault_watch",
            cfg.fault_watch.base.bypass_model.provider_id.as_deref(),
        ),
        (
            "decision_watch",
            cfg.decision_watch.base.bypass_model.provider_id.as_deref(),
        ),
    ] {
        if let Some(provider_id) = provider_id {
            ProviderId::parse(provider_id).map_err(|error| {
                format!("{label} bypass provider_id is not canonical: {error}")
            })?;
        }
    }
    let fault_needs_backup =
        cfg.fault_watch.base.enabled && cfg.fault_watch.base.tier == WatchTier::RulePlusModel;
    let decision_needs_backup =
        cfg.decision_watch.base.enabled && cfg.decision_watch.base.tier == WatchTier::RulePlusModel;
    if (fault_needs_backup || decision_needs_backup) && !backup_resolvable {
        return Err(
            "no backup model resolvable for the 旁路模型 (RulePlusModel) tier — pick a per-watch bypass model, set a \
             global default (设置 → 智能决策), or enable it on a conversation that already has a model selected"
                .into(),
        );
    }
    Ok(())
}

/// Whether an answer/action text looks destructive (vetoed unless explicitly
/// allowed). Case-insensitive substring match on common irreversible operations.
pub fn is_destructive(text: &str) -> bool {
    let low = text.to_lowercase();
    const SIGS: &[&str] = &[
        "rm -rf",
        "rm -fr",
        "drop table",
        "drop database",
        "truncate",
        "delete from",
        "force push",
        "push --force",
        "push -f",
        "reset --hard",
        "git clean -",
        "mkfs",
        "dd if=",
        "> /dev/",
    ];
    SIGS.iter().any(|s| low.contains(s))
}

/// Whether a decision option looks like a cancel / decline / skip choice.
/// `auto_pick_unmarked` must never auto-select one of these — the point of the
/// conservative auto-pick is to PROCEED with a real option, not to bail. When
/// every option is a cancel (or destructive) one, the policy falls through to
/// the sidecar / halt instead. Case-insensitive substring match.
pub fn is_cancel_option(text: &str) -> bool {
    let low = text.to_lowercase();
    const SIGS: &[&str] = &[
        "取消",
        "放弃",
        "跳过",
        "稍后",
        "暂不",
        "退出",
        "以后再",
        "什么都不",
        "都不选",
        "不需要",
        "cancel",
        "skip",
        "abort",
        "quit",
        "go back",
        "none of",
        "do nothing",
        "nevermind",
        "never mind",
    ];
    SIGS.iter().any(|s| low.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::{DecisionWatchConfig, FaultWatchConfig, IdmmConfig, WatchBase, WatchTier};

    fn decision_model_watch(enabled: bool) -> DecisionWatchConfig {
        DecisionWatchConfig {
            base: WatchBase {
                enabled,
                tier: WatchTier::RulePlusModel,
                ..WatchBase::default()
            },
            ..Default::default()
        }
    }

    fn fault_model_watch(enabled: bool) -> FaultWatchConfig {
        FaultWatchConfig {
            base: WatchBase {
                enabled,
                tier: WatchTier::RulePlusModel,
                ..WatchBase::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn validate_freeform_optional_when_model_tier() {
        // The strategy's freeform policy is no longer mandatory for the model
        // tier — an empty policy falls back to the conservative built-in. Only a
        // resolvable backup model is required.
        let mut cfg = IdmmConfig {
            decision_watch: decision_model_watch(true),
            ..Default::default()
        };
        assert!(validate(&cfg, true).is_ok(), "empty freeform must be allowed when backup resolves");
        cfg.decision_watch.strategy.freeform_policy = Some("prefer recommended".into());
        assert!(validate(&cfg, true).is_ok());
    }

    #[test]
    fn validate_requires_backup_when_decision_model_tier() {
        let cfg = IdmmConfig {
            decision_watch: decision_model_watch(true),
            ..Default::default()
        };
        assert!(validate(&cfg, false).is_err());
        assert!(validate(&cfg, true).is_ok());
    }

    #[test]
    fn validate_requires_backup_when_fault_model_tier() {
        let cfg = IdmmConfig {
            fault_watch: fault_model_watch(true),
            ..Default::default()
        };
        assert!(validate(&cfg, false).is_err());
        assert!(validate(&cfg, true).is_ok());
    }

    #[test]
    fn validate_ok_rule_only_regardless_of_backup() {
        // Both watches RuleOnly (default tier) + enabled → no backup required.
        let cfg = IdmmConfig {
            fault_watch: FaultWatchConfig {
                base: WatchBase { enabled: true, ..WatchBase::default() },
                ..Default::default()
            },
            decision_watch: DecisionWatchConfig {
                base: WatchBase { enabled: true, ..WatchBase::default() },
                ..Default::default()
            },
        };
        assert!(validate(&cfg, false).is_ok());
    }

    // ── A disabled watch must always be allowed through (turn-off must work) ──

    #[test]
    fn validate_allows_disable_without_backup() {
        // The user picked the model tier for the decision watch but left it
        // disabled (toggled off). This MUST succeed — an inactive watch carries
        // no operational requirements.
        let cfg = IdmmConfig {
            decision_watch: decision_model_watch(false),
            ..Default::default()
        };
        assert!(validate(&cfg, false).is_ok());
        assert!(validate(&cfg, true).is_ok());
    }

    #[test]
    fn is_provider_fault_covers_known_codes() {
        assert!(is_provider_fault(AgentErrorCode::UserLlmProviderEndpointNotFound));
        assert!(is_provider_fault(AgentErrorCode::UserLlmProviderGatewayError));
        assert!(is_provider_fault(AgentErrorCode::UserLlmProviderRateLimited));
        assert!(!is_provider_fault(AgentErrorCode::UserAgentNotInstalled));
        assert!(!is_provider_fault(AgentErrorCode::NomifunConversationBusy));
    }

    #[test]
    fn is_destructive_flags_dangerous_text() {
        assert!(is_destructive("run rm -rf /tmp/x"));
        assert!(is_destructive("git reset --hard origin/main"));
        assert!(is_destructive("DROP TABLE users"));
        assert!(!is_destructive("yes, continue"));
        assert!(!is_destructive("option 1"));
    }

    #[test]
    fn is_cancel_option_flags_decline_choices() {
        assert!(is_cancel_option("1) 取消"));
        assert!(is_cancel_option("3) 跳过此步"));
        assert!(is_cancel_option("2) Cancel and try later"));
        assert!(is_cancel_option("None of the above"));
        assert!(!is_cancel_option("1) Canvas 渲染"));
        assert!(!is_cancel_option("2) 方案B：双写过渡"));
    }
}
