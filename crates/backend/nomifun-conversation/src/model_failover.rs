//! Phase 3 模型故障转移队列(spec §5.5)的纯逻辑层 + 配置读写。
//!
//! 故障转移**只能**在会话服务层(send loop)做:`NomiAgentManager` 不保留重建
//! 输入、engine 也无法原地换 provider,所以"换模型"等于改 `conversation.model` +
//! 杀任务、下次 send 重建。本模块只承担两件无副作用/低副作用的事:
//!
//! 1. [`next_failover_model`] —— 纯函数挑选器(D2),给定失败模型与队列,按序返回
//!    首个可用候选;跳过 provider 关停 / 模型禁用 / 健康检查标 Unhealthy / 失败本身;
//!    队列耗尽返回 `None`(send-loop 见 `None` 即按现状 emit 原始错误,绝不无限切换)。
//! 2. 配置读写 —— 全局存 `client_preferences` 键 `agent.model_failover`(整体 JSON,
//!    形状抄 `nomifun-idmm/service.rs` 的多字段 pref 先例),会话级可在
//!    `conversations.extra.model_failover` 覆盖(存在则优先于全局)。
//!
//! 健康字段 fail-open:精确 Chat capability 的 `health` 是 TEXT JSON,解析失败时按
//! "未知健康"处理 —— 宁可多保留一个候选也不要因脏数据把队列误清空。模型启用
//! 状态只读 `provider_models.enabled`;任务存在性与健康只读 Chat capability。

use std::sync::Arc;

use nomifun_api_types::{AgentErrorCode, CapabilityHealth, HealthStatus, ModelFailoverConfig};
use nomifun_common::{AppError, ErrorChain, ProviderId, ProviderWithModel};
use nomifun_db::IClientPreferenceRepository;
use nomifun_db::models::Provider;
use nomifun_db::{ProviderModelCapabilityRow, ProviderModelRow};
use tracing::warn;

/// `client_preferences` 键,存放全局模型故障转移配置(整体 JSON)。
pub const MODEL_FAILOVER_PREF_KEY: &str = "agent.model_failover";

fn validate_model_reference(model: &ProviderWithModel) -> Result<(), AppError> {
    ProviderId::try_from(model.provider_id.as_str()).map_err(|_| {
        AppError::BadRequest("model failover requires canonical provider_id values".to_owned())
    })?;
    if model.model.is_empty() || model.model.trim() != model.model {
        return Err(AppError::BadRequest(
            "model failover requires trimmed, non-empty model values".to_owned(),
        ));
    }
    if model.use_model.as_deref().is_some_and(|value| {
        value.is_empty() || value.trim() != value
    }) {
        return Err(AppError::BadRequest(
            "model failover overrides must be trimmed and non-empty".to_owned(),
        ));
    }
    Ok(())
}

fn validate_failover_config(config: &ModelFailoverConfig) -> Result<(), AppError> {
    for model in &config.queue {
        validate_model_reference(model)?;
    }
    Ok(())
}

/// 判定一个 `AgentErrorCode` 是否为「provider 故障」——即换个备用模型可能绕过的
/// 单厂商失败(限流 / 5xx / 网络 / 配置)。
///
/// 这是全仓库唯一的权威副本:故障转移 seam 在 `nomifun-conversation`,而
/// `nomifun-idmm` 在其之上,通过 `nomifun_idmm::config::is_provider_fault`
/// re-export 复用本函数。
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

/// 读全局故障转移配置。未设置(无该 pref 行)或 JSON 损坏时回落到
/// [`ModelFailoverConfig::default`](默认关闭),保证调用方永远拿到可用配置。
pub async fn get_global_failover_config(
    client_prefs: &Arc<dyn IClientPreferenceRepository>,
) -> ModelFailoverConfig {
    let rows = match client_prefs.get_by_keys(&[MODEL_FAILOVER_PREF_KEY]).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(error = %ErrorChain(&e), "Failed to read model failover pref; defaulting to disabled");
            return ModelFailoverConfig::default();
        }
    };
    rows.into_iter()
        .find(|r| r.key == MODEL_FAILOVER_PREF_KEY)
        .and_then(|r| match serde_json::from_str::<ModelFailoverConfig>(&r.value) {
            Ok(cfg) if validate_failover_config(&cfg).is_ok() => Some(cfg),
            Ok(_) => {
                warn!("Invalid provider/model reference in model failover pref; defaulting to disabled");
                None
            }
            Err(e) => {
                warn!(error = %ErrorChain(&e), "Malformed model failover pref; defaulting to disabled");
                None
            }
        })
        .unwrap_or_default()
}

