//! Phase 3 模型故障转移 seam(plan D3/D5/D6)的会话服务侧实现。
//!
//! 纯逻辑(挑选器 / 配置读写 / 故障分类)在 [`crate::model_failover`];本模块只放
//! 需要 `&ConversationService`(仓库 + runtime_registry)的有副作用步骤。生产切换只由
//! 持有精确 turn generation 的 send-loop 发起;IDMM 仅验证观察结果,不会越权换模型或
//! 重建 runtime。
//!
//! 终态错误后仅终止 runtime 的旧恢复路径已随多引擎收敛一并移除;本模块是它的泛化:
//! 换模型后重建 runtime 并把新句柄交回 send-loop。

use std::sync::Arc;
use std::time::Duration;

use nomifun_api_types::{ExecutionModelPool, ExecutionModelRef};
use nomifun_common::{
    AgentKillReason, AgentType, AppError, ConversationStatus, ErrorChain, ProviderWithModel, now_ms,
};
use nomifun_db::ConversationRowUpdate;
use nomifun_ai_agent::{AgentRuntimeHandle, AgentRuntimeRegistry};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::convert::string_to_enum;
use crate::model_failover::{
    get_global_failover_config, next_failover_model, read_conversation_failover_override,
};
use crate::service::{ConversationService, parse_conv_id};
use crate::stream_relay::RelayOutcome;
use crate::runtime_options::provider_model_from_conversation_row;

/// 一次成功的故障转移结果:重建后的新任务句柄 + 被选中的候选模型。
pub struct FailoverSwitch {
    /// 换模型并重建后的 agent 句柄。send-loop 用它 `subscribe()` + 重发同一内容。
    pub agent: AgentRuntimeHandle,
    /// 本次切换到的 `(provider_id, model)`(已写入 `conversation.model`)。
    pub picked: ProviderWithModel,
    /// 新 runtime 构建时对应的完整 durable failover authority。send-loop
    /// 必须把它带入下一次切换，避免把上一轮快照误判为并发配置修改。
    pub(crate) authority: FailoverAuthoritySnapshot,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FailoverAuthoritySnapshot {
    pub(crate) model: ProviderWithModel,
    pub(crate) execution_model_pool: Option<String>,
    pub(crate) execution_template_id: Option<String>,
}

fn selected_model_ref(model: &ProviderWithModel) -> ExecutionModelRef {
    ExecutionModelRef {
        provider_id: model.provider_id.clone(),
        model: model
            .use_model
            .clone()
            .unwrap_or_else(|| model.model.clone()),
    }
}

fn rewrite_execution_model_pool_for_failover(
    encoded: Option<&str>,
    failed: &ProviderWithModel,
    picked: &ProviderWithModel,
) -> Result<Option<String>, String> {
    let Some(encoded) = encoded else {
        return Ok(None);
    };
    let current: ExecutionModelPool = serde_json::from_str(encoded)
        .map_err(|error| format!("invalid persisted execution model pool: {error}"))?;
    current.validate()?;
    let failed = selected_model_ref(failed);
    let picked = selected_model_ref(picked);
    let rewritten = match current {
        ExecutionModelPool::Automatic => ExecutionModelPool::Automatic,
        ExecutionModelPool::Single { .. } => ExecutionModelPool::Single { model: picked },
        ExecutionModelPool::Range { models } => {
            let mut retained = vec![picked.clone()];
            retained.extend(
                models
                    .into_iter()
                    .filter(|model| model != &failed && model != &picked),
            );
            ExecutionModelPool::Range { models: retained }
        }
    };
    rewritten.validate()?;
    serde_json::to_string(&rewritten)
        .map(Some)
        .map_err(|error| format!("encode execution model pool: {error}"))
}

impl ConversationService {
    /// 解析该会话**生效**的故障转移配置:会话级 `extra.model_failover` 覆盖存在
    /// 则优先,否则回落到全局 `client_preferences` 的 `agent.model_failover`。
    /// 未注册 client-prefs 依赖(`with_failover_deps` 没调过)时返回 `None` —— 视为
    /// 故障转移关闭(fail-safe)。
    pub(crate) async fn resolve_failover_config(
        &self,
        extra_json: &str,
    ) -> Option<nomifun_api_types::ModelFailoverConfig> {
        if let Some(override_cfg) = read_conversation_failover_override(extra_json) {
            return Some(override_cfg);
        }
        let (_, _, _, client_prefs) = self.failover_deps()?;
        Some(get_global_failover_config(&client_prefs).await)
    }

