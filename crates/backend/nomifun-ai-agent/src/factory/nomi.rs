use std::collections::HashMap;
use std::sync::Arc;

use nomi_agent::session::{Session, SessionManager};
use nomi_config::config::{McpServerConfig, TransportType};
use nomifun_api_types::{
    GatewayMcpConfig, McpServerId, NomiBuildExtra, SessionMcpServer, SessionMcpTransport,
};
use nomifun_common::{
    AppError, DelegationPolicy, ExecutionAuthority, LoopbackCapabilityLease,
    LoopbackCapabilityLeaseSet, ProviderId,
};
use nomifun_db::IMcpServerRepository;
use nomifun_db::ISettingsRepository;
use nomifun_db::models::McpServerRow;
use nomifun_runtime::resolve_command_path;
use tracing::{debug, info, warn};

use crate::runtime_handle::AgentRuntimeHandle;
use crate::factory::AgentFactoryDeps;
use crate::factory::context::FactoryContext;
use crate::factory::platform_table;
use crate::manager::nomi::{
    NomiAgentManager, NomiHostWiring, NomiSummonWiring, sanitize_session_messages,
};
use crate::types::{AgentRuntimeBuildOptions, NomiCompatOverrides, NomiResolvedConfig};

/// Apply the complete ceiling for an authenticated principal that does not own
/// this installation.  This is model-only execution: no OS tools, configured
/// MCP, platform domains, knowledge mounts, autonomous goal loop or Agent
/// delegation.  The non-empty allowlist is intentional because an empty
/// `retain_named` list means "keep everything".
fn apply_model_only_ceiling(overrides: &mut NomiBuildExtra) {
    overrides.computer_use = Some(false);
    overrides.browser_use = Some(false);
    overrides.gateway_mcp_config = None;
    overrides.mcp_server_ids = None;
    overrides.session_mcp_servers.clear();
    overrides.companion = false;
    overrides.companion_id = None;
    overrides.channel_platform = None;
    overrides.knowledge_mounts.clear();
    overrides.knowledge_writeback = false;
    overrides.knowledge_channel_write_enabled = false;
    overrides.allowed_tools = vec!["update_plan".to_owned()];
    overrides.session_mode = Some("default".to_owned());
    overrides.max_turns = Some(1);
    overrides.goal = None;
    overrides.delegation_policy = DelegationPolicy::Disabled;
    // Summon loads local companion memories/skills — installation-owner only.
    overrides.summon = None;
}

fn retarget_resumed_session(session: &mut Session, provider: &str, model: &str) -> bool {
    let changed = session.provider != provider || session.model != model;
    session.provider = provider.to_owned();
    session.model = model.to_owned();
    changed
}

