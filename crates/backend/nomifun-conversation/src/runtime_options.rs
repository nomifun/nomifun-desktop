//! Shared helpers for translating a [`ConversationRow`] into Agent runtime
//! construction inputs.
//!
//! Every Agent-runtime entry point — interactive `send_message`, cron, and
//! AutoWork — must derive the same typed conversation settings for a
//! given conversation; otherwise a Nomi job that runs fine
//! interactively can fail under cron with `Provider '<vendor>' not
//! found` (Sentry ELECTRON-1HM). Centralising the lookup here forces
//! both paths through one parser.
//!
//! The parser intentionally accepts both the canonical `ProviderWithModel`
//! shape and a few legacy variants (camelCase keys, `id` instead of
//! `provider_id`). When the row holds an unparseable or missing model,
//! we return an empty `ProviderWithModel`; non-nomi factory branches
//! ignore the field, and the nomi branch surfaces a clear "provider
//! not found" error against an empty id rather than a stale vendor
//! label.

use nomifun_common::{AppError, DelegationPolicy, ProviderWithModel};
use nomifun_db::models::ConversationRow;

/// Parse the authoritative conversation-level delegation policy for runtime
/// construction. Unknown persisted values are rejected rather than silently
/// widening Agent capabilities.
pub fn delegation_policy_from_conversation_row(row: &ConversationRow) -> Result<DelegationPolicy, AppError> {
    row.delegation_policy
        .parse()
        .map_err(|error| AppError::Internal(format!("Invalid conversation delegation policy: {error}")))
}

/// Resolve a conversation row's stored model into a [`ProviderWithModel`].
///
/// Returns an empty `ProviderWithModel { provider_id: "", model: "", use_model: None }`
/// when the row's `model` column is `NULL` or unparseable. This matches the
/// legacy behaviour of `ConversationService::build_runtime_options` and is the
/// canonical "no model selected" sentinel consumed by agent factories.
pub fn provider_model_from_conversation_row(row: &ConversationRow) -> ProviderWithModel {
    row.model
        .as_deref()
        .and_then(parse_provider_with_model_loose)
        .unwrap_or_else(empty_provider_model)
}

/// Canonical sentinel `ProviderWithModel` used when a conversation row has
/// no parseable model. Shared by both the interactive `send_message` path
/// and the cron executor so they agree on the "no model selected" shape:
/// `provider_id: ""`, `model: ""`, `use_model: None`. Non-nomi factories
/// ignore the field, while the nomi factory surfaces a clear "Provider
/// '' not found" error against the empty id rather than silently using a
/// stale vendor label.
pub fn empty_provider_model() -> ProviderWithModel {
    ProviderWithModel {
        provider_id: String::new(),
        model: String::new(),
        use_model: None,
    }
}