    async fn perform_model_failover_inner(
        &self,
        conversation_id: &str,
        config: &nomifun_api_types::ModelFailoverConfig,
        tried: &[ProviderWithModel],
        expected_authority: Option<&FailoverAuthoritySnapshot>,
        runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
        runtime_generation: u64,
        cancellation: &CancellationToken,
    ) -> Option<FailoverSwitch> {
        if cancellation.is_cancelled() {
            return None;
        }
        let runtime_state = self.runtime_state();
        let _configuration_guard = match runtime_state
            .acquire_preparation_gate(conversation_id, cancellation)
            .await
        {
            Ok(guard) => guard,
            Err(error) => {
                warn!(error = %ErrorChain(&error), conversation_id, "Failover skipped: configuration gate unavailable");
                return None;
            }
        };
        if cancellation.is_cancelled() {
            return None;
        }
        let Some((provider_repo, provider_model_repo, capability_repo, _)) = self.failover_deps()
        else {
            return None;
        };
        let conv_id = conversation_id;
        let row = match self.conversation_repo().get(conv_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                warn!(conversation_id, "Failover skipped: conversation row missing");
                return None;
            }
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover skipped: failed to load conversation");
                return None;
            }
        };
        if cancellation.is_cancelled() {
            return None;
        }

        // ACP 边界(review #9,plan D7)的**唯一强制闸**:仅 nomi 自有引擎的普通会话
        // 可换模型重建。ACP / 终端 / 远程等 agent 自管模型(独立 reconcile),在此被
        // fail-safe 拒绝——不终止 runtime、不写 model。IDMM 不进入这条有副作用路径。
        let agent_type: AgentType = match string_to_enum(&row.r#type) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, agent_type = %row.r#type, "Failover skipped: unparseable agent type");
                return None;
            }
        };
        if agent_type != AgentType::Nomi {
            warn!(
                conversation_id,
                agent_type = ?agent_type,
                "Failover skipped: not a nomi conversation (ACP/terminal self-manage their model)"
            );
            return None;
        }

        let failed = match provider_model_from_conversation_row(&row) {
            Ok(Some(model)) => model,
            Ok(None) => {
                warn!(conversation_id, "Failover skipped: conversation has no configured model");
                return None;
            }
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover skipped: invalid persisted conversation model");
                return None;
            }
        };
        if let Some(expected) = expected_authority
            && (failed != expected.model
                || row.execution_model_pool != expected.execution_model_pool
                || row.execution_template_id != expected.execution_template_id)
        {
            warn!(
                conversation_id,
                expected_provider = %expected.model.provider_id,
                expected_model = %expected.model.model,
                durable_provider = %failed.provider_id,
                durable_model = %failed.model,
                pool_changed = row.execution_model_pool != expected.execution_model_pool,
                template_changed = row.execution_template_id != expected.execution_template_id,
                "Failover skipped: explicit failover authority changed after the failing runtime was built"
            );
            return None;
        }
        let providers = match provider_repo.list().await {
            Ok(providers) => providers,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover skipped: failed to list providers");
                return None;
            }
        };
        // 模型 enabled 来自 provider_models;可用性和健康来自精确 Chat capability。
        let model_rows = match provider_model_repo.list().await {
            Ok(rows) => rows,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover skipped: failed to list provider models");
                return None;
            }
        };
        let capability_rows = match capability_repo.list().await {
            Ok(rows) => rows,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover skipped: failed to list provider model capabilities");
                return None;
            }
        };
        if cancellation.is_cancelled() {
            return None;
        }

        // 队列耗尽 / 无可用候选 → None(调用方回落到原始错误)。
        let picked = next_failover_model(
            &config.queue,
            &failed,
            tried,
            &providers,
            &model_rows,
            &capability_rows,
        )?;

        // 先构造新模型的持久化内容，但必须等旧 runtime 精确退出后才写入。
        // 否则 teardown 失败或取消会留下“DB 是新模型、进程仍是旧模型”的半提交。
        let model_json = match serde_json::to_string(&picked) {
            Ok(json) => json,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover aborted: serialize picked model failed");
                return None;
            }
        };
        let execution_model_pool = match rewrite_execution_model_pool_for_failover(
            row.execution_model_pool.as_deref(),
            &failed,
            &picked,
        ) {
            Ok(pool) => pool,
            Err(error) => {
                warn!(%error, conversation_id, "Failover aborted: invalid execution model authority");
                return None;
            }
        };
        info!(
            conversation_id,
            failed_provider = %failed.provider_id,
            failed_model = %failed.model,
            next_provider = %picked.provider_id,
            next_model = %picked.model,
            reason = ?AgentKillReason::ConfigurationChanged,
            "Model failover: awaiting old runtime teardown before committing model switch"
        );

        // kill_and_wait:旧任务句柄绑定旧 provider/model,必须等它落幕再用新行重建。
        // Cancellation cannot be allowed to drop an in-flight teardown: the
        // registry quarantines the old slot, and this owner must keep retrying
        // until process-tree exit is proven before either rebuilding or letting
        // the durable Running turn finalize.
        let mut retry_delay = Duration::from_millis(25);
        let warning_deadline = Instant::now() + Duration::from_secs(2);
        let mut warning_attempted = false;
        let mut teardown_warning = None;
        loop {
            if !warning_attempted && Instant::now() >= warning_deadline {
                warning_attempted = true;
                teardown_warning = self
                    .persist_and_broadcast_model_failover_teardown_tip(
                        &row.user_id,
                        conversation_id,
                        None,
                    )
                    .await;
            }
            let teardown = Self::terminate_runtime_with_proof(
                runtime_registry,
                conversation_id,
                AgentKillReason::ConfigurationChanged,
                "model failover",
            );
            tokio::pin!(teardown);
            let teardown_result = if warning_attempted {
                teardown.await
            } else {
                tokio::select! {
                    biased;
                    result = &mut teardown => result,
                    _ = tokio::time::sleep_until(warning_deadline) => {
                        warning_attempted = true;
                        teardown_warning = self
                            .persist_and_broadcast_model_failover_teardown_tip(
                                &row.user_id,
                                conversation_id,
                                None,
                            )
                            .await;
                        teardown.await
                    }
                }
            };
            match teardown_result {
                Ok(()) => break,
                Err(_) => {}
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(Duration::from_secs(2));
        }
        if let Some(mut warning) = teardown_warning {
            let _ = self
                .resolve_and_broadcast_model_failover_teardown_tip(
                    &row.user_id,
                    &mut warning,
                    None,
                )
                .await;
        }
        if cancellation.is_cancelled() {
            return None;
        }

        // Revalidate the durable authority after the potentially long teardown.
        // Public runtime-affecting PATCHes take the same preparation gate, and
        // this snapshot comparison also fails closed for any trusted writer
        // that bypassed that gate. Never let an old failover overwrite a newer
        // explicit model/pool/template choice.
        let current = match self.conversation_repo().get(conv_id).await {
            Ok(Some(current)) => current,
            Ok(None) => {
                warn!(conversation_id, "Failover aborted: conversation vanished after runtime teardown");
                return None;
            }
            Err(error) => {
                warn!(error = %ErrorChain(&error), conversation_id, "Failover aborted: failed to revalidate configuration after runtime teardown");
                return None;
            }
        };
        if current.model != row.model
            || current.execution_model_pool != row.execution_model_pool
            || current.execution_template_id != row.execution_template_id
        {
            warn!(
                conversation_id,
                "Failover aborted: durable model authority changed while old runtime was tearing down"
            );
            return None;
        }

        let next_authority = FailoverAuthoritySnapshot {
            model: picked.clone(),
            execution_model_pool: execution_model_pool.clone(),
            execution_template_id: None,
        };
        let update = ConversationRowUpdate {
            model: Some(Some(model_json)),
            execution_model_pool: Some(execution_model_pool),
            execution_template_id: Some(None),
            updated_at: Some(now_ms()),
            ..Default::default()
        };
        if let Err(e) = self.conversation_repo().update(conv_id, &update).await {
            warn!(error = %ErrorChain(&e), conversation_id, "Failover aborted: failed to persist new model after runtime teardown");
            return None;
        }
        if cancellation.is_cancelled() {
            return None;
        }

        // 用**刷新后**的行重建。re-fetch 以拿到刚写入的新 model 列。
        let refreshed = match self.conversation_repo().get(conv_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                warn!(conversation_id, "Failover aborted: conversation vanished after model write");
                return None;
            }
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover aborted: re-fetch after model write failed");
                return None;
            }
        };
        if cancellation.is_cancelled() {
            return None;
        }
        let (runtime_options, knowledge_signature) = match self
            .prepare_runtime_options_for_execution(
                &refreshed,
                runtime_registry,
                Some(cancellation),
            )
            .await
        {
            Ok(prepared) => prepared,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover aborted: strict runtime preparation on refreshed row failed");
                return None;
            }
        };
        let agent = match runtime_registry
            .get_or_create_runtime_for_turn(
                conversation_id,
                runtime_generation,
                cancellation.clone(),
                runtime_options,
            )
            .await
        {
            Ok(agent) => agent,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "Failover aborted: rebuild task failed");
                return None;
            }
        };
        if cancellation.is_cancelled() {
            if let Err(error) = runtime_registry.cancel_runtime_turn(
                conversation_id,
                runtime_generation,
                Some(AgentKillReason::UserCancelled),
            ) {
                warn!(
                    conversation_id,
                    error = %ErrorChain(&error),
                    "Cancelled failover rebuild could not initiate exact teardown; retrying through registry barrier"
                );
            }
            Self::terminate_runtime_until_confirmed(
                runtime_registry,
                conversation_id,
                AgentKillReason::UserCancelled,
                "cancelled failover rebuild",
            )
            .await;
            return None;
        }

        self.commit_runtime_knowledge_signature(conversation_id, knowledge_signature);
        Some(FailoverSwitch {
            agent,
            picked,
            authority: next_authority,
        })
    }

    /// 同模型"剔图重建":标记 registry(该 provider+model 不支持图片)→终止 runtime→
    /// 用同一行重建任务。重建时工厂重新读 registry → compat.supports_image=false →
    /// build_messages 剔图。仅 nomi 会话放行;返回新句柄或 None(不可重建)。
    pub(crate) async fn strip_images_and_rebuild(
        &self,
        conversation_id: &str,
        runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
        turn_generation: u64,
        cancellation: &CancellationToken,
    ) -> Option<AgentRuntimeHandle> {
        if cancellation.is_cancelled() {
            return None;
        }
        let conv_id = conversation_id;
        let row = match self.conversation_repo().get(conv_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                warn!(conversation_id, "strip_images_and_rebuild skipped: conversation row missing");
                return None;
            }
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "strip_images_and_rebuild skipped: load failed");
                return None;
            }
        };
        if cancellation.is_cancelled() {
            return None;
        }
        let agent_type: AgentType = string_to_enum(&row.r#type).ok()?;
        if agent_type != AgentType::Nomi {
            return None;
        }
        let pm = match provider_model_from_conversation_row(&row) {
            Ok(Some(model)) => model,
            Ok(None) => {
                warn!(conversation_id, "strip_images_and_rebuild skipped: conversation has no configured model");
                return None;
            }
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "strip_images_and_rebuild skipped: invalid persisted model");
                return None;
            }
        };
        nomifun_common::VisionUnsupportedRegistry::global().mark_unsupported(&pm.provider_id, &pm.model);

        if cancellation.is_cancelled() {
            return None;
        }

        Self::terminate_runtime_until_confirmed(
            runtime_registry,
            conversation_id,
            AgentKillReason::ConfigurationChanged,
            "image fallback rebuild",
        )
        .await;
        if cancellation.is_cancelled() {
            return None;
        }

        let (runtime_options, knowledge_signature) = match self
            .prepare_runtime_options_for_execution(&row, runtime_registry, Some(cancellation))
            .await
        {
            Ok(prepared) => prepared,
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "strip_images_and_rebuild aborted: strict runtime preparation failed");
                return None;
            }
        };
        match runtime_registry
            .get_or_create_runtime_for_turn(
                conversation_id,
                turn_generation,
                cancellation.clone(),
                runtime_options,
            )
            .await
        {
            Ok(agent) if !cancellation.is_cancelled() => {
                self.commit_runtime_knowledge_signature(conversation_id, knowledge_signature);
                Some(agent)
            }
            Ok(agent) => {
                let _ = agent;
                if let Err(error) = runtime_registry.cancel_runtime_turn(
                    conversation_id,
                    turn_generation,
                    Some(AgentKillReason::UserCancelled),
                ) {
                    warn!(
                        conversation_id,
                        error = %ErrorChain(&error),
                        "Cancelled image fallback rebuild could not initiate exact teardown; retrying through registry barrier"
                    );
                }
                Self::terminate_runtime_until_confirmed(
                    runtime_registry,
                    conversation_id,
                    AgentKillReason::UserCancelled,
                    "cancelled image fallback rebuild",
                )
                .await;
                None
            }
            Err(e) => {
                warn!(error = %ErrorChain(&e), conversation_id, "strip_images_and_rebuild aborted: rebuild failed");
                None
            }
        }
    }

    /// send-loop(plan D3)的故障转移决策入口:在 `consume_with_send_error` 之后调用。
    /// **全部满足**才转移(否则返回 `None`,send-loop 按现状 emit 原始错误):
    /// 1. terminal 是 Error 且 code 命中 [`crate::model_failover::is_provider_fault`];
    /// 2. **pre-response**:本轮未吐任何 assistant Text / 工具动作
    ///    (`!outcome.emitted_response`,plan D4 + review #4)—— 杜绝重复可见输出 /
    ///    工具副作用。上游在返回 500 / EOF 前仍可能已经消耗推理资源,不能保证零重复
    ///    计费;额外 token 风险由较低的 provider 重试次数和有限模型切换次数约束;
    /// 3. 故障转移启用(会话级覆盖否则全局,`enabled == true`);
    /// 4. `switches_done < min(max_switches, queue.len())` —— bounded;
    /// 5. agent 是 **nomi** 实例(plan D7;终端 CLI / ACP 自管模型,排除)。
    /// 6. 持久化 model / pool / template 仍与失败 runtime 构建时的完整 authority
    ///    一致;并发显式 PATCH 已提交时立即放弃旧 failover,绝不覆盖用户的新选择。
    ///
    /// 命中且挑到可用候选 → 换模型 + 重建,返回 `Some(FailoverSwitch)`;
    /// 任一条件不满足 / 队列耗尽 → `None`。
    ///
    /// **不变量**:user-cancel 不会进到这里(取消是 `RelayTerminal::ChannelClosed`
    /// 或非 provider-fault 码,`is_provider_fault` 与 `is_error` 双重过滤);
    /// mid-response 故障被第 2 条挡掉(emit 错误,不转移)。
    pub(crate) async fn maybe_failover_in_send_loop(
        &self,
        conversation_id: &str,
        agent_type: AgentType,
        outcome: &RelayOutcome,
        switches_done: u32,
        tried: &[ProviderWithModel],
        failed_turn_authority: &FailoverAuthoritySnapshot,
        extra_json: &str,
        runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
        turn_generation: u64,
        cancellation: &CancellationToken,
    ) -> Option<FailoverSwitch> {
        if cancellation.is_cancelled() {
            return None;
        }
        // (5) 仅 nomi 自有引擎的普通会话。便宜的早闸(避免无谓加载);真正的强制点
        //     在 inner 的 ACP 边界闸(review #9)。IDMM 不进入有副作用路径。
        if agent_type != AgentType::Nomi {
            return None;
        }
        // (1) provider 故障的终态错误。
        let RelayOutcome {
            terminal,
            emitted_response,
            ..
        } = outcome;
        if !terminal.is_error() {
            return None;
        }
        let Some(code) = terminal.code() else {
            return None;
        };
        if !crate::model_failover::is_provider_fault(code) {
            return None;
        }
        // (2) pre-response:本轮已吐过 Text / 工具动作则不转移(post-response 故障 →
        //     emit 错误,杜绝重复可见输出 / 工具副作用)。这不保证零重复计费:上游在
        //     500 / EOF 前可能已经消耗推理资源;额外 token 风险由低重试和有限切换约束。
        if *emitted_response {
            return None;
        }
        // (3) 启用?(会话级覆盖否则全局)
        let config = self.resolve_failover_config(extra_json).await?;
        if cancellation.is_cancelled() {
            return None;
        }
        if !config.enabled {
            return None;
        }
        // (4) bounded:受 max_switches 与队列长度双重封顶。
        let bound = config.max_switches.min(config.queue.len() as u32);
        if switches_done >= bound {
            warn!(
                conversation_id,
                switches_done,
                max_switches = config.max_switches,
                queue_len = config.queue.len(),
                "Model failover bound reached; surfacing original error"
            );
            return None;
        }

        self.perform_model_failover_inner(
            conversation_id,
            &config,
            tried,
            Some(failed_turn_authority),
            runtime_registry,
            turn_generation,
            cancellation,
        )
        .await
    }

    /// Validate an IDMM failover observation without taking turn ownership.
    ///
    /// Only the send-loop that owns [`crate::runtime_state::AgentTurnHandle`]
    /// may switch models and rebuild a runtime. An out-of-band IDMM probe can
    /// observe a live turn, but it must never turn that observation into a new
    /// execution. In particular, a stale wake-up for a Finished conversation
    /// must fail closed instead of rebuilding and sending "Please continue.".
    ///
    /// `Ok(false)` means that the observation was current but deliberately not
    /// acted on. Missing/stale lifecycle authority is reported as `Conflict`.
    pub async fn idmm_failover_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
        runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
    ) -> Result<bool, AppError> {
        let conv_id = parse_conv_id(conversation_id)?;
        self.ensure_public_mutation_allowed(user_id, conv_id).await?;

        // A preparation-only lease cannot authorize a build or a turn. It only
        // closes the stale-read window against stop/reset/delete while the
        // lifecycle snapshots below are checked.
        let lease = self.begin_public_runtime_preparation(conv_id, user_id)?;
        let cancellation = lease.cancellation_token();
        let runtime_state = self.runtime_state();
        let _preparation_guard = runtime_state
            .acquire_preparation_gate(conv_id, &cancellation)
            .await?;
        lease.ensure_active()?;

        let row = self
            .conversation_repo()
            .get(conv_id)
            .await?
            .filter(|row| row.user_id == user_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("Conversation {conversation_id} not found"))
            })?;
        if row.status.as_deref() != Some("running") {
            return Err(AppError::Conflict(
                "IDMM failover observation requires a durable Running Conversation".to_owned(),
            ));
        }

        let admission = self
            .conversation_repo()
            .get_turn_admission_state(user_id, conv_id)
            .await?;
        if let Some(operation_id) = admission.active_operation_id.as_deref() {
            let receipt = self
                .conversation_repo()
                .get_delivery_receipt(user_id, conv_id, operation_id)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict(
                        "IDMM failover observation has no matching durable turn receipt"
                            .to_owned(),
                    )
                })?;
            if receipt.user_id != user_id
                || receipt.conversation_id != conv_id
                || receipt.operation_id != operation_id
                || receipt.kind != "turn"
                || receipt.status != "accepted"
            {
                return Err(AppError::Conflict(
                    "IDMM failover observation lost its durable turn receipt authority"
                        .to_owned(),
                ));
            }
        }

        let active_turn = runtime_state
            .active_turn_cancellation(conv_id)
            .ok_or_else(|| {
                AppError::Conflict(
                    "IDMM failover observation has no active turn owner".to_owned(),
                )
            })?;
        if active_turn.is_cancelled() || !runtime_registry.has_registered_runtime(conv_id) {
            return Err(AppError::Conflict(
                "IDMM failover observation lost its runtime authority".to_owned(),
            ));
        }

        let runtime = runtime_registry.get_runtime(conv_id).ok_or_else(|| {
            AppError::Conflict(
                "IDMM failover observation requires a live non-quarantined runtime".to_owned(),
            )
        })?;
        if runtime.status() != Some(ConversationStatus::Running) {
            return Err(AppError::Conflict(
                "IDMM failover observation requires a Running runtime".to_owned(),
            ));
        }

        lease.ensure_active()?;
        info!(
            conversation_id,
            "IDMM failover observation declined; the active send-loop exclusively owns failover"
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAILED_PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const PICKED_PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const OTHER_PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000003";

    fn provider(provider_id: &str, model: &str) -> ProviderWithModel {
        ProviderWithModel {
            provider_id: provider_id.to_owned(),
            model: model.to_owned(),
            use_model: Some(model.to_owned()),
        }
    }

    #[test]
    fn failover_atomically_replaces_the_lead_and_preserves_collaborator_order() {
        let encoded = serde_json::to_string(&ExecutionModelPool::Range {
            models: vec![
                selected_model_ref(&provider(FAILED_PROVIDER, "m1")),
                selected_model_ref(&provider(PICKED_PROVIDER, "m2")),
                selected_model_ref(&provider(OTHER_PROVIDER, "m3")),
            ],
        })
        .unwrap();
        let rewritten = rewrite_execution_model_pool_for_failover(
            Some(&encoded),
            &provider(FAILED_PROVIDER, "m1"),
            &provider(PICKED_PROVIDER, "m2"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::from_str::<ExecutionModelPool>(&rewritten).unwrap(),
            ExecutionModelPool::Range {
                models: vec![
                    selected_model_ref(&provider(PICKED_PROVIDER, "m2")),
                    selected_model_ref(&provider(OTHER_PROVIDER, "m3")),
                ],
            }
        );
    }

    #[test]
    fn failover_preserves_inherited_and_explicit_automatic_modes() {
        assert_eq!(
            rewrite_execution_model_pool_for_failover(
                None,
                &provider(FAILED_PROVIDER, "m1"),
                &provider(PICKED_PROVIDER, "m2"),
            )
            .unwrap(),
            None,
        );
        let automatic = serde_json::to_string(&ExecutionModelPool::Automatic).unwrap();
        let rewritten = rewrite_execution_model_pool_for_failover(
            Some(&automatic),
            &provider(FAILED_PROVIDER, "m1"),
            &provider(PICKED_PROVIDER, "m2"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::from_str::<ExecutionModelPool>(&rewritten).unwrap(),
            ExecutionModelPool::Automatic,
        );
    }
}