fn persist_repaired_session(manager: &SessionManager, session: &Session) -> Result<(), String> {
    manager.save(session).map_err(|error| error.to_string())?;
    manager
        .update_index_for(session)
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Sanitize a resumed transcript without losing an exact rewind boundary.
///
/// The sanitizer removes messages but never reorders or inserts them. Splitting
/// at the root-turn boundary therefore lets each side be repaired independently
/// and remaps `start_len` to the retained prefix length. No valid tool-call /
/// tool-result pair can cross this boundary because a root user message starts
/// the suffix.
fn sanitize_resumed_session(
    session: &mut Session,
    provider_changed: bool,
) -> crate::manager::nomi::history_sanitize::SessionRepairStats {
    let Some(start_len) = session
        .editable_turn
        .as_ref()
        .map(|checkpoint| checkpoint.start_len)
    else {
        return sanitize_session_messages(&mut session.messages, provider_changed);
    };
    if start_len > session.messages.len() {
        session.editable_turn = None;
        return sanitize_session_messages(&mut session.messages, provider_changed);
    }

    let mut suffix = session.messages.split_off(start_len);
    let mut stats = sanitize_session_messages(&mut session.messages, provider_changed);
    stats.merge(sanitize_session_messages(&mut suffix, provider_changed));
    if let Some(checkpoint) = session.editable_turn.as_mut() {
        checkpoint.start_len = session.messages.len();
    }
    session.messages.append(&mut suffix);
    stats
}

pub(super) async fn build(
    deps: Arc<AgentFactoryDeps>,
    options: AgentRuntimeBuildOptions,
    ctx: FactoryContext,
    authority: ExecutionAuthority,
) -> Result<AgentRuntimeHandle, AppError> {
    let mut overrides: NomiBuildExtra = serde_json::from_value(options.extra)
        .map_err(|error| AppError::BadRequest(format!("Invalid Nomi build options: {error}")))?;
    overrides.user_id = Some(options.user_id.clone());
    // The first-class conversation field is authoritative. Never let an
    // open-ended extra payload override execution policy.
    overrides.delegation_policy = options.delegation_policy;
    let is_instance_owner = authority.controls_host();

    // Gateway entitlement is derived from the immutable principal, never from
    // persisted/open JSON. Process-owned config is injected only after the
    // ownership ceiling has been applied.
    overrides.gateway_mcp_config = None;

    // A non-owner runtime is deliberately model-only.  Hiding a few tools is
    // insufficient because every native shell/ACP process shares the backend's
    // OS uid; the single ceiling below is the enforceable boundary.
    if !is_instance_owner {
        apply_model_only_ceiling(&mut overrides);
    }

    // Merge reusable preset instructions into `system_prompt` (used as
    // `custom_prompt` in Nomi's prompt builder).
    if let Some(rules) = overrides.preset_rules.take() {
        overrides.system_prompt = Some(match overrides.system_prompt.take() {
            Some(existing) => format!("{existing}\n\n{rules}"),
            None => rules,
        });
    }

    // Companion-companion sessions without a persisted persona prompt (channel
    // Channel Agent sessions) get one built fresh per Agent build, so the
    // embedded memory snapshot stays current across restarts. `extra.companion_id`
    // picks the persona (per-bot binding > legacy platform binding); when no
    // companion is bound (None / dead id) there is no persona — an unbound channel
    // is hosted by no companion (no default-companion fallback).
    if overrides.companion
        && overrides.system_prompt.is_none()
        && let Some(provider) = deps.companion_prompt.as_ref()
        && let Some(prompt) = provider
            .build_system_prompt(
                overrides.companion_id.as_deref(),
                overrides.channel_platform.as_deref(),
            )
            .await
    {
        overrides.system_prompt = Some(prompt);
    }

    // In-session companion summon (spec §设计 B2/B3): only owner-authority,
    // non-companion work sessions. The persona is never taken
    // over — the system prompt gains exactly one loading notice; memories are
    // injected per turn by a ContextContributor and stay read-only.
    let summon_config = if is_instance_owner && !overrides.companion {
        overrides.summon.clone()
    } else {
        None
    };
    let mut summon_wiring: Option<NomiSummonWiring> = None;
    if let Some(provider) = deps.companion_summon.as_ref() {
        match summon_config.as_ref() {
            Some(summon) => {
                // Skill materialization (workspace is resolved by now). Manifest
                // ownership makes this idempotent and prunes stale entries when
                // exclusions change. Best-effort: a skill failure degrades the
                // session, it must not block chatting.
                match provider
                    .sync_summon_workspace_skills(
                        &ctx.conversation_id,
                        std::path::Path::new(&ctx.workspace),
                        &summon.companion_id,
                        &summon.skill_exclusions,
                    )
                    .await
                {
                    Ok(linked) => debug!(
                        conversation_id = %ctx.conversation_id,
                        skills = linked.len(),
                        "summon: companion skills materialized into workspace"
                    ),
                    Err(error) => warn!(
                        conversation_id = %ctx.conversation_id,
                        error = %error,
                        "summon: companion skill materialization failed; continuing without them"
                    ),
                }
                let name = provider.companion_name(&summon.companion_id).await;
                let notice = format!(
                    "本会话已装载伙伴「{}」的技能与所选记忆（只读）。伙伴人格不接管本会话。\
                     需要补查伙伴记忆用 recall_memories；发现长期有价值的新事实用 \
                     propose_companion_memory 提议（主人确认后才写入伙伴记忆），宁缺毋滥。",
                    name.as_deref().unwrap_or("（已不存在）")
                );
                overrides.system_prompt = Some(match overrides.system_prompt.take() {
                    Some(existing) if !existing.trim().is_empty() => {
                        format!("{existing}\n\n{notice}")
                    }
                    _ => notice,
                });
                match (
                    provider.summon_memory_sink(&summon.companion_id),
                    provider.summon_proposal_sink(&summon.companion_id),
                    provider.summon_context_sink(summon),
                ) {
                    (Ok(memory_sink), Ok(proposal_sink), Ok(context_sink)) => {
                        summon_wiring = Some(NomiSummonWiring {
                            memory_sink,
                            proposal_sink,
                            context_sink,
                        });
                    }
                    (memory, proposal, context) => {
                        warn!(
                            conversation_id = %ctx.conversation_id,
                            memory_sink_err = ?memory.err().map(|e| e.to_string()),
                            proposal_sink_err = ?proposal.err().map(|e| e.to_string()),
                            context_sink_err = ?context.err().map(|e| e.to_string()),
                            "summon: sink construction failed; session continues without summon tools"
                        );
                    }
                }
            }
            None if is_instance_owner && !overrides.companion => {
                // A cleared (or never-set) summon unloads its manifest-owned
                // skills on the next build. No-op without a manifest; companion
                // threads manage their own manifest and are excluded above.
                if let Err(error) = provider
                    .clear_summon_workspace_skills(
                        &ctx.conversation_id,
                        std::path::Path::new(&ctx.workspace),
                    )
                    .await
                {
                    warn!(
                        conversation_id = %ctx.conversation_id,
                        error = %error,
                        "summon: workspace skill cleanup failed"
                    );
                }
            }
            None => {}
        }
    }

    // A process-owned configuration object is the capability. There is no
    // serializable boolean grant that persisted or client JSON can forge.
    let platform_gateway_entitled = is_instance_owner && overrides.allowed_tools.is_empty();
    overrides.gateway_mcp_config = if platform_gateway_entitled {
        deps.gateway_mcp_config.clone()
    } else {
        None
    };
    if overrides.gateway_mcp_config.is_some() {
        info!(
            conversation_id = %ctx.conversation_id,
            gateway_mcp_port = deps.gateway_mcp_config.as_ref().map(|c| c.port()),
            "gateway_mcp: injected into owner nomi session"
        );
    }
    let has_platform_gateway = overrides.gateway_mcp_config.is_some();

    let (mut extra_mcp_servers, loopback_capability_leases) =
        resolve_mcp_servers(&overrides, &ctx.conversation_id);
    if is_instance_owner && let Some(repo) = deps.mcp_server_repo.as_ref() {
        for (name, config) in load_user_mcp_servers(
            repo.as_ref(),
            overrides.mcp_server_ids.as_deref(),
            &ctx.conversation_id,
        )
        .await
        {
            extra_mcp_servers.entry(name).or_insert(config);
        }
    }
    if is_instance_owner {
        merge_session_snapshot_mcp_servers(
            &mut extra_mcp_servers,
            &overrides.session_mcp_servers,
            &ctx.conversation_id,
        );
    }

    // Per-surface write policy (spec §3.2 unit 5): companion → direct, external
    // IM channel → disabled (P1; opt-in re-enable is P2), regular chat → the
    // binding's staged|direct (staged default). Resolved here where the surface
    // is known from the build extra, reusing the shared rule so the gateway path
    // can't drift. Expressed downstream via existing signals: sink=None disables
    // the tool; the staged bool drives placement.
    let knowledge_write_surface = if overrides.companion {
        nomifun_knowledge::WriteSurface::Companion
    } else if overrides.channel_platform.is_some() {
        nomifun_knowledge::WriteSurface::ExternalChannel
    } else {
        nomifun_knowledge::WriteSurface::RegularChat
    };
    let knowledge_write_policy = nomifun_knowledge::resolve_write_policy(
        knowledge_write_surface,
        &nomifun_knowledge::KnowledgeBinding {
            enabled: true,
            writeback: overrides.knowledge_writeback,
            writeback_mode: overrides
                .knowledge_writeback_mode
                .clone()
                .unwrap_or_else(|| "staged".to_owned()),
            // Threaded from the binding via MountOutcome → build-extra so the
            // external-IM-channel opt-in actually reaches resolve_write_policy;
            // a `..Default::default()` here would pin it to `false` and keep
            // channel write-back permanently disabled on the nomi engine.
            channel_write_enabled: overrides.knowledge_channel_write_enabled,
            ..Default::default()
        },
        &ctx.conversation_id,
    );
    let knowledge_write_enabled = !matches!(
        knowledge_write_policy.mode,
        nomifun_knowledge::WriteMode::Disabled
    );
    let knowledge_writeback_staged = matches!(
        knowledge_write_policy.mode,
        nomifun_knowledge::WriteMode::Staged { .. }
    );

    // Knowledge bases: append the mounted-bases section (per-base TOC +
    // write-back contract) to the system prompt, so nomi-engine sessions
    // (companion companion threads included) see the same knowledge context the
    // ACP path gets via its preset_context.
    overrides.system_prompt = append_knowledge_context(
        overrides.system_prompt.take(),
        &overrides,
        &ctx.conversation_id,
        knowledge_write_enabled,
    );

    // 持久委派提示：对普通桌面会话按 typed delegation policy 塑形，指导 Agent 在
    // 合适场景使用统一 `nomi_delegate` 并在执行画布呈现。该策略只影响提示，不授予
    // 工具能力或改变审批模式。伙伴、渠道/远程和对外服务走各自受限能力面。
    let delegation_hint_available = should_inject_delegation_hint(
        has_platform_gateway,
        overrides.companion,
        overrides.channel_platform.is_some(),
    );
    overrides.system_prompt = compose_delegation_hint(
        overrides.system_prompt.take(),
        delegation_hint_available,
        overrides.delegation_policy,
    );

    // Every native Nomi session — regular desktop chat, companion, IM
    // Channel Agent — must think AND reply in the
    // app's UI language, not a hardcoded one. The persona prompt no longer forces
    // a language, so it is decided HERE from the live system setting and appended
    // LAST (so it wins over the English base prompt / any earlier persisted
    // language line, and the first turn follows the system language). Read live
    // per build → switching the language takes effect on the next new session.
    // External ACP/openclaw agents own their own prompts (built elsewhere) and
    // are intentionally unaffected.
    {
        let lang = read_app_language(deps.settings_repo.as_ref()).await;
        let directive = output_language_directive(&lang);
        overrides.system_prompt = Some(match overrides.system_prompt.take() {
            Some(existing) => format!("{existing}\n\n{directive}"),
            None => directive.to_owned(),
        });
    }

    if !extra_mcp_servers.is_empty() {
        info!(
            conversation_id = %ctx.conversation_id,
            mcp_count = extra_mcp_servers.len(),
            mcp_names = ?extra_mcp_servers.keys().collect::<Vec<_>>(),
            "Injecting MCP servers into nomi session"
        );
    }

    let model_selection = options.model.as_ref().ok_or_else(|| {
        AppError::BadRequest("Nomi runtime requires a provider and model".to_owned())
    })?;
    ProviderId::try_from(model_selection.provider_id.as_str()).map_err(|_| {
        AppError::BadRequest("Nomi runtime requires a canonical provider_id".to_owned())
    })?;
    if model_selection.model.is_empty() || model_selection.model.trim() != model_selection.model {
        return Err(AppError::BadRequest(
            "Nomi runtime requires a trimmed, non-empty model".to_owned(),
        ));
    }
    if model_selection.use_model.as_deref().is_some_and(|model| {
        model.is_empty() || model.trim() != model
    }) {
        return Err(AppError::BadRequest(
            "Nomi runtime model override must be trimmed and non-empty".to_owned(),
        ));
    }
    let provider_id = &model_selection.provider_id;

    let model_id = model_selection
        .use_model
        .as_deref()
        .unwrap_or(&model_selection.model)
        .to_owned();

    let fields = super::provider_config::resolve_provider_fields_with_fallback(
        &deps.provider_repo,
        &deps.provider_model_repo,
        &deps.encryption_key,
        provider_id,
        &model_id,
    )
    .await?;

    let session_directory = deps.data_dir.join("nomi-sessions");

    // Stable identity of this conversation instance (row `created_at`).
    // `accept_owned` rejects a session file whose owner token does not match,
    // providing defense in depth against stale or misplaced derived state.
    let conv_created_ms = options.conversation_created_at.ok_or_else(|| {
        AppError::Internal(format!(
            "conversation {} is missing its v3 runtime owner token",
            ctx.conversation_id
        ))
    })?;
    let owner_token = Some(conv_created_ms.to_string());
    let accept_owned =
        |session: nomi_agent::session::Session| -> Option<nomi_agent::session::Session> {
            if !nomi_agent::session::session_belongs_to(
                session.owner_token.as_deref(),
                session.created_at.timestamp_millis(),
                owner_token
                    .as_deref()
                    .expect("v3 nomi owner token was derived above"),
                conv_created_ms,
            ) {
                warn!(
                    conversation_id = %ctx.conversation_id,
                    session_id = %session.id,
                    "Discarding stale nomi session (belongs to a prior conversation that reused this id); starting fresh"
                );
                return None;
            }
            Some(session)
        };

    let resume_session = {
        let session_mgr = SessionManager::new(session_directory.clone(), 100);
        match session_mgr.load(&ctx.conversation_id) {
            Ok(mut session) => {
                // Drop orphaned assistant tool-calls left behind when the user
                // pressed Stop mid-stream. Strict providers (Ollama-style,
                // some OpenAI-compatible proxies) reject replayed assistants
                // with `tool_calls != null` and `content == null` when no
                // matching tool_result follows. See ELECTRON-1HV / ELECTRON-1J6.
                let provider_changed = session.provider != fields.provider;
                let repair = sanitize_resumed_session(&mut session, provider_changed);
                info!(
                    conversation_id = %ctx.conversation_id,
                    session_id = %session.id,
                    message_count = session.messages.len(),
                    provider_changed,
                    removed_messages = repair.removed_messages,
                    removed_tool_calls = repair.removed_tool_calls,
                    removed_tool_results = repair.removed_tool_results,
                    removed_images = repair.removed_images,
                    removed_thinking = repair.removed_thinking,
                    "Loaded existing nomi session for resume"
                );
                retarget_resumed_session(&mut session, &fields.provider, &fields.model);
                let accepted = accept_owned(session);
                if let Some(ref repaired) = accepted
                    && let Err(error) = persist_repaired_session(&session_mgr, repaired)
                {
                    warn!(
                        conversation_id = %ctx.conversation_id,
                        session_id = %repaired.id,
                        error = %error,
                        "Failed to persist repaired nomi session metadata"
                    );
                }
                accepted
            }
            Err(e) => {
                debug!(
                    conversation_id = %ctx.conversation_id,
                    error = %e,
                    "No current-generation nomi session found, starting fresh"
                );
                None
            }
        }
    };

    // System Settings capability toggles, read LIVE per session (toggling in
    // System Settings affects new sessions without a restart). No setting row →
    // host default. computer-use defaults ON on the desktop build (the only one
    // with the feature); browser-use also defaults ON. Browser execution is
    // delegated through a runtime-scoped BrowserLaneClient to the process-wide
    // BrowserSessionHub, which starts managed Browser Hosts lazily. The toggle
    // only controls whether this runtime exposes Browser tools.
    let computer_use_default = read_bool_pref(
        &deps,
        PREF_COMPUTER_USE,
        cfg!(feature = "computer-use") || env_flag("NOMIFUN_COMPUTER_USE"),
    )
    .await;
    // browser-use has a cargo-feature gate (`browser-use`, desktop builds); on
    // those builds it defaults **ON** (user decision). The main-process
    // BrowserSessionHub is the only Chromium/profile owner and shares managed
    // Primary or Crawl Hosts across authorized Lanes. A Nomi runtime receives
    // only a BrowserLaneClient. Builds without the feature register no Browser
    // tools. `NOMIFUN_BROWSER_USE` forces the setting on for parity/testing.
    let browser_use_default = read_bool_pref(
        &deps,
        PREF_BROWSER_USE,
        cfg!(feature = "browser-use") || env_flag("NOMIFUN_BROWSER_USE"),
    )
    .await;
    // F1-sec: evaluate「全权模式」LIVE 值（裁决⑨，default-deny）。用户在 System Settings 显式 opt-in
    // 的 `agent.browserUse.fullPower` 开关，每会话构造时 LIVE 读（read_bool_pref 范式，与上面的启用开关
    // 同源），灌进 BrowserConfig.full_power，由 Hub-backed Browser tool adapter 在进入
    // BrowserLaneClient 前执行 evaluate gate。默认 OFF（host_default=false）——evaluate 是最高危
    // 逃生舱，无 opt-in 即封死。**绝不看 session_mode**（不变量⑧）。
    let browser_full_power_default = read_bool_pref(
        &deps,
        PREF_BROWSER_FULL_POWER,
        env_flag("NOMIFUN_BROWSER_FULL_POWER"),
    )
    .await;
    // SD-6: 持久登录 LIVE 值（DESIGN §16/§27 互斥约束）。产品默认 ON（host_default=true）——持久登录
    // 开启时与全权互斥（evaluate Blocked）。用户可在 System Settings 关闭以解除互斥。
    let browser_persistent_login_default =
        read_bool_pref(&deps, PREF_BROWSER_PERSISTENT_LOGIN, true).await;
    // P7A: site-memory LIVE 值。host_default=false（OFF）——把站点交互持久化到磁盘是隐私相关行为，
    // 须用户在 System Settings 显式 opt-in。
    let browser_site_memory_default = read_bool_pref(&deps, PREF_BROWSER_SITE_MEMORY, false).await;
    // Phase D: takeover/approval gate LIVE value. host_default=true (ON): install a gate by
    // default. Non-yolo sessions can prompt for risky Browser actions / gated cross-origin
    // POSTs; full-auto/yolo sessions still install the gate, but the gate approves directly.
    let browser_takeover_default = read_bool_pref(&deps, PREF_BROWSER_TAKEOVER, true).await;
    let browser_unrestricted_approval_default =
        read_bool_pref(&deps, PREF_BROWSER_UNRESTRICTED_APPROVAL, false).await;
    // P7B: visual-fallback LIVE 值。host_default=false（OFF）——每次兜底都过一遍视觉模型，有额外 token
    // 成本，须用户在 System Settings 显式 opt-in。
    let browser_visual_fallback_default =
        read_bool_pref(&deps, PREF_BROWSER_VISUAL_FALLBACK, false).await;
    // Browser management is status-only. Primary Chromium is always shown in
    // its managed external window; historical embedded/headless/silent values
    // are frontend migration inputs only and never affect runtime headlessness.
    // Browser Host 可执行文件来源偏好（与 silent 正交）。host_default="system"，优先系统安装
    // 的 Chrome/Edge，未探测到时回退 managed。该值不授予 runtime 所有权：主进程
    // BrowserSessionHub 统一创建/共享 Host，Primary 使用应用管理的稳定 profile，Crawl 使用临时 profile。
    let browser_source_default =
        read_string_pref(&deps, PREF_BROWSER_SOURCE, BROWSER_SOURCE_DEFAULT).await;

    let browser_use_enabled = overrides.browser_use.unwrap_or(browser_use_default);

    let persistent_login_key = browser_use_enabled.then_some(deps.encryption_key);

    #[cfg(feature = "browser-use")]
    let browser_lane_binding = if browser_use_enabled {
        match deps.browser_lane_provider.as_ref() {
            Some(slot) => {
                let provider = slot.get().ok_or_else(|| {
                    AppError::Internal(
                        "browser use is enabled but the process-wide Browser Session Hub provider \
                         has not been installed"
                            .to_owned(),
                    )
                })?;
                let runtime_instance_id = format!(
                    "native:{}:{}",
                    ctx.conversation_id,
                    uuid::Uuid::now_v7()
                );
                Some(
                    provider
                        .issue(
                            crate::factory::browser_lane::TrustedBrowserRuntimeContext {
                                user_id: options.user_id.clone(),
                                conversation_id: Some(ctx.conversation_id.clone()),
                                runtime_instance_id,
                                agent_id: Some("nomi".to_owned()),
                                // Execution ownership is resolved by the host
                                // provider from the authoritative persisted
                                // ConversationLink. It is never read from
                                // `options.extra`.
                                execution_id: None,
                                step_id: None,
                                attempt_id: None,
                                surface:
                                    nomifun_browser_platform::BrowserSurface::Native,
                            },
                        )
                        .await?,
                )
            }
            // Explicit standalone/test composition. Production AppServices
            // always supplies a slot, so a provider outage cannot create an
            // alternate browser owner outside BrowserSessionHub.
            None => None,
        }
    } else {
        None
    };

    let config = NomiResolvedConfig {
        provider: fields.provider,
        api_key: fields.api_key,
        model: fields.model.clone(),
        base_url: fields.base_url,
        system_prompt: overrides.system_prompt,
        max_tokens: overrides.max_tokens,
        max_turns: overrides.max_turns,
        context_limit: fields.context_limit.map(|v| v as u64),
        compat_overrides: fields.compat_overrides,
        session_directory,
        // 默认授权模式 = 全自动（yolo）。产品决策：所有 nomi 会话默认自动批准
        // 标准工具类别（info/edit/exec/mcp —— 文件编辑 / Shell / 标准工具 & MCP），
        // 不再反复弹授权框。理由：
        //  - companion / IM Channel Agent 本就无审批 UI（其首个 gateway/file/bash
        //    工具调用会 park 在 rx.await，turn 永不 finish → 聊天永久「思考中」），
        //    所以它们历来必须 yolo；现在把这一默认推广到普通桌面会话。
        //  - **显式 `extra.session_mode` 仍胜出**：用户在权限选择器里手动降级为
        //    `default` / `auto_edit` 会写偏好并经 extra 传入，这里的 `.or_else` 让显式值
        //    优先，降级正常生效。
        //  - Full-power evaluate and desktop-control toggles remain separate System Settings
        //    and are not granted by session_mode. Browser approval prompts are ordinary
        //    permission friction: full-auto/yolo is honored by the Browser approval gate, so
        //    gated Browser actions approve without UI.
        session_mode: overrides
            .session_mode
            .clone()
            .or_else(|| Some("yolo".to_owned())),
        extra_mcp_servers,
        loopback_capability_leases,
        bedrock_config: fields.bedrock_config,
        computer_use: overrides.computer_use.unwrap_or(computer_use_default),
        browser_use: browser_use_enabled,
        // Browser Host 可执行文件来源偏好；BrowserSessionHub 仍是唯一 owner。
        browser_source: browser_source_default,
        // F1-sec: 全权模式 LIVE 值（无 per-session override，纯 client_preferences 全局开关）。
        browser_full_power: browser_full_power_default,
        // SD-6: 持久登录 LIVE 值（产品默认 ON，无 per-session override）。
        browser_persistent_login: browser_persistent_login_default,
        // P7A: site-memory LIVE 值（默认 OFF，opt-in；无 per-session override）。
        browser_site_memory: browser_site_memory_default,
        // Phase D: takeover/审批 gate LIVE 值（产品默认 ON；无 per-session override）。
        browser_takeover: browser_takeover_default,
        browser_unrestricted_approval: browser_unrestricted_approval_default,
        // P7B: visual-fallback LIVE 值（默认 OFF，opt-in；无 per-session override）。
        browser_visual_fallback: browser_visual_fallback_default,
        goal: overrides.goal.clone().map(|g| {
            nomi_agent::goal::runtime::GoalSpec::new(
                g.objective,
                g.max_auto_continuations.unwrap_or(8),
            )
        }),
        // Persistent-login encryption key; absent when browser-use is off.
        persistent_login_key,
        // Owning conversation instance identity — the nomi manager stamps it
        // onto the session after build so a future reused id is rejected.
        owner_token: owner_token.clone(),
        // Host composition is backend-authoritative, never user config. A
        // Platform Gateway owns persistent AgentExecution; secondary users
        // cannot install host execution. Only trusted no-gateway standalone
        // sessions receive the embedded adapter.
        install_embedded_agent_execution: should_install_embedded_agent_execution(
            has_platform_gateway,
            is_instance_owner,
        ),
        // Per-session 工具白名单（受限角色的 Agent attempt；普通会话恒空）。
        allowed_tools: overrides.allowed_tools.clone(),
        // 原生文件工具写根：本地桌面全权（None），渠道会话收窄到工作区。
        // 与 gateway file-service 的 PathAuthority 同一信任模型（file-access spec）。
        write_root: if is_instance_owner {
            resolve_native_write_root(
                overrides.channel_platform.as_deref(),
                &ctx.workspace,
            )
        } else {
            Some(ctx.workspace.clone())
        },
    };

    // Scope of the native knowledge_search / knowledge_read tools, derived
    // from the mounted bases.
    let knowledge_kb_ids: Vec<nomifun_common::KnowledgeBaseId> = overrides
        .knowledge_mounts
        .iter()
        .map(|m| m.knowledge_base_id.clone())
        .collect();

    // Write-back ("回血") wiring for the native knowledge_write tool. The sink
    // is passed only when the resolved policy permits writing (channel sessions
    // resolve to Disabled → sink=None → tool not registered). `(id, name)` lets
    // the tool resolve the base the model names back to the opaque id. The
    // staged/direct decision was made above by the per-surface policy.
    let knowledge_write_bases: Vec<(nomifun_common::KnowledgeBaseId, String)> = overrides
        .knowledge_mounts
        .iter()
        .map(|m| (m.knowledge_base_id.clone(), m.name.clone()))
        .collect();
    let knowledge_writeback_sink = if knowledge_write_enabled {
        deps.knowledge_writeback.clone()
    } else {
        None
    };

    let knowledge_prelude: Option<String> = if overrides.knowledge_mounts.is_empty() {
        None
    } else {
        let names: Vec<&str> = overrides
            .knowledge_mounts
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        Some(format!(
            "[Knowledge bases mounted: {}] Before answering, if this task relates to any of these, \
             call the knowledge_search tool first and open the matching document. Do not rely on \
             memory for topics these bases cover.",
            names.join(", ")
        ))
    };

    let conv_id_for_cron = ctx.conversation_id.clone();
    let owner_id_for_cron = overrides
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|owner| !owner.is_empty())
        .map(ToOwned::to_owned);
    // SSH-bound session: connect the saved host now (decrypt credential, dial,
    // open shell + SFTP) so the runtime gets the remote tool family. A binding
    // without a configured provider, or a failed connect, fails the build with a
    // clear error rather than silently running against the local machine.
    let ssh_backend = if let Some(ssh_host_id) = overrides.ssh_host_id.clone() {
        let user_id = overrides.user_id.clone().unwrap_or_default();
        let remote_cwd = overrides
            .ssh_remote_cwd
            .clone()
            .unwrap_or_else(|| ".".to_string());
        match &deps.ssh_provider {
            Some(provider) => Some(
                provider
                    .connect(&user_id, &ssh_host_id, &remote_cwd)
                    .await
                    .map_err(|e| AppError::Internal(format!("SSH connect failed: {e}")))?,
            ),
            None => {
                return Err(AppError::BadRequest(
                    "conversation is bound to an SSH host but SSH support is not configured".into(),
                ));
            }
        }
    } else {
        None
    };
    let host_wiring = NomiHostWiring {
        #[cfg(feature = "browser-use")]
        browser_lane_binding,
        ssh_backend,
    };
    let agent = NomiAgentManager::new_with_host_wiring(
        ctx.conversation_id,
        ctx.workspace,
        config,
        resume_session,
        is_instance_owner.then(|| deps.requirement_sink.clone()).flatten(),
        if is_instance_owner && overrides.companion {
            deps.companion_sink.clone()
        } else {
            None
        },
        is_instance_owner.then(|| deps.knowledge_retrieval.clone()).flatten(),
        knowledge_kb_ids,
        knowledge_prelude,
        knowledge_writeback_sink,
        knowledge_write_bases,
        knowledge_writeback_staged,
        if is_instance_owner && overrides.companion {
            deps.companion_skill_sink.clone()
        } else {
            None
        },
        summon_wiring,
        host_wiring,
    )
    .await?;
    // Native cron tools persist background work and can recursively create
    // model traffic. They are host-control capabilities, not part of the
    // secondary principal's model-only ceiling. Register them only for the
    // installation owner, after the manager has been assembled.
    if is_instance_owner
        && let (Some(make_sink), Some(owner_id)) =
        (deps.cron_sink_factory.as_ref(), owner_id_for_cron.as_deref())
    {
        agent
            .register_cron_sink(make_sink(owner_id, &conv_id_for_cron))
            .await;
    }
    Ok(AgentRuntimeHandle::Nomi(Arc::new(agent)))
}

