use nomifun_api_types::AgentMetadata;

pub(super) fn normalize_requested_mode(metadata: &AgentMetadata, mode: &str) -> String {
    let trimmed = mode.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Nomi persists the legacy aliases `yolo` / `yoloNoSandbox` while
    // ACP backends expect their native mode id (e.g. `agent-full-access`
    // for Codex). Resolution is data-driven: the mapping lives on each
    // catalog row's top-level `yolo_id` column. Backends without a
    // `yolo_id` have no equivalent, so the alias passes through
    // unchanged and `session/set_mode` gets the caller's original
    // value.
    let resolved = if matches!(trimmed, "yolo" | "yoloNoSandbox")
        && let Some(native) = metadata.yolo_id.as_deref()
    {
        native
    } else {
        trimmed
    };

    // Codex mode-id lineage. The bridge swap (migration 022, from the
    // archived @zed-industries/codex-acp to @agentclientprotocol/codex-acp)
    // renamed the native session modes:
    //     read-only   -> read-only          (unchanged)
    //     auto        -> agent
    //     full-access -> agent-full-access
    // Persisted sessions/config — and rows whose `yolo_id` still carries the
    // pre-022 value — may present any historical alias (`default`/`autoEdit`
    // predate even the old bridge), so fold the whole lineage onto the
    // current native ids AFTER yolo resolution. Keyed on the vendor backend
    // label rather than re-introducing an AcpBackend enum variant.
    if matches!(metadata.backend.as_deref(), Some("codex")) {
        match resolved {
            "default" | "autoEdit" | "auto" | "agent" => return "agent".to_owned(),
            "full-access" | "agent-full-access" => return "agent-full-access".to_owned(),
            _ => {}
        }
    }

    resolved.to_owned()
}

