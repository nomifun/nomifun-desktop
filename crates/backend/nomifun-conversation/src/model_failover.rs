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
//! 健康字段 fail-open:`provider_models.health` 是 TEXT JSON,解析失败时按
//! "未知健康"处理 —— 宁可多保留一个候选也不要因脏数据把队列误清空。迁移 016 之后
//! 每模型 enabled / health 只存在于 `provider_models` 行上,挑选器据此读行。

use std::sync::Arc;

use nomifun_api_types::{AgentErrorCode, HealthStatus, ModelFailoverConfig};
use nomifun_common::{AppError, ErrorChain, ProviderId, ProviderWithModel};
use nomifun_db::IClientPreferenceRepository;
use nomifun_db::models::Provider;
use nomifun_db::ProviderModelRow;
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
/// provider 启用 && 该模型的 `provider_models` 行未被禁用 && 行上健康 JSON 未标
/// Unhealthy(Unknown / Healthy / 无记录 / 脏 JSON 都放行)。模型没有行(已从
/// 目录移除)按"未禁用/未知健康"处理,与旧 map 缺项语义一致 —— provider 存在性
/// 本身仍是硬闸。
fn model_is_candidate(provider: &Provider, model_rows: &[ProviderModelRow], model: &str) -> bool {
    if !provider.enabled {
        return false;
    }
    let row = model_rows
        .iter()
        .find(|row| row.provider_id == provider.provider_id && row.model == model);
    let Some(row) = row else {
        // 无行 = 目录里已没有这个模型;沿用旧语义(map 缺项不禁用)让 provider
        // 存在性与调用侧校验兜底,不在这里误清空队列。
        return true;
    };
    if !row.enabled {
        return false;
    }
    // 只取健康 JSON 的 `status` 字段;解析失败按未知健康放行(fail-open)。
    let health = row
        .health
        .as_deref()
        .and_then(|s| serde_json::from_str::<nomifun_api_types::ModelHealthStatus>(s).ok())
        .map(|h| h.status);
    health != Some(HealthStatus::Unhealthy)
}