/// Permissive parser for `conversation.model` JSON.
///
/// Tries strict serde first, then falls back to manual extraction so older
/// shapes (camelCase, `id` instead of `provider_id`) keep working. Returns
/// `None` when no `provider_id` can be extracted; callers treat that as
/// "no model selected".
fn parse_provider_with_model_loose(raw: &str) -> Option<ProviderWithModel> {
    if let Ok(model) = serde_json::from_str::<ProviderWithModel>(raw) {
        return Some(model);
    }

    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let provider_id = value
        .get("provider_id")
        .or_else(|| value.get("providerId"))
        .or_else(|| value.get("id"))
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_owned();

    if provider_id.is_empty() {
        return None;
    }

    let model = value
        .get("model")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_owned();
    let use_model = value
        .get("use_model")
        .or_else(|| value.get("useModel"))
        .and_then(|item| item.as_str())
        .map(ToOwned::to_owned);

    Some(ProviderWithModel {
        provider_id,
        model,
        use_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_with_model(model: Option<&str>) -> ConversationRow {
        ConversationRow {
            id: 1,
            user_id: "user-1".into(),
            name: "test".into(),
            r#type: "nomi".into(),
            model: model.map(ToOwned::to_owned),
            extra: "{}".into(),
            delegation_policy: "automatic".into(),
            execution_model_pool: None,
            decision_policy: "automatic".into(),
            execution_template_id: None,
            status: None,
            source: None,
            channel_chat_id: None,
            pinned: false,
            pinned_at: None,
            cron_job_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn parses_canonical_shape() {
        let json = r#"{"provider_id":"abc123","model":"gpt-5","use_model":"gpt-5-turbo"}"#;
        let row = row_with_model(Some(json));
        let m = provider_model_from_conversation_row(&row);
        assert_eq!(m.provider_id, "abc123");
        assert_eq!(m.model, "gpt-5");
        assert_eq!(m.use_model.as_deref(), Some("gpt-5-turbo"));
    }

    #[test]
    fn parses_camelcase_legacy_shape() {
        let json = r#"{"providerId":"abc123","model":"gpt-5","useModel":"gpt-5-turbo"}"#;
        let row = row_with_model(Some(json));
        let m = provider_model_from_conversation_row(&row);
        assert_eq!(m.provider_id, "abc123");
        assert_eq!(m.model, "gpt-5");
        assert_eq!(m.use_model.as_deref(), Some("gpt-5-turbo"));
    }

    #[test]
    fn parses_id_alias() {
        let json = r#"{"id":"abc123","model":"gpt-5"}"#;
        let row = row_with_model(Some(json));
        let m = provider_model_from_conversation_row(&row);
        assert_eq!(m.provider_id, "abc123");
        assert_eq!(m.model, "gpt-5");
        assert!(m.use_model.is_none());
    }

    #[test]
    fn empty_provider_model_returns_documented_sentinel() {
        let m = empty_provider_model();
        assert!(m.provider_id.is_empty());
        assert!(m.model.is_empty());
        assert!(m.use_model.is_none());
    }

    #[test]
    fn null_model_returns_empty_sentinel() {
        let row = row_with_model(None);
        let m = provider_model_from_conversation_row(&row);
        assert!(m.provider_id.is_empty());
        assert!(m.model.is_empty());
        assert!(m.use_model.is_none());
    }

    #[test]
    fn invalid_json_returns_empty_sentinel() {
        let row = row_with_model(Some("not-json"));
        let m = provider_model_from_conversation_row(&row);
        assert!(m.provider_id.is_empty());
    }

    #[test]
    fn missing_provider_id_returns_empty_sentinel() {
        let json = r#"{"model":"gpt-5"}"#;
        let row = row_with_model(Some(json));
        let m = provider_model_from_conversation_row(&row);
        assert!(m.provider_id.is_empty());
    }

    #[test]
    fn typed_delegation_policy_is_parsed_from_conversation_column() {
        let mut row = row_with_model(None);
        row.delegation_policy = "prefer_parallel".into();
        assert_eq!(
            delegation_policy_from_conversation_row(&row).unwrap(),
            DelegationPolicy::PreferParallel
        );
    }

    #[test]
    fn unknown_delegation_policy_is_rejected() {
        let mut row = row_with_model(None);
        row.delegation_policy = "unbounded".into();
        assert!(delegation_policy_from_conversation_row(&row).is_err());
    }

    /// Regression: the interactive `send_message` path and the cron
    /// executor must derive the same `(provider_id, model)` for a given
    /// conversation. Before this helper existed, cron read
    /// `agent_config.backend` (which fell back to the literal vendor
    /// label `"nomi"` when the conversation's model JSON was an older
    /// shape) and `send_message` parsed the row directly, so the cron
    /// path would emit `Provider 'nomi' not found` while the
    /// interactive path used the real provider hash. Now both paths
    /// route through `provider_model_from_conversation_row` and must
    /// agree on every row shape we accept.
    #[test]
    fn interactive_and_cron_paths_agree_on_provider_id() {
        // Canonical shape (what `build_runtime_options` previously parsed strictly).
        let canonical = r#"{"provider_id":"hash-abc","model":"gpt-5","use_model":null}"#;
        // Legacy camelCase shape (what cron's loose parser previously
        // accepted but `build_runtime_options`'s strict parser rejected).
        let legacy = r#"{"providerId":"hash-abc","model":"gpt-5"}"#;

        let canonical_row = row_with_model(Some(canonical));
        let legacy_row = row_with_model(Some(legacy));

        let canonical_resolved = provider_model_from_conversation_row(&canonical_row);
        let legacy_resolved = provider_model_from_conversation_row(&legacy_row);

        // Both shapes must resolve to the same provider hash so the cron
        // executor and interactive `send_message` can never diverge.
        assert_eq!(canonical_resolved.provider_id, "hash-abc");
        assert_eq!(legacy_resolved.provider_id, "hash-abc");
        assert_eq!(canonical_resolved.provider_id, legacy_resolved.provider_id);
        // The vendor-label fallback must not leak in.
        assert_ne!(canonical_resolved.provider_id, "nomi");
        assert_ne!(legacy_resolved.provider_id, "nomi");
    }
}