/// Whether the agent resumes a session by calling `session/new` again
/// with a vendor-specific `_meta.<vendor>.options.resume` field, instead
/// of the generic ACP `session/load` method.
///
/// Returns the bool on `metadata.behavior_policy` verbatim — the
/// catalog row is the single source of truth. No backend-name
/// sniffing, no handshake blob inspection.
pub(super) fn agent_metadata_uses_meta_resume(metadata: &AgentMetadata) -> bool {
    metadata.behavior_policy.session_load_via_meta_field
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::AgentHandshake;
    use nomifun_common::AgentType;

    fn metadata_with_yolo_id(yolo_id: Option<&str>) -> AgentMetadata {
        use nomifun_api_types::{AgentSource, AgentSourceInfo, BehaviorPolicy};
        AgentMetadata {
            agent_id: "0190f5fe-7c00-7a00-8000-000000000102".into(),
            icon: None,
            name: "Test".into(),
            name_i18n: None,
            description: None,
            description_i18n: None,
            backend: None,
            agent_type: AgentType::Acp,
            agent_source: AgentSource::Builtin,
            agent_source_info: AgentSourceInfo::default(),
            enabled: true,
            available: true,
            command: None,
            resolved_command: None,
            args: vec![],
            env: vec![],
            native_skills_dirs: None,
            behavior_policy: BehaviorPolicy::default(),
            yolo_id: yolo_id.map(ToOwned::to_owned),
            sort_order: 3130,
            handshake: AgentHandshake::default(),
        }
    }

    #[test]
    fn normalize_requested_mode_rewrites_yolo_when_behavior_policy_maps_it() {
        let meta = metadata_with_yolo_id(Some("full-access"));
        assert_eq!(normalize_requested_mode(&meta, "yolo"), "full-access");
        assert_eq!(normalize_requested_mode(&meta, "yoloNoSandbox"), "full-access");
    }

    #[test]
    fn normalize_requested_mode_passes_through_when_no_yolo_id() {
        let meta = metadata_with_yolo_id(None);
        // No mapping configured — aliases flow through unchanged.
        assert_eq!(normalize_requested_mode(&meta, "yolo"), "yolo");
        assert_eq!(normalize_requested_mode(&meta, "yoloNoSandbox"), "yoloNoSandbox");
    }

    #[test]
    fn normalize_requested_mode_passes_through_non_yolo_modes() {
        let meta = metadata_with_yolo_id(Some("full-access"));
        // Non-codex rows: aliases flow through untouched.
        assert_eq!(normalize_requested_mode(&meta, "default"), "default");
        assert_eq!(normalize_requested_mode(&meta, "read-only"), "read-only");
        assert_eq!(
            normalize_requested_mode(&meta, "bypassPermissions"),
            "bypassPermissions"
        );
    }

    /// Vendor-specific yolo rewrites are entirely data-driven by
    /// `metadata.yolo_id`. Rebuild fixtures with the seed values
    /// the v3 baseline would hydrate, then assert both yolo
    /// aliases hit the native mode id for each vendor.
    #[test]
    fn normalize_requested_mode_rewrites_yolo_for_builtin_vendors() {
        // Claude / Codebuddy → bypassPermissions.
        let claude_like = metadata_with_yolo_id(Some("bypassPermissions"));
        assert_eq!(normalize_requested_mode(&claude_like, "yolo"), "bypassPermissions");
        assert_eq!(
            normalize_requested_mode(&claude_like, "yoloNoSandbox"),
            "bypassPermissions"
        );
        // Opencode → build.
        let opencode_like = metadata_with_yolo_id(Some("build"));
        assert_eq!(normalize_requested_mode(&opencode_like, "yolo"), "build");
        // Cursor → agent.
        let cursor_like = metadata_with_yolo_id(Some("agent"));
        assert_eq!(normalize_requested_mode(&cursor_like, "yolo"), "agent");
        // When a row has no yolo_id the alias flows through unchanged.
        let gemini_like = metadata_with_yolo_id(None);
        assert_eq!(normalize_requested_mode(&gemini_like, "yolo"), "yolo");
    }

    /// The codex mode lineage folds onto the CURRENT bridge's native ids
    /// (post-022 swap): legacy `default`/`autoEdit` and the old bridge's
    /// `auto` all become `agent`; the old bridge's `full-access` becomes
    /// `agent-full-access`. Other backends must leave these untouched.
    #[test]
    fn normalize_requested_mode_folds_codex_mode_lineage() {
        let mut codex_meta = metadata_with_yolo_id(Some("agent-full-access"));
        codex_meta.backend = Some("codex".into());
        assert_eq!(normalize_requested_mode(&codex_meta, "default"), "agent");
        assert_eq!(normalize_requested_mode(&codex_meta, "autoEdit"), "agent");
        assert_eq!(normalize_requested_mode(&codex_meta, "auto"), "agent");
        assert_eq!(normalize_requested_mode(&codex_meta, "agent"), "agent");
        assert_eq!(
            normalize_requested_mode(&codex_meta, "full-access"),
            "agent-full-access"
        );
        assert_eq!(
            normalize_requested_mode(&codex_meta, "agent-full-access"),
            "agent-full-access"
        );
        assert_eq!(normalize_requested_mode(&codex_meta, "read-only"), "read-only");
        // yolo aliases go through the yolo_id column (migrated value).
        assert_eq!(normalize_requested_mode(&codex_meta, "yolo"), "agent-full-access");

        // A codex row whose yolo_id still carries the pre-022 value (e.g. a
        // DB where the migration's UPDATE filter didn't match because the
        // row was edited) resolves yolo to the old "full-access" — the
        // lineage fold must still land on the new id.
        let mut stale_codex = metadata_with_yolo_id(Some("full-access"));
        stale_codex.backend = Some("codex".into());
        assert_eq!(normalize_requested_mode(&stale_codex, "yolo"), "agent-full-access");

        let other = metadata_with_yolo_id(None);
        assert_eq!(normalize_requested_mode(&other, "default"), "default");
        assert_eq!(normalize_requested_mode(&other, "autoEdit"), "autoEdit");
        assert_eq!(normalize_requested_mode(&other, "full-access"), "full-access");
    }

    #[test]
    fn uses_meta_resume_true_when_policy_flag_set() {
        use nomifun_api_types::BehaviorPolicy;
        let mut meta = metadata_with_yolo_id(None);
        meta.backend = Some("claude".into());
        meta.behavior_policy = BehaviorPolicy {
            session_load_via_meta_field: true,
            ..BehaviorPolicy::default()
        };
        assert!(agent_metadata_uses_meta_resume(&meta));
    }

    #[test]
    fn uses_meta_resume_false_when_policy_flag_unset_even_for_claude_backend() {
        // Hardening test: previously hardcoded `backend == "claude"`. Now
        // the policy is the sole source of truth — a catalog row with
        // backend=claude but no session_load_via_meta_field must return false.
        let mut meta = metadata_with_yolo_id(None);
        meta.backend = Some("claude".into());
        assert!(!agent_metadata_uses_meta_resume(&meta));
    }

    #[test]
    fn uses_meta_resume_false_for_default_metadata() {
        let meta = metadata_with_yolo_id(None);
        assert!(!agent_metadata_uses_meta_resume(&meta));
    }

    #[test]
    fn normalize_requested_mode_trims_and_returns_empty_for_blank() {
        let meta = metadata_with_yolo_id(Some("full-access"));
        assert_eq!(normalize_requested_mode(&meta, "   "), "");
    }
}