/// Host-level default for opt-in tool capabilities ("1"/"true" enables).
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// `client_preferences` keys for the System Settings capability toggles
/// (written by the frontend via `configService`, read here per session).
const PREF_COMPUTER_USE: &str = "agent.computerUse";
const PREF_BROWSER_USE: &str = "agent.browserUse";
/// **F1-sec**: browser-use evaluate「全权模式」开关（裁决⑨）。`true` → evaluate 放行（仍受与持久登录
/// 互斥约束）；缺/`false` → evaluate 默认 OFF（最高危逃生舱 default-deny）。前端 System Settings 写。
const PREF_BROWSER_FULL_POWER: &str = "agent.browserUse.fullPower";
/// **SD-6**: browser-use 持久登录开关（裁决⑨ 互斥约束）。`true`（产品默认）→ 与全权互斥；`false` → 解除互斥。
const PREF_BROWSER_PERSISTENT_LOGIN: &str = "agent.browserUse.persistentLogin";
/// **P7A**: browser-use 站点记忆开关（opt-in，隐私相关）。`true` → 跨会话记住站点结构 + 注入 hints；
/// 缺/`false`（host_default）→ OFF（不持久化、零行为变化）。前端 System Settings 写。
const PREF_BROWSER_SITE_MEMORY: &str = "agent.browserUse.siteMemory";
/// **Phase D**: browser-use 人机接管 + 跨域 POST 审批 gate。`true` → 注入审批 gate
/// （默认会话浮给用户；full-auto/yolo 直接通过）；缺失时 host_default=true。前端 System Settings 写。
const PREF_BROWSER_TAKEOVER: &str = "agent.browserUse.takeover";
/// **Phase D**: browser-use 显式无限制审批开关。`true` → Browser approval gate 不再浮出确认。
const PREF_BROWSER_UNRESTRICTED_APPROVAL: &str = "agent.browserUse.unrestrictedApproval";
/// **P7B**: browser-use 视觉兜底点击（opt-in，有 token 成本）。`true` → DOM/aria 锚定失败时截图交视觉
/// 模型定位再点；缺/`false`（host_default）→ OFF（不注入 locator、零行为变化）。前端 System Settings 写。
const PREF_BROWSER_VISUAL_FALLBACK: &str = "agent.browserUse.visualFallback";
/// Browser Host 可执行文件来源偏好（与 silent 正交）。`"managed"` = 内置/下载 CfT；
/// `"system"`（默认）= 系统 Chrome/Edge 本体优先（未探到回退 managed）。前端写入偏好；
/// 主进程 BrowserSessionHub 仍统一拥有 Host 和应用管理 profile。
const PREF_BROWSER_SOURCE: &str = "agent.browserUse.source";
/// Browser Host 来源默认值（无设置行/无 client_prefs 时）：系统安装的 Chrome / Edge。
const BROWSER_SOURCE_DEFAULT: &str = "system";

/// Read a boolean `client_preferences` toggle live, falling back to
/// `host_default` when there is no setting row (fresh install) or no
/// client_prefs repo is wired. The frontend `configService` persists bare JSON
/// (`true`/`false`); the raw settings API may store the quoted string forms.
/// Read per session so toggling the setting affects new sessions without a
/// restart.
async fn read_bool_pref(deps: &AgentFactoryDeps, key: &str, host_default: bool) -> bool {
    let Some(repo) = deps.client_prefs.as_ref() else {
        return host_default;
    };
    match repo.get_by_keys(&[key]).await {
        Ok(rows) => rows
            .into_iter()
            .find(|r| r.key == key)
            .map(|r| parse_bool_pref(&r.value, host_default))
            .unwrap_or(host_default),
        Err(_) => host_default,
    }
}

/// Shared boolean-preference parse semantics for the `agent.browserUse.*`
/// toggles this factory shares with the Hub.
///
/// Deliberately identical to the boot-time reader in nomifun-app
/// `load_browser_startup_preferences` (services.rs), so Hub startup policy and
/// this per-session policy can never disagree about the same stored row:
/// quotes are trimmed (a raw settings-API write stores JSON strings like
/// `"false"`), an explicit opposite value flips the toggle, and any junk value
/// resolves to `host_default` — default-ON toggles (e.g. persistentLogin)
/// parse as `value != "false"`, default-OFF toggles (e.g. fullPower) parse as
/// `value == "true"`.
fn parse_bool_pref(value: &str, host_default: bool) -> bool {
    let value = value.trim().trim_matches('"');
    if host_default {
        value != "false"
    } else {
        value == "true"
    }
}

/// Read a string `client_preferences` value live, falling back to `host_default`
/// when there is no setting row (fresh install), no client_prefs repo is wired, or
/// the stored value is blank. Mirrors [`read_bool_pref`] for stringly settings
/// (e.g. `agent.browserUse.source` = `"managed"`/`"system"`). Read per session so
/// toggling the setting affects new sessions without a restart.
async fn read_string_pref(deps: &AgentFactoryDeps, key: &str, host_default: &str) -> String {
    let Some(repo) = deps.client_prefs.as_ref() else {
        return host_default.to_owned();
    };
    match repo.get_by_keys(&[key]).await {
        Ok(rows) => rows
            .into_iter()
            .find(|r| r.key == key)
            .map(|r| r.value.trim().trim_matches('"').to_owned())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| host_default.to_owned()),
        Err(_) => host_default.to_owned(),
    }
}

/// App UI language default — the final fallback when no language is persisted
/// AND the host OS locale is unavailable. Matches
/// `SystemSettingsResponse::default().language` in `nomifun-api-types`.
const DEFAULT_APP_LANGUAGE: &str = "en-US";

/// Normalize an arbitrary locale tag to the output-language directive's supported
/// axis. [`output_language_directive`] only distinguishes `zh-CN` from everything
/// else, so any Chinese locale (`zh`, `zh_CN`, `zh-Hans`, `zh-Hans-CN`, …) folds
/// to `zh-CN`; any other tag is returned normalized (→ English directive).
fn normalize_lang(code: &str) -> String {
    let c = code.trim().replace('_', "-");
    if c.to_ascii_lowercase().starts_with("zh") {
        "zh-CN".to_owned()
    } else {
        c
    }
}

/// Resolve the effective app language: an explicitly **persisted** System-Settings
/// value wins; otherwise fall back to the host **OS locale** (so a fresh install
/// on a Chinese system replies in Chinese without the owner touching settings —
/// 首轮跟随系统语言); finally [`DEFAULT_APP_LANGUAGE`]. `os_locale` is injected so
/// the resolution is deterministically unit-testable.
fn resolve_language(persisted: Option<&str>, os_locale: Option<&str>) -> String {
    if let Some(l) = persisted.map(str::trim).filter(|s| !s.is_empty()) {
        return normalize_lang(l);
    }
    if let Some(l) = os_locale.map(str::trim).filter(|s| !s.is_empty()) {
        return normalize_lang(l);
    }
    DEFAULT_APP_LANGUAGE.to_owned()
}

/// Read the effective app UI language live (mirrors `read_bool_pref`): the
/// persisted System-Settings value if set, else the host OS locale, else
/// [`DEFAULT_APP_LANGUAGE`]. Read per build so a language switch — or first-run OS
/// detection — takes effect on the next agent (re)build. Takes the bare repo
/// option (not the whole deps) so the persisted branch is trivially testable.
async fn read_app_language(settings_repo: Option<&Arc<dyn ISettingsRepository>>) -> String {
    let persisted = match settings_repo {
        Some(repo) => match repo.get_settings().await {
            Ok(Some(settings)) if !settings.language.trim().is_empty() => Some(settings.language),
            _ => None,
        },
        None => None,
    };
    resolve_language(persisted.as_deref(), sys_locale::get_locale().as_deref())
}

/// Map a stored app-language code to the output-language directive appended LAST
/// to every nomi session's system prompt. Covers BOTH the final reply and the
/// model's reasoning / thinking, phrased as an explicit override so it wins over
/// the English base prompt and any earlier (possibly persisted) language line,
/// while still letting the owner pull the session into another language by
/// writing in it. Unknown / empty / en-US all resolve to English (the app
/// default); only the supported `zh-CN` selects Chinese (supported set lives in
/// `nomifun-system`).
fn output_language_directive(lang: &str) -> &'static str {
    match lang {
        "zh-CN" => {
            "【输出语言】无论上文的指令或记忆使用何种语言，请始终用简体中文进行思考与回复\
                    （包括你的推理/思考过程）——除非主人主动用其他语言和你说话，或明确要求你换一种语言。"
        }
        _ => {
            "[Output language] Regardless of the language used in the instructions or memories \
              above, always think and reply in English (including your reasoning / thinking \
              process) — unless the owner writes to you in another language or explicitly asks \
              you to switch."
        }
    }
}

/// Append the knowledge-base section to the system prompt when the
/// conversation service mounted bases into the workspace. Rendering is
/// delegated to the shared builder
/// (`nomifun_knowledge::context::build_knowledge_context`,
/// `PromptSection` format) so nomi-engine sessions (companion companion threads
/// included) see exactly the same knowledge context the ACP path gets via
/// its preset_context — single source of truth, no more structural copies.
fn append_knowledge_context(
    base: Option<String>,
    config: &NomiBuildExtra,
    conversation_id: &str,
    has_write_tool: bool,
) -> Option<String> {
    use nomifun_knowledge::context::{
        KnowledgeContextFormat, KnowledgeContextOptions, build_knowledge_context,
    };

    let section = build_knowledge_context(
        &config.knowledge_mounts,
        &KnowledgeContextOptions {
            format: KnowledgeContextFormat::PromptSection,
            writeback: config.knowledge_writeback,
            writeback_mode: config.knowledge_writeback_mode.as_deref(),
            writeback_eagerness: config.knowledge_writeback_eagerness.as_deref(),
            target_id: conversation_id,
            has_search_tool: true,
            // The nomi engine registers the native knowledge_write tool whenever
            // the backend wired a write-back sink; the contract must then point
            // the model at that tool, not the (unreachable) generic Write path.
            has_write_tool,
        },
    );
    match (base, section) {
        (Some(ctx), Some(section)) => Some(format!("{ctx}\n\n{section}")),
        (base, None) => base,
        (None, section) => section,
    }
}

/// Standard persistent-delegation guidance for an ordinary desktop session.
pub(crate) const DELEGATION_STANDARD_HINT: &str = "遇到可并行的独立工作，或需要成体系拆解的复杂多步目标时，统一使用 `nomi_delegate`：独立工作传 `strategy=parallel` 和 tasks，复杂目标传 `strategy=planned` 和 goal，让规划器生成依赖 DAG。每个受委派的 Agent 都在右侧画布实时显示状态与转录。顶层会话委派会创建一个 Agent Execution；执行中的 Attempt 再委派只会向同一个 Execution 追加 Step，不会创建子执行。拿到 execution_id（以及追加时的 added_step_ids）后立即结束本轮，不要轮询等待或重复创建。全部结束时系统会把持久化最终结果直接作为 assistant 回执写入顶层会话，不会再启动一轮模型汇总；用户主动询问进度时才用 `nomi_execution_get` 读取一次。简单或单步问题直接作答，无需委派。";

/// Additional guidance for [`DelegationPolicy::PreferParallel`].
pub(crate) const DELEGATION_PREFER_PARALLEL_HINT: &str = "本会话偏好并行委派：面对每个请求都先明确评估能否拆成多个互相独立的 Agent 工作，并在确有并行收益时优先使用 `nomi_delegate`。只有任务确实单步可答或无法安全拆分时才直接处理；不要为了形式并行制造重复工作。";

/// 是否给本会话追加常驻 delegation 提示（纯策略，可单测）。提示点名的
/// `nomi_delegate` 工具只随进程签发的桌面网关能力提供给本地可信会话，
/// 故必须 `has_gateway` 才注入——否则会话拿不到这些工具，提示就成了空头支票（远程
/// WebUI 未授信、对外服务被钳制关网关等）。伙伴、渠道/远程和对外服务
/// 都走各自的受限能力面，故一并排除。
pub(crate) fn should_inject_delegation_hint(
    has_gateway: bool,
    is_companion: bool,
    is_channel: bool,
) -> bool {
    has_gateway && !is_companion && !is_channel
}

/// Append typed persistent-delegation guidance without replacing preset,
/// persona, or knowledge context. Unavailable surfaces and
/// [`DelegationPolicy::Disabled`] preserve `base` unchanged.
pub(crate) fn compose_delegation_hint(
    base: Option<String>,
    available: bool,
    policy: DelegationPolicy,
) -> Option<String> {
    if !available || policy == DelegationPolicy::Disabled {
        return base;
    }
    let hint = match policy {
        DelegationPolicy::Automatic => DELEGATION_STANDARD_HINT.to_owned(),
        DelegationPolicy::PreferParallel => {
            format!("{DELEGATION_STANDARD_HINT}\n\n{DELEGATION_PREFER_PARALLEL_HINT}")
        }
        DelegationPolicy::Disabled => unreachable!("disabled policy returned above"),
    };
    Some(match base {
        Some(existing) if !existing.is_empty() => format!("{existing}\n\n{hint}"),
        _ => hint,
    })
}

/// Backend-authoritative host composition gate. It is intentionally derived
/// from resolved runtime authority rather than user configuration: Platform
/// Gateway owns persistent AgentExecution, and untrusted identities never
/// receive an embedded host execution surface.
pub(crate) fn should_install_embedded_agent_execution(
    has_platform_gateway: bool,
    is_instance_owner: bool,
) -> bool {
    !has_platform_gateway && is_instance_owner
}

/// 原生文件工具（Write/Edit/ApplyPatch）的写根钳制解析（纯函数，可单测）。与
/// gateway `caps_files::file_authority` 同一信任模型:仅**本地桌面**会话
/// (无渠道平台)获得不钳制(`None` = OS 用户全权,今日行为);渠道(channel)会话
/// 一律收窄到会话工作区(`Some(workspace)`)。工作区为空时回退 `None`
/// (无从钳制则不劣于今日行为)。
pub(crate) fn resolve_native_write_root(
    channel_platform: Option<&str>,
    workspace: &str,
) -> Option<String> {
    let is_channel = channel_platform.map(str::trim).is_some_and(|s| !s.is_empty());
    if !is_channel {
        return None;
    }
    let ws = workspace.trim();
    if ws.is_empty() { None } else { Some(ws.to_owned()) }
}