/// 写全局故障转移配置(整体 JSON 进单个 pref 键)。形状抄 idmm `set_settings` 的
/// `upsert_batch` 先例。
pub async fn set_global_failover_config(
    client_prefs: &Arc<dyn IClientPreferenceRepository>,
    config: &ModelFailoverConfig,
) -> Result<(), AppError> {
    validate_failover_config(config)?;
    let value =
        serde_json::to_string(config).map_err(|e| AppError::Internal(format!("serialize failover config: {e}")))?;
    client_prefs
        .upsert_batch(&[(MODEL_FAILOVER_PREF_KEY, value.as_str())])
        .await?;
    Ok(())
}

/// 从 `conversations.extra` 的 JSON 文本里读会话级覆盖。`extra.model_failover`
/// 存在(且能解析)则返回它,否则 `None`(交由调用方回落到全局)。脏 `extra` /
/// 缺字段一律按"无覆盖"处理 —— 与会话其余 extra 字段的容错读法一致。
pub fn read_conversation_failover_override(extra_json: &str) -> Option<ModelFailoverConfig> {
    let value: serde_json::Value = serde_json::from_str(extra_json).ok()?;
    let raw = value.get("model_failover")?;
    serde_json::from_value::<ModelFailoverConfig>(raw.clone())
        .ok()
        .filter(|config| validate_failover_config(config).is_ok())
}

/// 一个候选 `(provider, model)` 的可用性判定(行读,fail-open)。
///
/// provider 启用 && 该模型行启用 && 存在精确 Chat capability && 该 capability
/// 健康 JSON 未标 Unhealthy(Unknown / Healthy / 无记录 / 脏 JSON 都放行)。
/// 模型行或 Chat capability 不存在都 fail closed。
fn model_is_candidate(
    provider: &Provider,
    model_rows: &[ProviderModelRow],
    capability_rows: &[ProviderModelCapabilityRow],
    model: &str,
) -> bool {
    if !provider.enabled {
        return false;
    }
    let row = model_rows
        .iter()
        .find(|row| row.provider_id == provider.provider_id && row.model == model);
    let Some(row) = row else {
        return false;
    };
    if !row.enabled {
        return false;
    }
    // 只取健康 JSON 的 `status` 字段;解析失败按未知健康放行(fail-open)。
    // Only an explicitly Chat-scoped observation can exclude a Chat failover
    // candidate. Another modality may fail while the same model's Chat
    // endpoint remains healthy.
    let Some(chat) = capability_rows.iter().find(|capability| {
        capability.provider_id == provider.provider_id
            && capability.model == model
            && capability.task == "chat"
    }) else {
        return false;
    };
    let blocks_chat = chat
        .health
        .as_deref()
        .and_then(|value| serde_json::from_str::<CapabilityHealth>(value).ok())
        .is_some_and(|health| health.status == HealthStatus::Unhealthy);
    !blocks_chat
}