/// D2 挑选器(纯函数):按队列序返回首个可用候选模型。
///
/// 跳过:`provider.enabled == false`、`model_enabled[model] == Some(false)`、
/// `model_health[model].status == Unhealthy`、与刚失败的 `(provider_id, model)`
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
        if model_is_candidate(provider, model_rows, &candidate.model) {
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

    /// 构造一行 provider(每模型状态在 `row(...)` 构造的 provider_models 行上)。
    fn provider(id: &str, enabled: bool) -> Provider {
        Provider {
            id: 0,
            provider_id: id.into(),
            platform: "openai".into(),
            name: id.into(),
            base_url: "https://example.com".into(),
            api_key_encrypted: "x".into(),
            enabled,
            bedrock_config: None,
            is_full_url: false,
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// 构造一条 provider_models 行:enabled 标志 + 可选健康 JSON。
    fn row(provider_id: &str, model: &str, enabled: bool, health: Option<HealthStatus>) -> ProviderModelRow {
        let health_json = health.map(|status| {
            serde_json::to_string(&nomifun_api_types::ModelHealthStatus {
                status,
                last_check: None,
                latency: None,
                error: None,
            })
            .unwrap()
        });
        ProviderModelRow {
            id: 0,
            provider_id: provider_id.into(),
            model: model.into(),
            enabled,
            sort_order: 0,
            tasks: "[]".into(),
            traits: "[]".into(),
            protocol: None,
            connection_role: None,
            params: "{}".into(),
            context_limit: None,
            description: None,
            source: "inferred".into(),
            health: health_json,
            health_checked_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn picks_next_available_skipping_failed() {
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2")];
        let failed = pwm(P1, "m1");
        let providers = vec![provider(P1, true), provider(P2, true)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &[])
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
        let pick = next_failover_model(&queue, &failed, &tried, &providers, &[])
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
        assert!(next_failover_model(&queue, &failed, &tried, &providers, &[]).is_none());
    }

    #[test]
    fn skips_disabled_provider() {
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2")];
        let failed = pwm(GHOST, "orig");
        // p1 是禁用 provider → 跳过,落到 p2。
        let providers = vec![provider(P1, false), provider(P2, true)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &[])
            .expect("should skip disabled p1");
        assert_eq!(pick.provider_id, P2);
    }

    #[test]
    fn skips_model_disabled() {
        let queue = vec![pwm(P1, "m1"), pwm(P1, "m2")];
        let failed = pwm(GHOST, "orig");
        // p1 的 m1 行被禁用 → 跳过,落到 m2。
        let providers = vec![provider(P1, true)];
        let rows = vec![row(P1, "m1", false, None), row(P1, "m2", true, None)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &rows)
            .expect("should skip disabled m1");
        assert_eq!(pick.model, "m2");
    }

    #[test]
    fn skips_unhealthy_model() {
        let queue = vec![pwm(P1, "m1"), pwm(P1, "m2")];
        let failed = pwm(GHOST, "orig");
        // m1 行标 Unhealthy → 跳过;m2 Healthy → 选中。
        let providers = vec![provider(P1, true)];
        let rows = vec![
            row(P1, "m1", true, Some(HealthStatus::Unhealthy)),
            row(P1, "m2", true, Some(HealthStatus::Healthy)),
        ];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &rows)
            .expect("should skip unhealthy m1");
        assert_eq!(pick.model, "m2");
    }

    #[test]
    fn unknown_health_is_still_a_candidate() {
        // Unknown / 无健康记录 不应被跳过(只有 Unhealthy 才排除)。
        let queue = vec![pwm(P1, "m1")];
        let failed = pwm(GHOST, "orig");
        let providers = vec![provider(P1, true)];
        let rows = vec![row(P1, "m1", true, Some(HealthStatus::Unknown))];
        assert!(next_failover_model(&queue, &failed, &[], &providers, &rows).is_some());
    }

    #[test]
    fn returns_none_when_exhausted() {
        // 队列里唯一候选就是刚失败的那个 → 耗尽 → None。
        let queue = vec![pwm(P1, "m1")];
        let failed = pwm(P1, "m1");
        let providers = vec![provider(P1, true)];
        assert!(next_failover_model(&queue, &failed, &[], &providers, &[]).is_none());
    }

    #[test]
    fn returns_none_when_all_unavailable() {
        let queue = vec![pwm(P1, "m1"), pwm(P2, "m2")];
        let failed = pwm(GHOST, "orig");
        // p1 禁用 + p2 的 m2 Unhealthy → 全不可用 → None。
        let providers = vec![provider(P1, false), provider(P2, true)];
        let rows = vec![row(P2, "m2", true, Some(HealthStatus::Unhealthy))];
        assert!(next_failover_model(&queue, &failed, &[], &providers, &rows).is_none());
    }

    #[test]
    fn missing_provider_row_is_not_a_candidate() {
        // 候选引用的 provider 不在表中 → 找不到即不可用,跳到下一个。
        let queue = vec![pwm(GHOST, "m1"), pwm(P2, "m2")];
        let failed = pwm(GHOST, "orig");
        let providers = vec![provider(P2, true)];
        let pick = next_failover_model(&queue, &failed, &[], &providers, &[])
            .expect("should fall to p2");
        assert_eq!(pick.provider_id, P2);
    }

    #[test]
    fn empty_queue_returns_none() {
        let providers = vec![provider(P1, true)];
        assert!(next_failover_model(&[], &pwm(P1, "m1"), &[], &providers, &[]).is_none());
    }

    #[test]
    fn malformed_model_health_json_fails_open() {
        // 行上的 health 是垃圾字符串 → 按未知健康处理,候选仍可用。
        let mut broken = row(P1, "m1", true, None);
        broken.health = Some("{not json".into());
        let providers = vec![provider(P1, true)];
        let queue = vec![pwm(P1, "m1")];
        assert!(
            next_failover_model(&queue, &pwm(GHOST, "orig"), &[], &providers, &[broken])
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
        // 未给的字段仍走默认。
        assert!(cfg.stamp_unhealthy);
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