/// Map Nomi DB platform name to the nomi provider identifier.
///
/// Mirrors the frontend `src/process/agent/nomi/envBuilder.ts` mapping. Pure
/// table lookup against [`platform_table::PLATFORM_CHAT_RULES`] (default row:
/// `openai`), except the new-api gateway special case: for the `new-api`
/// platform the model's per-row `protocol` override (from its
/// `provider_models` row) takes precedence over the table.
pub(crate) fn map_nomi_provider(platform: &str, protocol: Option<&str>) -> String {
    if platform == "new-api" && protocol == Some("anthropic") {
        return "anthropic".to_owned();
    }

    platform_table::platform_chat_rule(platform).nomi_provider.to_owned()
}

/// Resolve base_url and compat overrides for the nomi provider.
///
/// `is_full_url` bypasses every platform rule (the configured URL is the
/// request URL, minus trailing `/`, with an empty `api_path`). Otherwise the
/// platform's [`platform_table::UrlRule`] decides:
/// - `GeminiOpenAiCompat`: prepend `/v1beta/openai`, pin `api_path` to
///   `/chat/completions`
/// - `ConfiguredChatBase`: keep the configured base (nonstandard version
///   path), pin `api_path` to `/chat/completions`
/// - `StripTrailingV1` (default row): strip trailing `/v1` (nomi appends its
///   own path); OpenAI official (`api.openai.com`, mapped provider `openai`)
///   additionally sets `max_tokens_field = max_completion_tokens`
pub(crate) fn resolve_nomi_url_and_compat(
    platform: &str,
    raw_base_url: &str,
    mapped_provider: &str,
    is_full_url: bool,
) -> (Option<String>, NomiCompatOverrides) {
    let mut compat = NomiCompatOverrides::default();

    if is_full_url {
        let trimmed = raw_base_url.trim_end_matches('/');
        compat.api_path = Some(String::new());
        return (Some(trimmed.to_owned()), compat);
    }

    match platform_table::platform_chat_rule(platform).url_rule {
        platform_table::UrlRule::GeminiOpenAiCompat => {
            let trimmed = raw_base_url.trim_end_matches('/');
            let base = format!("{trimmed}/v1beta/openai");
            compat.api_path = Some("/chat/completions".to_owned());
            (Some(base), compat)
        }
        platform_table::UrlRule::ConfiguredChatBase => {
            let base = raw_base_url.trim_end_matches('/').to_owned();
            compat.api_path = Some("/chat/completions".to_owned());
            (Some(base).filter(|u| !u.is_empty()), compat)
        }
        platform_table::UrlRule::StripTrailingV1 => {
            let normalized = normalize_nomi_base_url(raw_base_url);
            let base_url = Some(normalized).filter(|u| !u.is_empty());

            if mapped_provider == "openai" && is_openai_host(raw_base_url) {
                compat.max_tokens_field = Some("max_completion_tokens".to_owned());
            }

            (base_url, compat)
        }
    }
}

fn is_openai_host(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .map(|rest| rest == "api.openai.com" || rest.starts_with("api.openai.com/"))
        .unwrap_or(false)
}

/// Strip trailing `/v1`, `/v1/`, or lone `/` from a base URL so that
/// nomi can append its own path suffix (`/v1/messages`, `/v1/chat/completions`).
fn normalize_nomi_base_url(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_owned()
}

pub(crate) fn resolve_bedrock_config(
    json: Option<&str>,
) -> Option<nomi_config::config::BedrockConfig> {
    let bc: nomifun_api_types::BedrockConfig = serde_json::from_str(json?).ok()?;
    Some(nomi_config::config::BedrockConfig {
        region: Some(bc.region),
        access_key_id: bc.access_key_id,
        secret_access_key: bc.secret_access_key,
        session_token: None,
        profile: bc.profile,
    })
}

async fn load_user_mcp_servers(
    repo: &dyn IMcpServerRepository,
    selected_ids: Option<&[McpServerId]>,
    conversation_id: &str,
) -> HashMap<String, McpServerConfig> {
    let rows_result = match selected_ids {
        Some(ids) => {
            let ids = ids.iter().map(ToString::to_string).collect::<Vec<_>>();
            repo.list_by_ids_any(&ids).await
        }
        None => repo.list().await,
    };
    let rows = match rows_result {
        Ok(r) => r,
        Err(err) => {
            warn!(
                conversation_id,
                error = %err,
                "user_mcp: list() failed; skipping injection"
            );
            return HashMap::new();
        }
    };

    let mut servers = HashMap::new();
    for row in rows {
        let selected = selected_ids
            .map(|ids| {
                ids.iter()
                    .any(|id| id.as_str() == row.mcp_server_id)
            })
            .unwrap_or(row.enabled);
        if !selected || row.builtin {
            continue;
        }

        match row_to_mcp_server_config(&row) {
            Ok(config) => {
                servers.insert(row.name.clone(), config);
            }
            Err(err) => {
                warn!(
                    conversation_id,
                    mcp_server_id = %row.mcp_server_id,
                    server_name = %row.name,
                    error = %err,
                    "user_mcp: failed to convert row; skipping"
                );
            }
        }
    }

    servers
}

fn row_to_mcp_server_config(row: &McpServerRow) -> Result<McpServerConfig, String> {
    let value: serde_json::Value = serde_json::from_str(&row.transport_config)
        .map_err(|e| format!("invalid transport_config JSON: {e}"))?;

    match row.transport_type.as_str() {
        "stdio" => {
            let command = value
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "stdio: missing command".to_owned())?;
            let resolved_command = resolve_stdio_command(command);
            let args = value
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let env = value
                .get("env")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();

            Ok(McpServerConfig {
                transport: TransportType::Stdio,
                command: Some(resolved_command),
                args: Some(args),
                env: Some(env),
                url: None,
                headers: None,
                deferred: Some(false),
            })
        }
        "http" | "streamable_http" => {
            let url = value
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "http: missing url".to_owned())?;
            let headers = value
                .get("headers")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();

            Ok(McpServerConfig {
                transport: TransportType::StreamableHttp,
                command: None,
                args: None,
                env: None,
                url: Some(url.to_owned()),
                headers: Some(headers),
                deferred: Some(false),
            })
        }
        "sse" => {
            let url = value
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "sse: missing url".to_owned())?;
            let headers = value
                .get("headers")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();

            Ok(McpServerConfig {
                transport: TransportType::Sse,
                command: None,
                args: None,
                env: None,
                url: Some(url.to_owned()),
                headers: Some(headers),
                deferred: Some(false),
            })
        }
        other => Err(format!("unsupported transport_type: {other}")),
    }
}

fn session_server_to_mcp_server_config(
    server: &SessionMcpServer,
) -> Result<McpServerConfig, String> {
    match &server.transport {
        SessionMcpTransport::Stdio { command, args, env } => {
            if command.is_empty() {
                return Err("stdio: missing command".to_owned());
            }
            Ok(McpServerConfig {
                transport: TransportType::Stdio,
                command: Some(resolve_stdio_command(command)),
                args: Some(args.clone()),
                env: Some(env.clone()),
                url: None,
                headers: None,
                deferred: Some(false),
            })
        }
        SessionMcpTransport::Http { url, headers } => {
            if url.is_empty() {
                return Err("http: missing url".to_owned());
            }
            Ok(McpServerConfig {
                transport: TransportType::StreamableHttp,
                command: None,
                args: None,
                env: None,
                url: Some(url.clone()),
                headers: Some(headers.clone()),
                deferred: Some(false),
            })
        }
        SessionMcpTransport::Sse { url, headers } => {
            if url.is_empty() {
                return Err("sse: missing url".to_owned());
            }
            Ok(McpServerConfig {
                transport: TransportType::Sse,
                command: None,
                args: None,
                env: None,
                url: Some(url.clone()),
                headers: Some(headers.clone()),
                deferred: Some(false),
            })
        }
        SessionMcpTransport::StreamableHttp { url, headers } => {
            if url.is_empty() {
                return Err("streamable_http: missing url".to_owned());
            }
            Ok(McpServerConfig {
                transport: TransportType::StreamableHttp,
                command: None,
                args: None,
                env: None,
                url: Some(url.clone()),
                headers: Some(headers.clone()),
                deferred: Some(false),
            })
        }
    }
}

fn merge_session_snapshot_mcp_servers(
    extra_mcp_servers: &mut HashMap<String, McpServerConfig>,
    session_mcp_servers: &[SessionMcpServer],
    conversation_id: &str,
) {
    for server in session_mcp_servers {
        match session_server_to_mcp_server_config(server) {
            Ok(config) => {
                if extra_mcp_servers
                    .insert(server.name.clone(), config)
                    .is_some()
                {
                    debug!(
                        conversation_id = %conversation_id,
                        server_name = %server.name,
                        "session_mcp: session snapshot overrides repo-backed MCP config"
                    );
                }
            }
            Err(err) => {
                warn!(
                    conversation_id = %conversation_id,
                    mcp_server_id = %server.mcp_server_id,
                    server_name = %server.name,
                    error = %err,
                    "session_mcp: failed to convert session snapshot; skipping"
                );
            }
        }
    }
}

fn resolve_stdio_command(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return command.to_owned();
    }

    let path = std::path::Path::new(trimmed);
    if path.is_absolute()
        || trimmed.contains(std::path::MAIN_SEPARATOR)
        || trimmed.contains('/')
        || trimmed.contains('\\')
    {
        return trimmed.to_owned();
    }

    resolve_command_path(trimmed)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.to_owned())
}

fn resolve_mcp_servers(
    overrides: &NomiBuildExtra,
    conversation_id: &str,
) -> (HashMap<String, McpServerConfig>, LoopbackCapabilityLeaseSet) {
    let mut servers = HashMap::new();
    let mut leases = LoopbackCapabilityLeaseSet::new();
    // Presence of the process-owned config is the capability grant.
    if let Some(gw_cfg) = &overrides.gateway_mcp_config {
        if let Some((name, server, lease)) =
            gateway_mcp_to_config(gw_cfg, overrides, conversation_id)
        {
            servers.insert(name, server);
            leases.push(lease);
        }
    }
    (servers, leases)
}

fn resolved_session_mode(overrides: &NomiBuildExtra) -> String {
    overrides
        .session_mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("yolo")
        .to_owned()
}