/// D2 挑选器(纯函数):按队列序返回首个可用候选模型。
///
/// 跳过已禁用 provider、已禁用/不存在的模型、缺少 Chat capability、
/// Chat capability 健康状态为 `Unhealthy`、与刚失败的 `(provider_id, model)`
/// 完全相同的条目、以及**本轮已经试过**的任何 `(provider_id, model)`(`tried`,
/// review #2 单调性:多次切换时不回头重试已切过的候选,杜绝队列里循环抖动)。
/// 无可用候选时返回 `None`(队列耗尽 → send-loop 不再转移)。
///
/// `providers` 是当前全部 provider 行;队列里引用的 provider 若不在表中,该候选
/// 被视为不可用(找不到 = 不能用)。
pub fn next_failover_model(
    queue: &[ProviderWithModel],
    failed: &ProviderWithModel,
    tried: &[ProviderWithModel],
    providers: &[Provider],
    model_rows: &[ProviderModelRow],
    capability_rows: &[ProviderModelCapabilityRow],
) -> Option<ProviderWithModel> {
    let same = |a: &ProviderWithModel, b: &ProviderWithModel| a.provider_id == b.provider_id && a.model == b.model;
    queue.iter().find_map(|candidate| {
        if validate_model_reference(candidate).is_err() {
            return None;
        }
        // 跳过刚失败的同一 (provider_id, model)。
        if same(candidate, failed) {
            return None;
        }
        // review #2:跳过本轮已经切到过的候选(单调推进,不重试)。
        if tried.iter().any(|t| same(candidate, t)) {
            return None;
        }
        let provider = providers.iter().find(|p| {
            ProviderId::try_from(p.provider_id.as_str()).is_ok()
                && p.provider_id == candidate.provider_id
        })?;
        if model_is_candidate(provider, model_rows, capability_rows, &candidate.model) {
            Some(candidate.clone())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const P1: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const P2: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const P3: &str = "0190f5fe-7c00-7a00-8000-000000000003";
    const GHOST: &str = "0190f5fe-7c00-7a00-8000-000000000099";

    fn pwm(provider_id: &str, model: &str) -> ProviderWithModel {
        ProviderWithModel {
            provider_id: provider_id.into(),
            model: model.into(),
            use_model: None,
        }
    }

    /// 构造一行 provider(模型启用状态与 Chat 健康分别由两个 fixture 提供)。
    fn provider(id: &str, enabled: bool) -> Provider {
        Provider {
            id: 0,
            provider_id: id.into(),
            platform: "openai".into(),
            name: id.into(),
            base_url: "https://example.com".into(),
            auth_scheme: "bearer".into(),
            credentials_encrypted: nomifun_common::encrypt_string(
                r#"{"api_keys":["test-only"]}"#,
                &[0x42; 32],
            )
            .unwrap(),
            enabled,
            bedrock_config: None,
            sort_order: 0,
            config_revision: 1,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn row(provider_id: &str, model: &str, enabled: bool) -> ProviderModelRow {
        ProviderModelRow {
            id: 0,
            provider_id: provider_id.into(),
            model: model.into(),
            enabled,
            sort_order: 0,
            description: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn capability_for_task(
        provider_id: &str,
        model: &str,
        task: &str,
        health: Option<HealthStatus>,
    ) -> ProviderModelCapabilityRow {
        let health = health.map(|status| {
            serde_json::to_string(&CapabilityHealth {
                status,
                latency: None,
                error: None,
                error_kind: None,
                http_status: None,
                attempted_url: None,
            })
            .unwrap()
        });
        ProviderModelCapabilityRow {
            id: 0,
            provider_id: provider_id.into(),
            model: model.into(),
            task: task.into(),
            traits: "[]".into(),
            protocol: "openai.chat_text".into(),
            connection_role: "default".into(),
            base_url_override: None,
            endpoint: None,
            poll_endpoint: None,
            content_endpoint: None,
            realtime_endpoint: None,
            allow_cross_origin_credentials: false,
            provider_params: "{}".into(),
            context_limit: None,
            health,
            health_checked_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn capability(
        provider_id: &str,
        model: &str,
        health: Option<HealthStatus>,
    ) -> ProviderModelCapabilityRow {
        capability_for_task(provider_id, model, "chat", health)
    }

    #[test]
    fn picks_next_available_skipping_failed() {
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2")];
        let failed = pwm(P1, "m1");
        let providers = vec![provider(P1, true), provider(P2, true)];
        let rows = vec![row(P2, "m2", true)];
        let capabilities = vec![capability(P2, "m2", None)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities)
            .expect("should pick p2/m2");
        assert_eq!(pick.provider_id, P2);
        assert_eq!(pick.model, "m2");
    }

    #[test]
    fn skips_already_tried_candidates() {
        // review #2 (monotonicity): a candidate already switched to this turn is
        // skipped even though it is still healthy/enabled — so multiple failover
        // hops advance through the queue instead of bouncing back to p2/m2.
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2"), pwm(P3, "m3")];
        let failed = pwm(P1, "m1");
        let tried = vec![pwm(P2, "m2")];
        let providers = vec![
            provider(P1, true),
            provider(P2, true),
            provider(P3, true),
        ];
        let rows = vec![row(P3, "m3", true)];
        let capabilities = vec![capability(P3, "m3", None)];
        let pick = next_failover_model(&queue, &failed, &tried, &providers, &rows, &capabilities)
            .expect("should skip tried p2/m2");
        assert_eq!(pick.provider_id, P3);
        assert_eq!(pick.model, "m3");
    }

    #[test]
    fn exhausts_when_only_remaining_candidate_already_tried() {
        // Queue has p1 (failed) and p2 (already tried) → nothing left → None.
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2")];
        let failed = pwm(P1, "m1");
        let tried = vec![pwm(P2, "m2")];
        let providers = vec![provider(P1, true), provider(P2, true)];
        assert!(next_failover_model(&queue, &failed, &tried, &providers, &[], &[]).is_none());
    }

    #[test]
    fn skips_disabled_provider() {
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2")];
        let failed = pwm(GHOST, "orig");
        // p1 是禁用 provider → 跳过,落到 p2。
        let providers = vec![provider(P1, false), provider(P2, true)];
        let rows = vec![row(P2, "m2", true)];
        let capabilities = vec![capability(P2, "m2", None)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities)
            .expect("should skip disabled p1");
        assert_eq!(pick.provider_id, P2);
    }

    #[test]
    fn skips_model_disabled() {
        let queue = vec![pwm(P1, "m1"), pwm(P1, "m2")];
        let failed = pwm(GHOST, "orig");
        // p1 的 m1 行被禁用 → 跳过,落到 m2。
        let providers = vec![provider(P1, true)];
        let rows = vec![row(P1, "m1", false), row(P1, "m2", true)];
        let capabilities = vec![capability(P1, "m1", None), capability(P1, "m2", None)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities)
            .expect("should skip disabled m1");
        assert_eq!(pick.model, "m2");
    }

    #[test]
    fn skips_unhealthy_model() {
        let queue = vec![pwm(P1, "m1"), pwm(P1, "m2")];
        let failed = pwm(GHOST, "orig");
        // m1 Chat capability 标 Unhealthy → 跳过;m2 Healthy → 选中。
        let providers = vec![provider(P1, true)];
        let rows = vec![
            row(P1, "m1", true),
            row(P1, "m2", true),
        ];
        let capabilities = vec![
            capability(P1, "m1", Some(HealthStatus::Unhealthy)),
            capability(P1, "m2", Some(HealthStatus::Healthy)),
        ];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities)
            .expect("should skip unhealthy m1");
        assert_eq!(pick.model, "m2");
    }

    #[test]
    fn missing_chat_capability_is_not_a_candidate() {
        let queue = vec![pwm(P1, "m1")];
        let failed = pwm(GHOST, "orig");
        let providers = vec![provider(P1, true)];
        let rows = vec![row(P1, "m1", true)];

        assert!(next_failover_model(&queue, &failed, &[], &providers, &rows, &[]).is_none());
    }

    #[test]
    fn non_chat_capability_does_not_qualify_for_chat_failover() {
        let queue = vec![pwm(P1, "m1")];
        let failed = pwm(GHOST, "orig");
        let providers = vec![provider(P1, true)];
        let rows = vec![row(P1, "m1", true)];
        let capabilities = vec![capability_for_task(
            P1,
            "m1",
            "speech_synthesis",
            None,
        )];

        assert!(
            next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities).is_none()
        );
    }

    #[test]
    fn unknown_health_is_still_a_candidate() {
        // Unknown / 无健康记录 不应被跳过(只有 Unhealthy 才排除)。
        let queue = vec![pwm(P1, "m1")];
        let failed = pwm(GHOST, "orig");
        let providers = vec![provider(P1, true)];
        let rows = vec![row(P1, "m1", true)];
        let capabilities = vec![capability(P1, "m1", Some(HealthStatus::Unknown))];
        assert!(
            next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities).is_some()
        );
    }

    #[test]
    fn returns_none_when_exhausted() {
        // 队列里唯一候选就是刚失败的那个 → 耗尽 → None。
        let queue = vec![pwm(P1, "m1")];
        let failed = pwm(P1, "m1");
        let providers = vec![provider(P1, true)];
        assert!(next_failover_model(&queue, &failed, &[], &providers, &[], &[]).is_none());
    }

    #[test]
    fn returns_none_when_all_unavailable() {
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2")];
        let failed = pwm(GHOST, "orig");
        // p1 禁用 + p2 的 m2 Unhealthy → 全不可用 → None。
        let providers = vec![provider(P1, false), provider(P2, true)];
        let rows = vec![row(P2, "m2", true)];
        let capabilities = vec![capability(P2, "m2", Some(HealthStatus::Unhealthy))];
        assert!(
            next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities).is_none()
        );
    }

    #[test]
    fn missing_provider_row_is_not_a_candidate() {
        // 候选引用的 provider 不在表中 → 找不到即不可用,跳到下一个。
        let queue = vec![pwm(GHOST, "m1"), pwm(P2, "m2")];
        let failed = pwm(GHOST, "orig");
        let providers = vec![provider(P2, true)];
        let rows = vec![row(P2, "m2", true)];
        let capabilities = vec![capability(P2, "m2", None)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &rows, &capabilities)
            .expect("should fall to p2");
        assert_eq!(pick.provider_id, P2);
    }

    #[test]
    fn empty_queue_returns_none() {
        let providers = vec![provider(P1, true)];
        assert!(next_failover_model(&[], &pwm(P1, "m1"), &[], &providers, &[], &[]).is_none());
    }

    #[test]
    fn malformed_chat_capability_health_json_fails_open() {
        // Chat capability health 是垃圾字符串 → 按未知健康处理,候选仍可用。
        let row = row(P1, "m1", true);
        let mut broken = capability(P1, "m1", None);
        broken.health = Some("{not json".into());
        let providers = vec![provider(P1, true)];
        let queue = vec![pwm(P1, "m1")];
        assert!(
            next_failover_model(
                &queue,
                &pwm(GHOST, "orig"),
                &[],
                &providers,
                &[row],
                &[broken],
            )
                .is_some()
        );
    }

    // ── 配置读写 ──

    #[test]
    fn conversation_override_present_parses() {
        let extra = serde_json::json!({
            "workspace": "/tmp/x",
            "model_failover": {"enabled": true, "max_switches": 2}
        })
        .to_string();
        let cfg = read_conversation_failover_override(&extra).expect("override present");
        assert!(cfg.enabled);
        assert_eq!(cfg.max_switches, 2);
        assert!(cfg.queue.is_empty());
    }

    #[test]
    fn conversation_override_absent_is_none() {
        let extra = serde_json::json!({"workspace": "/tmp/x"}).to_string();
        assert!(read_conversation_failover_override(&extra).is_none());
    }

    #[test]
    fn conversation_override_malformed_extra_is_none() {
        assert!(read_conversation_failover_override("{not json").is_none());
    }

    // ── 故障分类(全仓库唯一权威表;idmm 经 re-export 复用)──

    #[test]
    fn is_provider_fault_matches_known_codes() {
        assert!(is_provider_fault(AgentErrorCode::UserLlmProviderRateLimited));
        assert!(is_provider_fault(AgentErrorCode::UserLlmProviderGatewayError));
        assert!(is_provider_fault(AgentErrorCode::UserLlmProviderTimeout));
        assert!(is_provider_fault(AgentErrorCode::UnknownUpstreamError));
        // 非 provider 故障:用户取消 / 会话忙 等不应触发转移。
        assert!(!is_provider_fault(AgentErrorCode::UserAgentNotInstalled));
        assert!(!is_provider_fault(AgentErrorCode::NomifunConversationBusy));
    }

    #[test]
    fn image_unsupported_is_not_provider_fault() {
        assert!(!is_provider_fault(AgentErrorCode::UserLlmProviderImageUnsupported));
    }
}