/// Platform Gateway MCP stdio bridge config for the Nomi engine, mirroring the
/// ACP assembler's `gateway_mcp_server`. Caller conversation + user ids ride
/// along for self-protection and data scoping; the companion binding (when present)
/// rides along for attribution.
fn gateway_mcp_to_config(
    cfg: &GatewayMcpConfig,
    overrides: &NomiBuildExtra,
    conversation_id: &str,
) -> Option<(String, McpServerConfig, LoopbackCapabilityLease)> {
    let session_mode = resolved_session_mode(overrides);
    let Some(user_id) = overrides.user_id.as_deref() else {
        warn!(conversation_id, "gateway MCP capability issuance requires a user ID");
        return None;
    };
    let child = match cfg.issue_for_conversation(
        user_id,
        conversation_id,
        overrides.companion_id.as_deref(),
        overrides.channel_platform.as_deref(),
        Some(&session_mode),
        &overrides.gateway_excluded_tools,
    ) {
        Ok(child) => child,
        Err(error) => {
            warn!(%error, conversation_id, "gateway MCP capability issuance failed closed");
            return None;
        }
    };
    let mut env = HashMap::new();
    env.insert(
        GatewayMcpConfig::ENV_CAPABILITY.into(),
        child
            .bootstrap_json()
            .expect("validated gateway bootstrap serializes"),
    );

    let server = McpServerConfig {
        transport: TransportType::Stdio,
        command: Some(child.binary_path),
        args: Some(vec!["mcp-gateway-stdio".into()]),
        env: Some(env),
        url: None,
        headers: None,
        deferred: Some(true),
    };

    Some((
        GatewayMcpConfig::SERVER_NAME.to_owned(),
        server,
        child.lease,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_pref_parse_matches_the_boot_time_reader_semantics() {
        // Fail-open default-ON keys (persistentLogin): only an explicit
        // "false" (bare or JSON-quoted) turns them off; junk keeps the default.
        for on in ["true", "\"true\"", "yes", "\"yes\"", "", "junk"] {
            assert!(parse_bool_pref(on, true), "{on:?} must keep a default-ON toggle on");
        }
        assert!(!parse_bool_pref("false", true));
        assert!(!parse_bool_pref("\"false\"", true));
        assert!(!parse_bool_pref("  \"false\"  ", true));

        // Fail-closed default-OFF keys (fullPower): only an explicit "true"
        // (bare or JSON-quoted) turns them on; junk keeps them off.
        for off in ["false", "\"false\"", "yes", "\"yes\"", "", "junk"] {
            assert!(!parse_bool_pref(off, false), "{off:?} must keep a default-OFF toggle off");
        }
        assert!(parse_bool_pref("true", false));
        assert!(parse_bool_pref("\"true\"", false));
        assert!(parse_bool_pref("  \"true\"  ", false));
    }

    fn gateway_config(port: u16, binary: &str, owner: &str) -> GatewayMcpConfig {
        GatewayMcpConfig::from_issuer(
            port,
            Arc::new(nomifun_common::LoopbackCapabilityIssuer::random().unwrap()),
            binary.into(),
            Arc::<str>::from(owner),
        )
    }

    #[test]
    fn secondary_nomi_session_is_model_only() {
        let mcp_server_id = McpServerId::new();
        let mut overrides = NomiBuildExtra {
            computer_use: Some(true),
            browser_use: Some(true),
            mcp_server_ids: Some(vec![mcp_server_id.clone()]),
            session_mcp_servers: vec![SessionMcpServer {
                mcp_server_id,
                name: "test-mcp".into(),
                transport: SessionMcpTransport::Stdio {
                    command: "server".into(),
                    args: Vec::new(),
                    env: Default::default(),
                },
            }],
            companion: true,
            companion_id: Some("0190f5fe-7c00-7a00-8abc-012345678967".into()),
            knowledge_mounts: vec![nomifun_api_types::KnowledgeMountInfo {
                knowledge_base_id: nomifun_common::KnowledgeBaseId::new(),
                name: "test knowledge".into(),
                description: "test mount removed by model-only ceiling".into(),
                rel_path: ".nomi/knowledge/test".into(),
                toc: Vec::new(),
                summary: None,
                live_sources: Vec::new(),
            }],
            knowledge_writeback: true,
            knowledge_channel_write_enabled: true,
            summon: Some(nomifun_api_types::SummonConfig {
                companion_id: "0190f5fe-7c00-7a00-8abc-012345678969".into(),
                memory_ids: vec![],
                skill_exclusions: vec![],
                summoned_at: 1,
            }),
            ..Default::default()
        };

        apply_model_only_ceiling(&mut overrides);

        assert!(overrides.gateway_mcp_config.is_none());
        assert_eq!(overrides.computer_use, Some(false));
        assert_eq!(overrides.browser_use, Some(false));
        assert!(overrides.mcp_server_ids.is_none());
        assert!(overrides.session_mcp_servers.is_empty());
        assert!(!overrides.companion && overrides.companion_id.is_none());
        assert!(overrides.knowledge_mounts.is_empty());
        assert!(!overrides.knowledge_writeback);
        assert!(!overrides.knowledge_channel_write_enabled);
        assert!(
            overrides.summon.is_none(),
            "summon loads local companion memories/skills — owner only"
        );
        assert_eq!(overrides.allowed_tools, vec!["update_plan"]);
        assert_eq!(overrides.session_mode.as_deref(), Some("default"));
        assert_eq!(overrides.max_turns, Some(1));
        assert_eq!(overrides.delegation_policy, DelegationPolicy::Disabled);
    }

    #[test]
    fn resumed_session_metadata_tracks_each_provider_switch() {
        let now = chrono::Utc::now();
        let mut session = Session {
            id: "provider-switch".into(),
            created_at: now,
            updated_at: now,
            provider: "provider-a".into(),
            model: "model-a".into(),
            cwd: "/workspace".into(),
            total_usage: Default::default(),
            messages: Vec::new(),
            owner_token: None,
            activated_deferred_tools: Vec::new(),
            editable_turn: None,
        };

        assert!(retarget_resumed_session(
            &mut session,
            "provider-b",
            "model-b"
        ));
        assert_eq!(session.provider, "provider-b");
        assert_eq!(session.model, "model-b");
        assert!(retarget_resumed_session(
            &mut session,
            "provider-a",
            "model-a2"
        ));
        assert_eq!(session.provider, "provider-a");
        assert_eq!(session.model, "model-a2");
    }

    #[test]
    fn session_sanitizer_remaps_the_editable_turn_boundary() {
        use nomi_agent::session::EditableTurnCheckpoint;
        use nomi_types::message::{ContentBlock, Message, Role};

        let now = chrono::Utc::now();
        let mut session = Session {
            id: "rewind-boundary".into(),
            created_at: now,
            updated_at: now,
            provider: "provider-a".into(),
            model: "model-a".into(),
            cwd: "/workspace".into(),
            total_usage: Default::default(),
            messages: vec![
                Message::new(
                    Role::User,
                    vec![ContentBlock::Text {
                        text: "stable history".into(),
                    }],
                ),
                Message::new(
                    Role::Assistant,
                    vec![ContentBlock::Text {
                        text: String::new(),
                    }],
                ),
                Message::new(
                    Role::User,
                    vec![ContentBlock::Text {
                        text: "editable root".into(),
                    }],
                ),
            ],
            owner_token: None,
            activated_deferred_tools: Vec::new(),
            editable_turn: Some(EditableTurnCheckpoint {
                source_message_id: "message-root".into(),
                start_len: 2,
            }),
        };

        let repair = sanitize_resumed_session(&mut session, false);

        assert_eq!(repair.removed_messages, 1);
        assert_eq!(session.messages.len(), 2);
        assert_eq!(
            session
                .editable_turn
                .as_ref()
                .map(|checkpoint| checkpoint.start_len),
            Some(1)
        );
    }

    #[test]
    fn resolved_fallback_metadata_is_persisted_to_session_and_index() {
        let directory = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(directory.path().to_path_buf(), 10);
        let mut session = manager
            .create("deleted-provider", "stale-model", "/workspace", Some("fallback"))
            .unwrap();

        retarget_resumed_session(&mut session, "resolved-provider", "resolved-fallback-model");
        persist_repaired_session(&manager, &session).unwrap();

        let reloaded = manager.load("fallback").unwrap();
        assert_eq!(reloaded.provider, "resolved-provider");
        assert_eq!(reloaded.model, "resolved-fallback-model");
        let metadata = manager
            .list()
            .unwrap()
            .into_iter()
            .find(|entry| entry.id == "fallback")
            .unwrap();
        assert_eq!(metadata.model, "resolved-fallback-model");
    }

    // ----- output-language directive (thinking + reply follow system language) -----

    /// Minimal mock settings repo for `read_app_language`: yields a fixed result
    /// (`Err(())` simulates a DB read failure). Mirrors the McpServerRepo mock in
    /// factory/acp.rs.
    struct MockSettingsRepo(Result<Option<nomifun_db::models::SystemSettings>, ()>);

    #[async_trait::async_trait]
    impl ISettingsRepository for MockSettingsRepo {
        async fn get_settings(
            &self,
        ) -> Result<Option<nomifun_db::models::SystemSettings>, nomifun_db::DbError> {
            self.0
                .clone()
                .map_err(|_| nomifun_db::DbError::Init("simulated".into()))
        }
        async fn upsert_settings(
            &self,
            _language: &str,
            _notification_enabled: bool,
            _cron_notification_enabled: bool,
            _command_queue_enabled: bool,
            _save_upload_to_workspace: bool,
        ) -> Result<nomifun_db::models::SystemSettings, nomifun_db::DbError> {
            unimplemented!("not exercised by the language tests")
        }
    }

    fn settings_row(language: &str) -> nomifun_db::models::SystemSettings {
        nomifun_db::models::SystemSettings {
            id: 1,
            singleton_key: "system".to_owned(),
            language: language.to_owned(),
            notification_enabled: true,
            cron_notification_enabled: false,
            command_queue_enabled: false,
            save_upload_to_workspace: false,
            updated_at: 0,
        }
    }

    fn settings_repo(
        result: Result<Option<nomifun_db::models::SystemSettings>, ()>,
    ) -> Arc<dyn ISettingsRepository> {
        Arc::new(MockSettingsRepo(result))
    }

    #[test]
    fn output_language_directive_maps_supported_and_defaults_to_english() {
        // zh-CN steers BOTH reply and thinking to Simplified Chinese.
        let zh = output_language_directive("zh-CN");
        assert!(zh.contains("简体中文"));
        assert!(zh.contains("思考"), "zh directive must cover the thinking process: {zh}");
        // en-US, unknown codes, and the empty string all resolve to English.
        for lang in ["en-US", "fr-FR", "zh-TW", ""] {
            let d = output_language_directive(lang);
            assert!(
                d.contains("in English"),
                "{lang} should map to English: {d}"
            );
            assert!(
                d.contains("think"),
                "{lang} directive must cover the thinking process: {d}"
            );
            assert!(!d.contains("简体中文"), "{lang} must not select Chinese");
        }
    }

    #[test]
    fn resolve_language_prefers_persisted_then_os_then_default() {
        // Persisted setting always wins (ignores OS locale).
        assert_eq!(resolve_language(Some("en-US"), Some("zh-CN")), "en-US");
        assert_eq!(resolve_language(Some("zh-CN"), Some("en-US")), "zh-CN");
        // No persisted value → follow the OS locale (首轮跟随系统语言).
        assert_eq!(resolve_language(None, Some("zh-CN")), "zh-CN");
        assert_eq!(resolve_language(Some("  "), Some("zh_CN")), "zh-CN");
        assert_eq!(resolve_language(None, Some("en-US")), "en-US");
        // Neither → hard default.
        assert_eq!(resolve_language(None, None), "en-US");
        assert_eq!(resolve_language(Some(""), Some("   ")), "en-US");
    }

    #[test]
    fn normalize_lang_folds_every_chinese_locale_to_zh_cn() {
        for zh in ["zh", "zh-CN", "zh_CN", "zh-Hans", "zh-Hans-CN", "ZH-cn"] {
            assert_eq!(normalize_lang(zh), "zh-CN", "{zh} must fold to zh-CN");
        }
        // Non-Chinese tags are returned normalized (→ English directive).
        assert_eq!(normalize_lang("en_US"), "en-US");
        assert_eq!(normalize_lang("fr-FR"), "fr-FR");
    }

    #[tokio::test]
    async fn read_app_language_returns_persisted_language() {
        // A persisted value wins over whatever OS locale the test host reports.
        assert_eq!(
            read_app_language(Some(&settings_repo(Ok(Some(settings_row("zh-CN")))))).await,
            "zh-CN"
        );
        assert_eq!(
            read_app_language(Some(&settings_repo(Ok(Some(settings_row("en-US")))))).await,
            "en-US"
        );
    }

    #[test]
    fn resolve_mcp_servers_adds_gateway_when_process_config_present() {
        let overrides = NomiBuildExtra {
            gateway_mcp_config: Some(gateway_config(41237, "/usr/bin/nomicore", "owner")),
            user_id: Some("0190f5fe-7c00-7a00-8abc-012345678961".into()),
            companion_id: Some("0190f5fe-7c00-7a00-8abc-012345678965".into()),
            gateway_excluded_tools: vec!["nomi_delegate".into()],
            ..Default::default()
        };
        let (servers, leases) = resolve_mcp_servers(&overrides, "0190f5fe-7c00-7a00-8abc-012345678963");
        assert_eq!(leases.len(), 1);
        let gw = servers
            .get(GatewayMcpConfig::SERVER_NAME)
            .expect("gateway server registered");
        assert_eq!(
            gw.args.as_deref(),
            Some(&["mcp-gateway-stdio".to_owned()][..])
        );
        let env = gw.env.as_ref().expect("env set");
        assert_eq!(env.len(), 1);
        let bootstrap: nomifun_api_types::ScopedMcpChildBootstrap<
            nomifun_api_types::GatewayCapabilityClaims,
        > = serde_json::from_str(
            env.get(GatewayMcpConfig::ENV_CAPABILITY)
                .expect("capability bootstrap env"),
        )
        .unwrap();
        assert_eq!(bootstrap.port, 41237);
        let claims = bootstrap.access.claims;
        assert_eq!(
            claims.user_id.as_str(),
            "0190f5fe-7c00-7a00-8abc-012345678961"
        );
        assert_eq!(claims.session.session_id, "0190f5fe-7c00-7a00-8abc-012345678963");
        assert_eq!(claims.session.conversation_id.as_deref(), Some("0190f5fe-7c00-7a00-8abc-012345678963"));
        assert_eq!(claims.scope.companion_id.as_deref(), Some("0190f5fe-7c00-7a00-8abc-012345678965"));
        assert_eq!(claims.scope.profile, GatewayMcpConfig::PROFILE_WORK);
        assert_eq!(claims.scope.session_mode.as_deref(), Some("yolo"));
        assert_eq!(claims.scope.excluded_tools, vec!["nomi_delegate"]);
        assert!(!claims.scope.instance_owner);
        assert!(!env[GatewayMcpConfig::ENV_CAPABILITY].contains("gw-root-secret"));
        assert_eq!(gw.deferred, Some(true));
    }

    #[test]
    fn gateway_env_omits_companion_id_when_unbound() {
        let overrides = NomiBuildExtra {
            gateway_mcp_config: Some(gateway_config(41237, "/usr/bin/nomicore", "owner")),
            user_id: Some("0190f5fe-7c00-7a00-8abc-012345678961".into()),
            companion_id: None,
            ..Default::default()
        };
        let (servers, _leases) = resolve_mcp_servers(&overrides, "0190f5fe-7c00-7a00-8abc-012345678963");
        let env = servers[GatewayMcpConfig::SERVER_NAME].env.as_ref().unwrap();
        let bootstrap: nomifun_api_types::ScopedMcpChildBootstrap<
            nomifun_api_types::GatewayCapabilityClaims,
        > = serde_json::from_str(env.get(GatewayMcpConfig::ENV_CAPABILITY).unwrap()).unwrap();
        let claims = bootstrap.access.claims;
        assert!(claims.scope.companion_id.is_none());
    }

    #[test]
    fn gateway_env_uses_lite_profile_for_channel_sessions() {
        let overrides = NomiBuildExtra {
            gateway_mcp_config: Some(gateway_config(41237, "/usr/bin/nomicore", "owner")),
            user_id: Some("0190f5fe-7c00-7a00-8abc-012345678961".into()),
            channel_platform: Some("lark".into()),
            ..Default::default()
        };
        let (servers, _leases) = resolve_mcp_servers(&overrides, "0190f5fe-7c00-7a00-8abc-012345678963");
        let env = servers[GatewayMcpConfig::SERVER_NAME].env.as_ref().unwrap();
        let bootstrap: nomifun_api_types::ScopedMcpChildBootstrap<
            nomifun_api_types::GatewayCapabilityClaims,
        > = serde_json::from_str(env.get(GatewayMcpConfig::ENV_CAPABILITY).unwrap()).unwrap();
        let claims = bootstrap.access.claims;
        assert_eq!(claims.scope.profile, GatewayMcpConfig::PROFILE_LITE);
    }

    #[test]
    fn resolve_mcp_servers_skips_gateway_without_process_config() {
        let overrides = NomiBuildExtra::default();
        let (servers, leases) = resolve_mcp_servers(&overrides, "0190f5fe-7c00-7a00-8abc-012345678963");
        assert!(!servers.contains_key(GatewayMcpConfig::SERVER_NAME));
        assert!(leases.is_empty());
    }

    #[test]
    fn normalize_nomi_base_url_strips_v1() {
        assert_eq!(
            normalize_nomi_base_url("https://api.openai.com/v1"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_nomi_base_url("https://api.openai.com/v1/"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_nomi_base_url("https://api.anthropic.com"),
            "https://api.anthropic.com"
        );
        assert_eq!(
            normalize_nomi_base_url("https://api.deepseek.com/"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            normalize_nomi_base_url("http://localhost:11434"),
            "http://localhost:11434"
        );
        assert_eq!(normalize_nomi_base_url(""), "");
    }

    #[test]
    fn map_nomi_provider_known_platforms() {
        assert_eq!(map_nomi_provider("anthropic", None), "anthropic");
        assert_eq!(map_nomi_provider("bedrock", None), "bedrock");
        assert_eq!(map_nomi_provider("gemini-vertex-ai", None), "vertex");
    }

    #[test]
    fn map_nomi_provider_custom_and_others_default_to_openai() {
        assert_eq!(map_nomi_provider("custom", None), "openai");
        assert_eq!(map_nomi_provider("gemini", None), "openai");
        assert_eq!(map_nomi_provider("new-api", None), "openai");
        assert_eq!(map_nomi_provider("unknown", None), "openai");
    }

    #[test]
    fn map_nomi_provider_new_api_with_anthropic_protocol() {
        assert_eq!(
            map_nomi_provider("new-api", Some("anthropic")),
            "anthropic"
        );
        assert_eq!(map_nomi_provider("new-api", Some("openai")), "openai");
        assert_eq!(map_nomi_provider("new-api", None), "openai");
    }

    #[test]
    fn map_nomi_provider_non_new_api_ignores_protocol_override() {
        assert_eq!(map_nomi_provider("custom", Some("anthropic")), "openai");
    }

    #[test]
    fn is_openai_host_detects_official_api() {
        assert!(is_openai_host("https://api.openai.com/v1"));
        assert!(is_openai_host("https://api.openai.com"));
        assert!(is_openai_host("https://API.OPENAI.COM/v1"));
        assert!(!is_openai_host("https://api.deepseek.com/v1"));
        assert!(!is_openai_host("https://openai.example.com/v1"));
        assert!(!is_openai_host(""));
        assert!(!is_openai_host("not-a-url"));
    }

    #[test]
    fn resolve_openai_official_sets_max_completion_tokens() {
        let (base_url, compat) =
            resolve_nomi_url_and_compat("custom", "https://api.openai.com/v1", "openai", false);
        assert_eq!(base_url.as_deref(), Some("https://api.openai.com"));
        assert_eq!(
            compat.max_tokens_field.as_deref(),
            Some("max_completion_tokens")
        );
        assert!(compat.api_path.is_none());
    }

    #[test]
    fn resolve_non_openai_keeps_default_max_tokens() {
        let (base_url, compat) =
            resolve_nomi_url_and_compat("custom", "https://api.deepseek.com/v1", "openai", false);
        assert_eq!(base_url.as_deref(), Some("https://api.deepseek.com"));
        assert!(compat.max_tokens_field.is_none());
    }

    #[test]
    fn resolve_gemini_prepends_path_and_sets_api_path() {
        let (base_url, compat) = resolve_nomi_url_and_compat(
            "gemini",
            "https://generativelanguage.googleapis.com",
            "openai",
            false,
        );
        assert_eq!(
            base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/openai")
        );
        assert_eq!(compat.api_path.as_deref(), Some("/chat/completions"));
        assert!(compat.max_tokens_field.is_none());
    }

    #[test]
    fn resolve_anthropic_no_compat_overrides() {
        let (base_url, compat) = resolve_nomi_url_and_compat(
            "anthropic",
            "https://api.anthropic.com",
            "anthropic",
            false,
        );
        assert_eq!(base_url.as_deref(), Some("https://api.anthropic.com"));
        assert!(compat.max_tokens_field.is_none());
        assert!(compat.api_path.is_none());
    }

    #[test]
    fn resolve_full_url_mode_uses_url_as_is() {
        let (base_url, compat) = resolve_nomi_url_and_compat(
            "custom",
            "https://proxy.example.com/v1/chat/completions",
            "openai",
            true,
        );
        assert_eq!(
            base_url.as_deref(),
            Some("https://proxy.example.com/v1/chat/completions")
        );
        assert_eq!(compat.api_path.as_deref(), Some(""));
        assert!(compat.max_tokens_field.is_none());
    }

    #[test]
    fn resolve_full_url_mode_strips_trailing_slash() {
        let (base_url, compat) = resolve_nomi_url_and_compat(
            "custom",
            "https://proxy.example.com/v1/chat/completions/",
            "openai",
            true,
        );
        assert_eq!(
            base_url.as_deref(),
            Some("https://proxy.example.com/v1/chat/completions")
        );
        assert_eq!(compat.api_path.as_deref(), Some(""));
    }

    #[test]
    fn resolve_full_url_false_still_normalizes() {
        let (base_url, compat) =
            resolve_nomi_url_and_compat("custom", "https://api.deepseek.com/v1", "openai", false);
        assert_eq!(base_url.as_deref(), Some("https://api.deepseek.com"));
        assert!(compat.api_path.is_none());
    }

    #[test]
    fn resolve_domestic_openai_compatible_platforms_use_configured_chat_base() {
        for (platform, base) in [
            ("ark", "https://ark.cn-beijing.volces.com/api/v3"),
            ("stepfun", "https://api.stepfun.com/v1"),
            ("zhipu", "https://open.bigmodel.cn/api/paas/v4"),
            ("qianfan", "https://qianfan.baidubce.com/v2"),
        ] {
            let (base_url, compat) = resolve_nomi_url_and_compat(platform, base, "openai", false);
            assert_eq!(base_url.as_deref(), Some(base), "platform={platform}");
            assert_eq!(
                compat.api_path.as_deref(),
                Some("/chat/completions"),
                "platform={platform}"
            );
        }
    }

    #[test]
    fn resolve_coding_plan_platforms_use_chat_completions_at_configured_base() {
        for (platform, base) in [
            (
                "ark-coding-plan",
                "https://ark.cn-beijing.volces.com/api/coding/v3",
            ),
            (
                "ark-agent-plan",
                "https://ark.cn-beijing.volces.com/api/plan/v3",
            ),
            ("stepfun-plan", "https://api.stepfun.com/step_plan/v1"),
            (
                "dashscope-coding",
                "https://coding.dashscope.aliyuncs.com/v1",
            ),
            (
                "glm-coding-plan",
                "https://open.bigmodel.cn/api/coding/paas/v4",
            ),
            (
                "qianfan-coding-plan",
                "https://qianfan.baidubce.com/v2/coding",
            ),
        ] {
            let (base_url, compat) = resolve_nomi_url_and_compat(platform, base, "openai", false);
            assert_eq!(base_url.as_deref(), Some(base), "platform={platform}");
            assert_eq!(
                compat.api_path.as_deref(),
                Some("/chat/completions"),
                "platform={platform}"
            );
        }
    }

    #[test]
    fn resolve_mcp_servers_empty_when_no_config() {
        let overrides = NomiBuildExtra::default();
        let (result, leases) = resolve_mcp_servers(&overrides, "conv-3");
        assert!(result.is_empty());
        assert!(leases.is_empty());
    }

    #[test]
    fn session_snapshot_overrides_repo_backed_mcp_config() {
        let mut servers = HashMap::from([(
            "demo-mcp".to_owned(),
            McpServerConfig {
                transport: TransportType::Stdio,
                command: Some("npx".into()),
                args: Some(vec!["-y".into(), "@old/server".into()]),
                env: Some(HashMap::new()),
                url: None,
                headers: None,
                deferred: Some(false),
            },
        )]);

        let snapshot = vec![SessionMcpServer {
            mcp_server_id: McpServerId::new(),
            name: "demo-mcp".into(),
            transport: SessionMcpTransport::Stdio {
                command: "uvx".into(),
                args: vec!["new-server".into()],
                env: HashMap::from([("TOKEN".into(), "abc".into())]),
            },
        }];

        merge_session_snapshot_mcp_servers(&mut servers, &snapshot, "conv-override");

        let server = servers.get("demo-mcp").expect("snapshot should remain");
        assert_eq!(server.transport, TransportType::Stdio);
        // `resolve_command_path` may resolve to an absolute path; on Windows
        // that includes the `.exe` extension.
        let command = server
            .command
            .as_deref()
            .expect("stdio command should exist");
        let command = command.replace('\\', "/").to_lowercase();
        assert!(
            command == "uvx" || command.ends_with("/uvx") || command.ends_with("/uvx.exe"),
            "unexpected stdio command path: {command}",
        );
        assert_eq!(server.args.as_deref(), Some(&["new-server".to_owned()][..]));
        assert_eq!(
            server.env.as_ref().and_then(|env| env.get("TOKEN")),
            Some(&"abc".to_owned())
        );
    }

    #[test]
    fn resolve_bedrock_config_access_key() {
        let json = r#"{"auth_method":"accessKey","region":"us-west-2","access_key_id":"AKIA123","secret_access_key":"secret456"}"#;
        let result = resolve_bedrock_config(Some(json)).unwrap();
        assert_eq!(result.region.as_deref(), Some("us-west-2"));
        assert_eq!(result.access_key_id.as_deref(), Some("AKIA123"));
        assert_eq!(result.secret_access_key.as_deref(), Some("secret456"));
        assert!(result.profile.is_none());
        assert!(result.session_token.is_none());
    }

    #[test]
    fn resolve_bedrock_config_profile() {
        let json = r#"{"auth_method":"profile","region":"eu-west-1","profile":"my-profile"}"#;
        let result = resolve_bedrock_config(Some(json)).unwrap();
        assert_eq!(result.region.as_deref(), Some("eu-west-1"));
        assert_eq!(result.profile.as_deref(), Some("my-profile"));
        assert!(result.access_key_id.is_none());
        assert!(result.secret_access_key.is_none());
    }

    #[test]
    fn resolve_bedrock_config_none_when_json_missing() {
        assert!(resolve_bedrock_config(None).is_none());
    }

    #[test]
    fn resolve_bedrock_config_none_when_json_invalid() {
        assert!(resolve_bedrock_config(Some("not-json")).is_none());
    }

    #[test]
    fn preset_rules_merged_into_system_prompt_when_no_existing() {
        let json = serde_json::json!({
            "preset_rules": "You are a data analyst. Always use Python.",
        });
        let mut overrides: NomiBuildExtra = serde_json::from_value(json).unwrap();

        if let Some(rules) = overrides.preset_rules.take() {
            overrides.system_prompt = Some(match overrides.system_prompt.take() {
                Some(existing) => format!("{existing}\n\n{rules}"),
                None => rules,
            });
        }

        assert_eq!(
            overrides.system_prompt.as_deref(),
            Some("You are a data analyst. Always use Python.")
        );
        assert!(overrides.preset_rules.is_none());
    }

    #[test]
    fn preset_rules_appended_to_existing_system_prompt() {
        let json = serde_json::json!({
            "system_prompt": "Be concise.",
            "preset_rules": "You are a data analyst.",
        });
        let mut overrides: NomiBuildExtra = serde_json::from_value(json).unwrap();

        if let Some(rules) = overrides.preset_rules.take() {
            overrides.system_prompt = Some(match overrides.system_prompt.take() {
                Some(existing) => format!("{existing}\n\n{rules}"),
                None => rules,
            });
        }

        assert_eq!(
            overrides.system_prompt.as_deref(),
            Some("Be concise.\n\nYou are a data analyst.")
        );
    }

    #[test]
    fn no_preset_rules_leaves_system_prompt_unchanged() {
        let json = serde_json::json!({
            "system_prompt": "Be concise.",
        });
        let mut overrides: NomiBuildExtra = serde_json::from_value(json).unwrap();

        if let Some(rules) = overrides.preset_rules.take() {
            overrides.system_prompt = Some(match overrides.system_prompt.take() {
                Some(existing) => format!("{existing}\n\n{rules}"),
                None => rules,
            });
        }

        assert_eq!(overrides.system_prompt.as_deref(), Some("Be concise."));
    }

    #[test]
    fn embedded_agent_execution_requires_trusted_no_gateway_host() {
        assert!(should_install_embedded_agent_execution(false, true));
        assert!(!should_install_embedded_agent_execution(true, true));
        assert!(!should_install_embedded_agent_execution(false, false));
    }

    #[test]
    fn automatic_delegation_hint_injects_for_plain_desktop_session() {
        assert!(super::should_inject_delegation_hint(true, false, false));
        let out = super::compose_delegation_hint(
            Some("基础提示".to_string()),
            true,
            DelegationPolicy::Automatic,
        );
        let s = out.unwrap();
        assert!(s.starts_with("基础提示"));
        assert!(s.contains("nomi_delegate"));
        assert!(s.contains("strategy=parallel"));
        assert!(s.contains("strategy=planned"));
        assert!(s.contains("nomi_execution_get"));
        assert!(!s.contains(super::DELEGATION_PREFER_PARALLEL_HINT));
    }

    #[test]
    fn delegation_hint_skips_when_gateway_absent() {
        assert!(!super::should_inject_delegation_hint(false, false, false));
    }

    #[test]
    fn delegation_hint_skips_restricted_surfaces() {
        assert!(!super::should_inject_delegation_hint(true, true, false));
        assert!(!super::should_inject_delegation_hint(true, false, true));
        let base = Some("仅基础".to_string());
        assert_eq!(
            super::compose_delegation_hint(base.clone(), false, DelegationPolicy::Automatic),
            base
        );
    }

    #[test]
    fn automatic_delegation_hint_handles_empty_base() {
        let out = super::compose_delegation_hint(None, true, DelegationPolicy::Automatic);
        assert_eq!(out, Some(super::DELEGATION_STANDARD_HINT.to_string()));
    }

    #[test]
    fn prefer_parallel_hint_appends_after_standard_hint() {
        let out = super::compose_delegation_hint(
            Some("基础提示".to_string()),
            true,
            DelegationPolicy::PreferParallel,
        )
        .unwrap();
        assert!(out.starts_with("基础提示"));
        let standard_pos = out.find(super::DELEGATION_STANDARD_HINT).expect("标准提示在场");
        let preference_pos = out
            .find(super::DELEGATION_PREFER_PARALLEL_HINT)
            .expect("并行偏好提示在场");
        assert!(standard_pos < preference_pos);
        assert!(out.contains("优先使用"));
    }

    #[test]
    fn disabled_delegation_policy_preserves_base() {
        let base = Some("仅基础".to_string());
        assert_eq!(
            super::compose_delegation_hint(base.clone(), true, DelegationPolicy::Disabled),
            base
        );
    }

    #[test]
    fn native_write_root_unrestricted_only_for_local_desktop() {
        // 本地桌面(无渠道)→ None(OS 用户全权,今日行为)。
        assert_eq!(resolve_native_write_root(None, "/ws"), None);
        assert_eq!(resolve_native_write_root(Some(""), "/ws"), None);
        // 渠道 → 收窄到工作区。
        assert_eq!(
            resolve_native_write_root(Some("lark"), "/ws"),
            Some("/ws".to_owned())
        );
        // 渠道但工作区为空 → 回退 None(无从钳制,不劣于今日)。
        assert_eq!(resolve_native_write_root(Some("lark"), "  "), None);
    }

    #[test]
    fn append_knowledge_context_without_mounts_is_passthrough() {
        let config = NomiBuildExtra::default();
        assert_eq!(
            append_knowledge_context(None, &config, "0190f5fe-7c00-7a00-8abc-012345678963", true),
            None
        );
        assert_eq!(
            append_knowledge_context(Some("hello".into()), &config, "0190f5fe-7c00-7a00-8abc-012345678963", true),
            Some("hello".into())
        );
    }

    #[test]
    fn append_knowledge_context_renders_mounts_and_writeback() {
        use nomifun_api_types::KnowledgeMountInfo;

        let conversation_id = "0190f5fe-7c00-7a00-8abc-012345678963";

        let mut config = NomiBuildExtra {
            knowledge_mounts: vec![KnowledgeMountInfo {
                knowledge_base_id: nomifun_common::KnowledgeBaseId::new(),
                name: "领域知识".into(),
                description: "domain docs".into(),
                rel_path: ".nomi/knowledge/领域知识".into(),
                toc: vec!["intro.md — 简介".into()],
                summary: Some("Covers deployment flows and runbooks.".into()),
                live_sources: vec![],
            }],
            knowledge_writeback: false,
            ..Default::default()
        };

        let readonly =
            append_knowledge_context(Some("base".into()), &config, conversation_id, true).unwrap();
        assert!(readonly.starts_with("base\n\n"));
        assert!(readonly.contains("## Knowledge bases"));
        assert!(readonly.contains("领域知识"));
        assert!(readonly.contains("intro.md — 简介"));
        assert!(readonly.contains("READ-ONLY"));
        // Hit-rate contract: retrieval protocol (once), per-base summary and
        // when-to-consult guidance — same shared builder as the ACP path.
        assert_eq!(readonly.matches("Retrieval protocol").count(), 1);
        assert!(readonly.contains("Covers deployment flows and runbooks."));
        assert!(readonly.contains("When to consult"));

        // nomi surface has the native tool → write-back contract points at it,
        // and the staged inbox path stays internal (not advertised to the model).
        config.knowledge_writeback = true;
        let staged = append_knowledge_context(None, &config, conversation_id, true).unwrap();
        assert!(staged.contains("STAGED mode"));
        assert!(staged.contains("knowledge_write"));
        assert!(
            !staged.contains(&format!("_inbox/{conversation_id}/")),
            "tool contract must not leak the inbox path: {staged}"
        );
        // Flag plumbs through: without the tool, the file-based prose returns.
        let staged_files = append_knowledge_context(None, &config, conversation_id, false).unwrap();
        assert!(staged_files.contains(&format!("_inbox/{conversation_id}/")));
        assert!(!staged_files.contains("knowledge_write"));

        config.knowledge_writeback_mode = Some("direct".into());
        let direct = append_knowledge_context(None, &config, conversation_id, true).unwrap();
        assert!(direct.contains("DIRECT mode"));
        assert!(direct.contains("knowledge_write"));
        assert!(!direct.contains("_inbox/"));
        // Disposition (回写意识) threads from build-extra → contract.
        assert!(direct.contains("Disposition — CONSERVATIVE"));
        config.knowledge_writeback_eagerness = Some("aggressive".into());
        let eager = append_knowledge_context(None, &config, conversation_id, true).unwrap();
        assert!(eager.contains("Disposition — AGGRESSIVE"));
    }

    #[test]
    fn knowledge_fields_deserialize_from_extra_and_reach_prompt() {
        // The conversation service writes snake_case keys into build-extra
        // JSON; the nomi build path must surface them in the system prompt.
        let json = serde_json::json!({
            "knowledge_mounts": [{
                "knowledge_base_id": "0190f5fe-7c00-7a00-8abc-012345678964",
                "name": "运维手册",
                "description": "",
                "rel_path": ".nomi/knowledge/运维手册",
                "toc": ["deploy.md — 部署"],
            }],
            "knowledge_writeback": true,
            "knowledge_writeback_mode": "staged",
            "knowledge_writeback_eagerness": "aggressive",
        });
        let overrides: NomiBuildExtra = serde_json::from_value(json).unwrap();
        assert_eq!(overrides.knowledge_mounts.len(), 1);
        assert!(overrides.knowledge_writeback);
        assert_eq!(
            overrides.knowledge_writeback_mode.as_deref(),
            Some("staged")
        );
        assert_eq!(
            overrides.knowledge_writeback_eagerness.as_deref(),
            Some("aggressive")
        );

        let prompt = append_knowledge_context(
            None,
            &overrides,
            "0190f5fe-7c00-7a00-8abc-012345678963",
            true,
        )
        .unwrap();
        assert!(prompt.contains("Knowledge bases"));
        assert!(prompt.contains("运维手册"));
        assert!(prompt.contains("knowledge_write"));
        // The disposition keyword threads all the way from extra JSON to prompt.
        assert!(prompt.contains("Disposition — AGGRESSIVE"));
        // Optional summary/live_sources may be absent while the canonical
        // knowledge-base identity contract remains strict.
        assert!(prompt.contains("When to consult"));
    }

    #[test]
    fn channel_write_opt_in_threads_from_extra_into_write_policy() {
        // Regression: the `channel_write_enabled` opt-in must survive the
        // build-extra round-trip so the nomi factory can resolve the
        // external-IM-channel write policy. Before the fix this field was never
        // threaded, so the reconstructed binding defaulted it to false and
        // channel write-back was permanently Disabled on the nomi engine.
        use nomifun_knowledge::{KnowledgeBinding, WriteMode, WriteSurface, resolve_write_policy};

        // Absent in JSON → serde default false (the previous, broken behavior).
        let off: NomiBuildExtra = serde_json::from_value(serde_json::json!({
            "knowledge_writeback": true,
        }))
        .unwrap();
        assert!(!off.knowledge_channel_write_enabled);

        // Present and true → carried through.
        let on: NomiBuildExtra = serde_json::from_value(serde_json::json!({
            "knowledge_writeback": true,
            "knowledge_channel_write_enabled": true,
        }))
        .unwrap();
        assert!(on.knowledge_channel_write_enabled);

        // Reconstruct the binding exactly as build_nomi does and confirm the
        // policy flips from Disabled to Staged for an external channel.
        let reconstruct = |extra: &NomiBuildExtra| KnowledgeBinding {
            enabled: true,
            writeback: extra.knowledge_writeback,
            writeback_mode: extra
                .knowledge_writeback_mode
                .clone()
                .unwrap_or_else(|| "staged".to_owned()),
            channel_write_enabled: extra.knowledge_channel_write_enabled,
            ..Default::default()
        };

        let disabled =
            resolve_write_policy(WriteSurface::ExternalChannel, &reconstruct(&off), "conv-c");
        assert!(matches!(disabled.mode, WriteMode::Disabled));

        let staged =
            resolve_write_policy(WriteSurface::ExternalChannel, &reconstruct(&on), "conv-c");
        assert!(matches!(staged.mode, WriteMode::Staged { .. }));
    }
}

/// P2 Task 6 behavior snapshot for the chat-path platform mapping.
///
/// Locks the EXACT `(provider, base_url, api_path, compat)` outputs of
/// `map_nomi_provider` + `resolve_nomi_url_and_compat` over the full platform
/// matrix — every `MODEL_PLATFORMS` entry from
/// `ui/src/renderer/utils/model/modelPlatforms.ts` (kept per-entry, so custom
/// presets with distinct base URLs each get a row) × representative base_url
/// variants (configured / trailing slash / toggled `/v1` / empty / full-URL),
/// plus new-api per-model protocol-override edge cases.
///
/// `SNAPSHOT` was generated by CALLING the pre-refactor implementation
/// (2026-07-29, commit eff19c8f working tree). It must stay byte-identical —
/// UNCHANGED — through the `platform_table` refactor; any diff means the
/// chat-path behavior regressed.
#[cfg(test)]
mod platform_chat_snapshot {
    use super::{map_nomi_provider, resolve_nomi_url_and_compat};

    /// Every `MODEL_PLATFORMS` entry as `(platform key, configured base_url)`,
    /// in file order. Entries without a preset base_url (Custom / New API /
    /// Vertex / Bedrock) use a representative or empty base. Two extra rows:
    /// the managed free-model platform and an unknown platform (default row).
    const PLATFORM_MATRIX: &[(&str, &str)] = &[
        ("custom", "https://api.example.com/v1"), // Custom (user-supplied base)
        ("new-api", "https://gateway.example.com/v1"), // New API gateway
        ("gemini", "https://generativelanguage.googleapis.com"),
        ("gemini-vertex-ai", ""),
        ("custom", "https://api.openai.com/v1"), // OpenAI preset
        ("anthropic", "https://api.anthropic.com"),
        ("bedrock", ""),
        ("deepseek", "https://api.deepseek.com/v1"),
        ("mimo", "https://api.xiaomimimo.com/v1"),
        ("mimo-token-plan-cn", "https://token-plan-cn.xiaomimimo.com/v1"),
        ("mimo-token-plan-sgp", "https://token-plan-sgp.xiaomimimo.com/v1"),
        ("mimo-token-plan-ams", "https://token-plan-ams.xiaomimimo.com/v1"),
        ("minimax", "https://api.minimaxi.com/v1"),
        ("minimax-code", "https://api.minimax.io/v1"),
        ("minimax-coding-plan", "https://api.minimaxi.com/v1"),
        ("custom", "https://api.novita.ai/openai/v1"), // Novita
        ("custom", "https://openrouter.ai/api/v1"),    // OpenRouter
        ("dashscope", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
        ("dashscope-coding", "https://coding.dashscope.aliyuncs.com/v1"),
        ("siliconflow", "https://api.siliconflow.cn/v1"), // SiliconFlow-CN
        ("siliconflow", "https://api.siliconflow.com/v1"), // SiliconFlow
        ("zhipu", "https://open.bigmodel.cn/api/paas/v4"),
        ("glm-coding-plan", "https://open.bigmodel.cn/api/coding/paas/v4"),
        ("moonshot-cn", "https://api.moonshot.cn/v1"),
        ("moonshot-global", "https://api.moonshot.ai/v1"),
        ("custom", "https://api.x.ai/v1"), // xAI
        ("ark", "https://ark.cn-beijing.volces.com/api/v3"),
        ("ark-coding-plan", "https://ark.cn-beijing.volces.com/api/coding/v3"),
        ("ark-agent-plan", "https://ark.cn-beijing.volces.com/api/plan/v3"),
        ("qianfan", "https://qianfan.baidubce.com/v2"),
        ("qianfan-coding-plan", "https://qianfan.baidubce.com/v2/coding"),
        ("hunyuan", "https://api.hunyuan.cloud.tencent.com/v1"),
        ("lingyi", "https://api.lingyiwanwu.com/v1"),
        ("custom", "https://api.poe.com/v1"),            // Poe
        ("custom", "https://api.ppinfra.com/v3/openai"), // PPIO
        ("custom", "https://api-inference.modelscope.cn/v1"), // ModelScope
        ("custom", "https://cloud.infini-ai.com/maas/v1"), // InfiniAI
        ("custom", "https://wishub-x1.ctyun.cn/v1"),     // Ctyun
        ("stepfun", "https://api.stepfun.com/v1"),
        ("stepfun-plan", "https://api.stepfun.com/step_plan/v1"),
        ("nomifun-free-model", "https://free.nomifun.example/v1"), // managed free model
        ("totally-unknown", "https://api.example.org/v1"), // default row
    ];

    /// `(platform, base_url, is_full_url, protocol)` extras: the new-api
    /// per-model protocol override, its interaction with the api.openai.com
    /// host rule, and full-URL edge cases (empty base; full-URL beating the
    /// gemini / domestic-whitelist platform rules).
    const EXTRA_CASES: &[(&str, &str, bool, Option<&str>)] = &[
        ("new-api", "https://gateway.example.com/v1", false, Some("anthropic")),
        ("new-api", "https://gateway.example.com/v1", false, Some("openai")),
        ("new-api", "https://gateway.example.com/v1", false, Some("gemini")),
        ("custom", "https://api.example.com/v1", false, Some("anthropic")),
        ("anthropic", "https://api.anthropic.com", false, Some("openai")),
        ("new-api", "https://api.openai.com/v1", false, None),
        ("new-api", "https://api.openai.com/v1", false, Some("anthropic")),
        ("custom", "", true, None),
        ("gemini", "https://proxy.example.com/gemini/chat", true, None),
        ("ark", "https://proxy.example.com/ark/chat", true, None),
    ];

    /// Base-url variants per platform row: configured / trailing slash /
    /// toggled `/v1` (stripped when present, appended when absent) / empty /
    /// full-URL (`is_full_url = true`).
    fn variants(base: &str) -> Vec<(String, bool)> {
        let toggled_v1 = match base.strip_suffix("/v1") {
            Some(stripped) => stripped.to_owned(),
            None => format!("{base}/v1"),
        };
        vec![
            (base.to_owned(), false),
            (format!("{base}/"), false),
            (toggled_v1, false),
            (String::new(), false),
            (format!("{base}/chat/completions"), true),
        ]
    }

    /// Mirrors the production call sequence (`provider_config.rs` /
    /// `provider_health.rs`): map the platform first, then resolve URL/compat
    /// with the MAPPED provider.
    fn render_case(platform: &str, base: &str, full: bool, proto: Option<&str>) -> String {
        let provider = map_nomi_provider(platform, proto);
        let (base_url, compat) = resolve_nomi_url_and_compat(platform, base, &provider, full);
        format!(
            "{platform} | in={base:?} | full={full} | proto={proto:?} => provider={provider} \
             | base={base_url:?} | api_path={:?} | max_tokens={:?} | image={:?} | reasoning={:?}",
            compat.api_path,
            compat.max_tokens_field,
            compat.supports_image,
            compat.require_reasoning_content,
        )
    }

    fn render_all() -> String {
        let mut out = String::new();
        for (platform, base) in PLATFORM_MATRIX {
            for (variant, full) in variants(base) {
                out.push_str(&render_case(platform, &variant, full, None));
                out.push('\n');
            }
        }
        for (platform, base, full, proto) in EXTRA_CASES {
            out.push_str(&render_case(platform, base, *full, *proto));
            out.push('\n');
        }
        out
    }

    #[test]
    fn platform_chat_rules_snapshot_locked() {
        let actual = render_all();
        if actual != SNAPSHOT {
            println!("=== ACTUAL SNAPSHOT BEGIN ===");
            print!("{actual}");
            println!("=== ACTUAL SNAPSHOT END ===");
            panic!(
                "platform chat snapshot changed — chat-path (provider, base_url, api_path, \
                 compat) must stay byte-identical to the pre-table behavior"
            );
        }
    }

    #[rustfmt::skip]
    const SNAPSHOT: &str = r#"custom | in="https://api.example.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.example.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.example.com" | full=false | proto=None => provider=openai | base=Some("https://api.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.example.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.example.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
new-api | in="https://gateway.example.com/v1" | full=false | proto=None => provider=openai | base=Some("https://gateway.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
new-api | in="https://gateway.example.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://gateway.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
new-api | in="https://gateway.example.com" | full=false | proto=None => provider=openai | base=Some("https://gateway.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
new-api | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
new-api | in="https://gateway.example.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://gateway.example.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
gemini | in="https://generativelanguage.googleapis.com" | full=false | proto=None => provider=openai | base=Some("https://generativelanguage.googleapis.com/v1beta/openai") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
gemini | in="https://generativelanguage.googleapis.com/" | full=false | proto=None => provider=openai | base=Some("https://generativelanguage.googleapis.com/v1beta/openai") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
gemini | in="https://generativelanguage.googleapis.com/v1" | full=false | proto=None => provider=openai | base=Some("https://generativelanguage.googleapis.com/v1/v1beta/openai") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
gemini | in="" | full=false | proto=None => provider=openai | base=Some("/v1beta/openai") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
gemini | in="https://generativelanguage.googleapis.com/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://generativelanguage.googleapis.com/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
gemini-vertex-ai | in="" | full=false | proto=None => provider=vertex | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
gemini-vertex-ai | in="/" | full=false | proto=None => provider=vertex | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
gemini-vertex-ai | in="/v1" | full=false | proto=None => provider=vertex | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
gemini-vertex-ai | in="" | full=false | proto=None => provider=vertex | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
gemini-vertex-ai | in="/chat/completions" | full=true | proto=None => provider=vertex | base=Some("/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://api.openai.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.openai.com") | api_path=None | max_tokens=Some("max_completion_tokens") | image=None | reasoning=None
custom | in="https://api.openai.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.openai.com") | api_path=None | max_tokens=Some("max_completion_tokens") | image=None | reasoning=None
custom | in="https://api.openai.com" | full=false | proto=None => provider=openai | base=Some("https://api.openai.com") | api_path=None | max_tokens=Some("max_completion_tokens") | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.openai.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.openai.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
anthropic | in="https://api.anthropic.com" | full=false | proto=None => provider=anthropic | base=Some("https://api.anthropic.com") | api_path=None | max_tokens=None | image=None | reasoning=None
anthropic | in="https://api.anthropic.com/" | full=false | proto=None => provider=anthropic | base=Some("https://api.anthropic.com") | api_path=None | max_tokens=None | image=None | reasoning=None
anthropic | in="https://api.anthropic.com/v1" | full=false | proto=None => provider=anthropic | base=Some("https://api.anthropic.com") | api_path=None | max_tokens=None | image=None | reasoning=None
anthropic | in="" | full=false | proto=None => provider=anthropic | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
anthropic | in="https://api.anthropic.com/chat/completions" | full=true | proto=None => provider=anthropic | base=Some("https://api.anthropic.com/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
bedrock | in="" | full=false | proto=None => provider=bedrock | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
bedrock | in="/" | full=false | proto=None => provider=bedrock | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
bedrock | in="/v1" | full=false | proto=None => provider=bedrock | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
bedrock | in="" | full=false | proto=None => provider=bedrock | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
bedrock | in="/chat/completions" | full=true | proto=None => provider=bedrock | base=Some("/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
deepseek | in="https://api.deepseek.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.deepseek.com") | api_path=None | max_tokens=None | image=None | reasoning=None
deepseek | in="https://api.deepseek.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.deepseek.com") | api_path=None | max_tokens=None | image=None | reasoning=None
deepseek | in="https://api.deepseek.com" | full=false | proto=None => provider=openai | base=Some("https://api.deepseek.com") | api_path=None | max_tokens=None | image=None | reasoning=None
deepseek | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
deepseek | in="https://api.deepseek.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.deepseek.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
mimo | in="https://api.xiaomimimo.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo | in="https://api.xiaomimimo.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo | in="https://api.xiaomimimo.com" | full=false | proto=None => provider=openai | base=Some("https://api.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
mimo | in="https://api.xiaomimimo.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.xiaomimimo.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
mimo-token-plan-cn | in="https://token-plan-cn.xiaomimimo.com/v1" | full=false | proto=None => provider=openai | base=Some("https://token-plan-cn.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-cn | in="https://token-plan-cn.xiaomimimo.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://token-plan-cn.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-cn | in="https://token-plan-cn.xiaomimimo.com" | full=false | proto=None => provider=openai | base=Some("https://token-plan-cn.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-cn | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-cn | in="https://token-plan-cn.xiaomimimo.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://token-plan-cn.xiaomimimo.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
mimo-token-plan-sgp | in="https://token-plan-sgp.xiaomimimo.com/v1" | full=false | proto=None => provider=openai | base=Some("https://token-plan-sgp.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-sgp | in="https://token-plan-sgp.xiaomimimo.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://token-plan-sgp.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-sgp | in="https://token-plan-sgp.xiaomimimo.com" | full=false | proto=None => provider=openai | base=Some("https://token-plan-sgp.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-sgp | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-sgp | in="https://token-plan-sgp.xiaomimimo.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://token-plan-sgp.xiaomimimo.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
mimo-token-plan-ams | in="https://token-plan-ams.xiaomimimo.com/v1" | full=false | proto=None => provider=openai | base=Some("https://token-plan-ams.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-ams | in="https://token-plan-ams.xiaomimimo.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://token-plan-ams.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-ams | in="https://token-plan-ams.xiaomimimo.com" | full=false | proto=None => provider=openai | base=Some("https://token-plan-ams.xiaomimimo.com") | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-ams | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
mimo-token-plan-ams | in="https://token-plan-ams.xiaomimimo.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://token-plan-ams.xiaomimimo.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
minimax | in="https://api.minimaxi.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.minimaxi.com") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax | in="https://api.minimaxi.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.minimaxi.com") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax | in="https://api.minimaxi.com" | full=false | proto=None => provider=openai | base=Some("https://api.minimaxi.com") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
minimax | in="https://api.minimaxi.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.minimaxi.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
minimax-code | in="https://api.minimax.io/v1" | full=false | proto=None => provider=openai | base=Some("https://api.minimax.io") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-code | in="https://api.minimax.io/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.minimax.io") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-code | in="https://api.minimax.io" | full=false | proto=None => provider=openai | base=Some("https://api.minimax.io") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-code | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-code | in="https://api.minimax.io/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.minimax.io/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
minimax-coding-plan | in="https://api.minimaxi.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.minimaxi.com") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-coding-plan | in="https://api.minimaxi.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.minimaxi.com") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-coding-plan | in="https://api.minimaxi.com" | full=false | proto=None => provider=openai | base=Some("https://api.minimaxi.com") | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-coding-plan | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
minimax-coding-plan | in="https://api.minimaxi.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.minimaxi.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://api.novita.ai/openai/v1" | full=false | proto=None => provider=openai | base=Some("https://api.novita.ai/openai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.novita.ai/openai/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.novita.ai/openai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.novita.ai/openai" | full=false | proto=None => provider=openai | base=Some("https://api.novita.ai/openai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.novita.ai/openai/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.novita.ai/openai/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://openrouter.ai/api/v1" | full=false | proto=None => provider=openai | base=Some("https://openrouter.ai/api") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://openrouter.ai/api/v1/" | full=false | proto=None => provider=openai | base=Some("https://openrouter.ai/api") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://openrouter.ai/api" | full=false | proto=None => provider=openai | base=Some("https://openrouter.ai/api") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://openrouter.ai/api/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://openrouter.ai/api/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
dashscope | in="https://dashscope.aliyuncs.com/compatible-mode/v1" | full=false | proto=None => provider=openai | base=Some("https://dashscope.aliyuncs.com/compatible-mode") | api_path=None | max_tokens=None | image=None | reasoning=None
dashscope | in="https://dashscope.aliyuncs.com/compatible-mode/v1/" | full=false | proto=None => provider=openai | base=Some("https://dashscope.aliyuncs.com/compatible-mode") | api_path=None | max_tokens=None | image=None | reasoning=None
dashscope | in="https://dashscope.aliyuncs.com/compatible-mode" | full=false | proto=None => provider=openai | base=Some("https://dashscope.aliyuncs.com/compatible-mode") | api_path=None | max_tokens=None | image=None | reasoning=None
dashscope | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
dashscope | in="https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
dashscope-coding | in="https://coding.dashscope.aliyuncs.com/v1" | full=false | proto=None => provider=openai | base=Some("https://coding.dashscope.aliyuncs.com/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
dashscope-coding | in="https://coding.dashscope.aliyuncs.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://coding.dashscope.aliyuncs.com/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
dashscope-coding | in="https://coding.dashscope.aliyuncs.com" | full=false | proto=None => provider=openai | base=Some("https://coding.dashscope.aliyuncs.com") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
dashscope-coding | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
dashscope-coding | in="https://coding.dashscope.aliyuncs.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://coding.dashscope.aliyuncs.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.cn/v1" | full=false | proto=None => provider=openai | base=Some("https://api.siliconflow.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.cn/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.siliconflow.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.cn" | full=false | proto=None => provider=openai | base=Some("https://api.siliconflow.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.cn/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.siliconflow.cn/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.siliconflow.com") | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.siliconflow.com") | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.com" | full=false | proto=None => provider=openai | base=Some("https://api.siliconflow.com") | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
siliconflow | in="https://api.siliconflow.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.siliconflow.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
zhipu | in="https://open.bigmodel.cn/api/paas/v4" | full=false | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/paas/v4") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
zhipu | in="https://open.bigmodel.cn/api/paas/v4/" | full=false | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/paas/v4") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
zhipu | in="https://open.bigmodel.cn/api/paas/v4/v1" | full=false | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/paas/v4/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
zhipu | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
zhipu | in="https://open.bigmodel.cn/api/paas/v4/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/paas/v4/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
glm-coding-plan | in="https://open.bigmodel.cn/api/coding/paas/v4" | full=false | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/coding/paas/v4") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
glm-coding-plan | in="https://open.bigmodel.cn/api/coding/paas/v4/" | full=false | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/coding/paas/v4") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
glm-coding-plan | in="https://open.bigmodel.cn/api/coding/paas/v4/v1" | full=false | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/coding/paas/v4/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
glm-coding-plan | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
glm-coding-plan | in="https://open.bigmodel.cn/api/coding/paas/v4/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://open.bigmodel.cn/api/coding/paas/v4/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
moonshot-cn | in="https://api.moonshot.cn/v1" | full=false | proto=None => provider=openai | base=Some("https://api.moonshot.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-cn | in="https://api.moonshot.cn/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.moonshot.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-cn | in="https://api.moonshot.cn" | full=false | proto=None => provider=openai | base=Some("https://api.moonshot.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-cn | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-cn | in="https://api.moonshot.cn/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.moonshot.cn/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
moonshot-global | in="https://api.moonshot.ai/v1" | full=false | proto=None => provider=openai | base=Some("https://api.moonshot.ai") | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-global | in="https://api.moonshot.ai/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.moonshot.ai") | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-global | in="https://api.moonshot.ai" | full=false | proto=None => provider=openai | base=Some("https://api.moonshot.ai") | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-global | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
moonshot-global | in="https://api.moonshot.ai/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.moonshot.ai/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://api.x.ai/v1" | full=false | proto=None => provider=openai | base=Some("https://api.x.ai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.x.ai/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.x.ai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.x.ai" | full=false | proto=None => provider=openai | base=Some("https://api.x.ai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.x.ai/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.x.ai/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
ark | in="https://ark.cn-beijing.volces.com/api/v3" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/v3") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark | in="https://ark.cn-beijing.volces.com/api/v3/" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/v3") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark | in="https://ark.cn-beijing.volces.com/api/v3/v1" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/v3/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark | in="https://ark.cn-beijing.volces.com/api/v3/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/v3/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
ark-coding-plan | in="https://ark.cn-beijing.volces.com/api/coding/v3" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/coding/v3") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-coding-plan | in="https://ark.cn-beijing.volces.com/api/coding/v3/" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/coding/v3") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-coding-plan | in="https://ark.cn-beijing.volces.com/api/coding/v3/v1" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/coding/v3/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-coding-plan | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-coding-plan | in="https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
ark-agent-plan | in="https://ark.cn-beijing.volces.com/api/plan/v3" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/plan/v3") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-agent-plan | in="https://ark.cn-beijing.volces.com/api/plan/v3/" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/plan/v3") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-agent-plan | in="https://ark.cn-beijing.volces.com/api/plan/v3/v1" | full=false | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/plan/v3/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-agent-plan | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
ark-agent-plan | in="https://ark.cn-beijing.volces.com/api/plan/v3/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://ark.cn-beijing.volces.com/api/plan/v3/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
qianfan | in="https://qianfan.baidubce.com/v2" | full=false | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan | in="https://qianfan.baidubce.com/v2/" | full=false | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan | in="https://qianfan.baidubce.com/v2/v1" | full=false | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan | in="https://qianfan.baidubce.com/v2/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
qianfan-coding-plan | in="https://qianfan.baidubce.com/v2/coding" | full=false | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2/coding") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan-coding-plan | in="https://qianfan.baidubce.com/v2/coding/" | full=false | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2/coding") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan-coding-plan | in="https://qianfan.baidubce.com/v2/coding/v1" | full=false | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2/coding/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan-coding-plan | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
qianfan-coding-plan | in="https://qianfan.baidubce.com/v2/coding/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://qianfan.baidubce.com/v2/coding/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
hunyuan | in="https://api.hunyuan.cloud.tencent.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.hunyuan.cloud.tencent.com") | api_path=None | max_tokens=None | image=None | reasoning=None
hunyuan | in="https://api.hunyuan.cloud.tencent.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.hunyuan.cloud.tencent.com") | api_path=None | max_tokens=None | image=None | reasoning=None
hunyuan | in="https://api.hunyuan.cloud.tencent.com" | full=false | proto=None => provider=openai | base=Some("https://api.hunyuan.cloud.tencent.com") | api_path=None | max_tokens=None | image=None | reasoning=None
hunyuan | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
hunyuan | in="https://api.hunyuan.cloud.tencent.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.hunyuan.cloud.tencent.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
lingyi | in="https://api.lingyiwanwu.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.lingyiwanwu.com") | api_path=None | max_tokens=None | image=None | reasoning=None
lingyi | in="https://api.lingyiwanwu.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.lingyiwanwu.com") | api_path=None | max_tokens=None | image=None | reasoning=None
lingyi | in="https://api.lingyiwanwu.com" | full=false | proto=None => provider=openai | base=Some("https://api.lingyiwanwu.com") | api_path=None | max_tokens=None | image=None | reasoning=None
lingyi | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
lingyi | in="https://api.lingyiwanwu.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.lingyiwanwu.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://api.poe.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.poe.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.poe.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.poe.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.poe.com" | full=false | proto=None => provider=openai | base=Some("https://api.poe.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.poe.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.poe.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://api.ppinfra.com/v3/openai" | full=false | proto=None => provider=openai | base=Some("https://api.ppinfra.com/v3/openai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.ppinfra.com/v3/openai/" | full=false | proto=None => provider=openai | base=Some("https://api.ppinfra.com/v3/openai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.ppinfra.com/v3/openai/v1" | full=false | proto=None => provider=openai | base=Some("https://api.ppinfra.com/v3/openai") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.ppinfra.com/v3/openai/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.ppinfra.com/v3/openai/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://api-inference.modelscope.cn/v1" | full=false | proto=None => provider=openai | base=Some("https://api-inference.modelscope.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api-inference.modelscope.cn/v1/" | full=false | proto=None => provider=openai | base=Some("https://api-inference.modelscope.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api-inference.modelscope.cn" | full=false | proto=None => provider=openai | base=Some("https://api-inference.modelscope.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api-inference.modelscope.cn/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api-inference.modelscope.cn/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://cloud.infini-ai.com/maas/v1" | full=false | proto=None => provider=openai | base=Some("https://cloud.infini-ai.com/maas") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://cloud.infini-ai.com/maas/v1/" | full=false | proto=None => provider=openai | base=Some("https://cloud.infini-ai.com/maas") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://cloud.infini-ai.com/maas" | full=false | proto=None => provider=openai | base=Some("https://cloud.infini-ai.com/maas") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://cloud.infini-ai.com/maas/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://cloud.infini-ai.com/maas/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
custom | in="https://wishub-x1.ctyun.cn/v1" | full=false | proto=None => provider=openai | base=Some("https://wishub-x1.ctyun.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://wishub-x1.ctyun.cn/v1/" | full=false | proto=None => provider=openai | base=Some("https://wishub-x1.ctyun.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://wishub-x1.ctyun.cn" | full=false | proto=None => provider=openai | base=Some("https://wishub-x1.ctyun.cn") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://wishub-x1.ctyun.cn/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://wishub-x1.ctyun.cn/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
stepfun | in="https://api.stepfun.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.stepfun.com/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun | in="https://api.stepfun.com/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.stepfun.com/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun | in="https://api.stepfun.com" | full=false | proto=None => provider=openai | base=Some("https://api.stepfun.com") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun | in="https://api.stepfun.com/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.stepfun.com/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
stepfun-plan | in="https://api.stepfun.com/step_plan/v1" | full=false | proto=None => provider=openai | base=Some("https://api.stepfun.com/step_plan/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun-plan | in="https://api.stepfun.com/step_plan/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.stepfun.com/step_plan/v1") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun-plan | in="https://api.stepfun.com/step_plan" | full=false | proto=None => provider=openai | base=Some("https://api.stepfun.com/step_plan") | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun-plan | in="" | full=false | proto=None => provider=openai | base=None | api_path=Some("/chat/completions") | max_tokens=None | image=None | reasoning=None
stepfun-plan | in="https://api.stepfun.com/step_plan/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.stepfun.com/step_plan/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
nomifun-free-model | in="https://free.nomifun.example/v1" | full=false | proto=None => provider=openai | base=Some("https://free.nomifun.example") | api_path=None | max_tokens=None | image=None | reasoning=None
nomifun-free-model | in="https://free.nomifun.example/v1/" | full=false | proto=None => provider=openai | base=Some("https://free.nomifun.example") | api_path=None | max_tokens=None | image=None | reasoning=None
nomifun-free-model | in="https://free.nomifun.example" | full=false | proto=None => provider=openai | base=Some("https://free.nomifun.example") | api_path=None | max_tokens=None | image=None | reasoning=None
nomifun-free-model | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
nomifun-free-model | in="https://free.nomifun.example/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://free.nomifun.example/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
totally-unknown | in="https://api.example.org/v1" | full=false | proto=None => provider=openai | base=Some("https://api.example.org") | api_path=None | max_tokens=None | image=None | reasoning=None
totally-unknown | in="https://api.example.org/v1/" | full=false | proto=None => provider=openai | base=Some("https://api.example.org") | api_path=None | max_tokens=None | image=None | reasoning=None
totally-unknown | in="https://api.example.org" | full=false | proto=None => provider=openai | base=Some("https://api.example.org") | api_path=None | max_tokens=None | image=None | reasoning=None
totally-unknown | in="" | full=false | proto=None => provider=openai | base=None | api_path=None | max_tokens=None | image=None | reasoning=None
totally-unknown | in="https://api.example.org/v1/chat/completions" | full=true | proto=None => provider=openai | base=Some("https://api.example.org/v1/chat/completions") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
new-api | in="https://gateway.example.com/v1" | full=false | proto=Some("anthropic") => provider=anthropic | base=Some("https://gateway.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
new-api | in="https://gateway.example.com/v1" | full=false | proto=Some("openai") => provider=openai | base=Some("https://gateway.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
new-api | in="https://gateway.example.com/v1" | full=false | proto=Some("gemini") => provider=openai | base=Some("https://gateway.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="https://api.example.com/v1" | full=false | proto=Some("anthropic") => provider=openai | base=Some("https://api.example.com") | api_path=None | max_tokens=None | image=None | reasoning=None
anthropic | in="https://api.anthropic.com" | full=false | proto=Some("openai") => provider=anthropic | base=Some("https://api.anthropic.com") | api_path=None | max_tokens=None | image=None | reasoning=None
new-api | in="https://api.openai.com/v1" | full=false | proto=None => provider=openai | base=Some("https://api.openai.com") | api_path=None | max_tokens=Some("max_completion_tokens") | image=None | reasoning=None
new-api | in="https://api.openai.com/v1" | full=false | proto=Some("anthropic") => provider=anthropic | base=Some("https://api.openai.com") | api_path=None | max_tokens=None | image=None | reasoning=None
custom | in="" | full=true | proto=None => provider=openai | base=Some("") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
gemini | in="https://proxy.example.com/gemini/chat" | full=true | proto=None => provider=openai | base=Some("https://proxy.example.com/gemini/chat") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
ark | in="https://proxy.example.com/ark/chat" | full=true | proto=None => provider=openai | base=Some("https://proxy.example.com/ark/chat") | api_path=Some("") | max_tokens=None | image=None | reasoning=None
"#;
}
