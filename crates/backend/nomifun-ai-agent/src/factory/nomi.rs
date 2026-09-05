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
use crate::image_generation::{
    CatalogImageGenerationToolDiscovery, ImageGenerationToolDiscovery, image_generation_prompt,
};
use crate::manager::nomi::{
    NomiAgentManager, NomiHostWiring, NomiSummonWiring, sanitize_session_messages,
};
use crate::types::{AgentRuntimeBuildOptions, NomiResolvedConfig};

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

/// Effective host authority for a Nomi runtime. Channel conversations are
/// physically owned by the installation owner, so the principal alone is not
/// enough: an automatically admitted group member must lower that authority at
/// the same single ceiling used for secondary users.
fn has_effective_host_authority(
    authority: ExecutionAuthority,
    channel_group_guest: bool,
) -> bool {
    authority.controls_host() && !channel_group_guest
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
    let is_instance_owner =
        has_effective_host_authority(authority, overrides.channel_group_guest);

    // Gateway entitlement is derived from the immutable principal, never from
    // persisted/open JSON. Process-owned config is injected only after the
    // ownership ceiling has been applied.
    overrides.gateway_mcp_config = None;

    // A non-owner runtime is deliberately model-only.  Hiding a few tools is
    // insufficient because every native shell process shares the backend's
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
                     需要补查伙伴记忆用 recall_memories；伙伴记忆在本会话中只读，不可写入。",
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
                    provider.summon_context_sink(summon),
                ) {
                    (Ok(memory_sink), Ok(context_sink)) => {
                        summon_wiring = Some(NomiSummonWiring {
                            memory_sink,
                            context_sink,
                        });
                    }
                    (memory, context) => {
                        warn!(
                            conversation_id = %ctx.conversation_id,
                            memory_sink_err = ?memory.err().map(|e| e.to_string()),
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

    // Build a retained local discovery authority from the authoritative
    // catalog. The manager reruns this resolver before every admitted turn (no
    // health probe / generation request), so provider, model, connection,
    // credential, task-tag and default changes do not require runtime teardown.
    // An incomplete candidate never leaks an unusable provider schema, and
    // restricted principals retain their model-only ceiling.
    let image_generation_discovery: Option<Arc<dyn ImageGenerationToolDiscovery>> =
        if platform_gateway_entitled {
            deps.model_invoke_service.as_ref().map(|invoke| {
                Arc::new(CatalogImageGenerationToolDiscovery::new(
                    deps.client_prefs.clone(),
                    invoke.clone(),
                )) as Arc<dyn ImageGenerationToolDiscovery>
            })
        } else {
            None
        };
    let (image_generation_tool, image_generation_discovery_failed) =
        match image_generation_discovery.as_ref() {
            Some(discovery) => match discovery.discover_tool().await {
                Ok(tool) => (tool, false),
                Err(error) => {
                    warn!(
                        conversation_id = %ctx.conversation_id,
                        error = %error,
                        "image_gen: catalog discovery failed closed"
                    );
                    (None, true)
                }
            },
            None => (None, false),
        };

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
            // Threaded from the binding via MountOutcome → build-extra so the
            // external-IM-channel opt-in actually reaches resolve_write_policy;
            // a `..Default::default()` here would pin it to `false` and keep
            // channel write-back permanently disabled on the nomi engine.
            channel_write_enabled: overrides.knowledge_channel_write_enabled,
            ..Default::default()
        },
    );
    let knowledge_write_enabled = !matches!(
        knowledge_write_policy.mode,
        nomifun_knowledge::WriteMode::Disabled
    );
    // Prompt capability must be derived from the same effective surface that
    // can register the native tools. Restricted workers retain a persistent
    // allowlist, so advertising search/read when either name is absent creates
    // a deterministic provider-authority violation later in the turn.
    let knowledge_search_enabled = should_expose_knowledge_search(
        is_instance_owner,
        deps.knowledge_retrieval.is_some(),
        !overrides.knowledge_mounts.is_empty(),
        &overrides.allowed_tools,
    );

    // Knowledge bases: append the mounted-bases section (per-base TOC +
    // write-back contract) to the system prompt, so nomi-engine sessions
    // (companion companion threads included) see the same knowledge context the
    // engine assembles into its session context.
    overrides.system_prompt = append_knowledge_context(
        overrides.system_prompt.take(),
        &overrides,
        knowledge_search_enabled,
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

    let app_language = read_app_language(deps.settings_repo.as_ref()).await;

    // The prompt is policy, not capability authority: registration below is
    // still conditional on a ready catalog snapshot. It tells weaker chat
    // models that ordinary image requests have exactly one native route and
    // that Browser/Computer/shell/third-party sites are reserved for an
    // explicit user request. The manager additionally enforces that route at
    // the advertised-tool and artifact-receipt boundaries.
    let image_policy = if platform_gateway_entitled {
        image_generation_prompt(None)
    } else {
        "This restricted Agent session is not entitled to native image generation. Do not use Browser, web search, or a third-party generator as a substitute, and do not claim that an image was generated. Tell the user to retry in a full local session or ask the session owner to enable the native capability.".to_owned()
    };
    overrides.system_prompt = Some(match overrides.system_prompt.take() {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{image_policy}")
        }
        _ => image_policy,
    });

    // Every native Nomi session — regular desktop chat, companion, and IM
    // Channel Agent — follows the language of the owner's current request.
    // This is appended last so an English base prompt, memories, retrieved
    // context, or tool output cannot pin the conversation to their language.
    // The decision is made again for every request instead of being frozen to
    // the app UI locale for the lifetime of a session.
    let directive = output_language_directive();
    overrides.system_prompt = Some(match overrides.system_prompt.take() {
        Some(existing) => format!("{existing}\n\n{directive}"),
        None => directive.to_owned(),
    });

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

    let fields = super::provider_config::resolve_provider_fields(
        deps.model_invoke.as_ref(),
        provider_id,
        &model_id,
    )
    .await?;

    let session_directory = deps.data_dir.join("nomi-sessions");
    let output_ceiling = fields
        .output_limit
        .map(u32::try_from)
        .transpose()
        .map_err(|_| {
            AppError::BadRequest(
                "chat capability output_limit exceeds the supported u32 token range".to_owned(),
            )
        })?;

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
                    rewritten_tool_search_results = repair.rewritten_tool_search_results,
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
        output_ceiling,
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

    let knowledge_prelude: Option<String> = if !knowledge_search_enabled {
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
    // clear error rather than silently running against the local machine. The
    // conversation id goes with the request: the provider pools one link per
    // (conversation, host), so a runtime rebuilt by a model switch rejoins the
    // session its predecessor was using instead of dialling again.
    let ssh_session = if let Some(ssh_host_id) = overrides.ssh_host_id.clone() {
        let user_id = overrides.user_id.clone().unwrap_or_default();
        let remote_cwd = overrides
            .ssh_remote_cwd
            .clone()
            .unwrap_or_else(|| ".".to_string());
        match &deps.ssh_provider {
            Some(provider) => Some(
                provider
                    .connect(
                        &user_id,
                        ctx.conversation_id.as_str(),
                        &ssh_host_id,
                        &remote_cwd,
                    )
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
        ssh_backend: ssh_session.as_ref().map(|s| Arc::clone(&s.backend)),
        ssh_lease: ssh_session.map(|s| s.lease),
        image_generation_tool,
        image_generation_discovery,
        image_generation_entitled: platform_gateway_entitled,
        image_generation_discovery_failed,
        image_generation_response_in_chinese: app_language == "zh-CN",
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
        knowledge_search_enabled
            .then(|| deps.knowledge_retrieval.clone())
            .flatten(),
        knowledge_kb_ids,
        knowledge_prelude,
        knowledge_writeback_sink,
        knowledge_write_bases,
        if is_instance_owner && overrides.companion {
            deps.companion_skill_sink.clone()
        } else {
            None
        },
        // Computer-history tools observe the local user, so they are gated to
        // the installation owner and dropped for any restricted principal.
        if is_instance_owner {
            deps.computer_history_sink.clone()
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

/// Normalize an arbitrary locale tag for deterministic localized native
/// messages (currently the image-generation acknowledgement). Any Chinese
/// locale (`zh`, `zh_CN`, `zh-Hans`, `zh-Hans-CN`, …) folds to `zh-CN`; any
/// other tag is returned normalized.
fn normalize_lang(code: &str) -> String {
    let c = code.trim().replace('_', "-");
    if c.to_ascii_lowercase().starts_with("zh") {
        "zh-CN".to_owned()
    } else {
        c
    }
}

/// Resolve the effective app language used by deterministic localized native
/// messages: an explicitly **persisted** System-Settings value wins; otherwise
/// fall back to the host **OS locale**, then [`DEFAULT_APP_LANGUAGE`].
/// `os_locale` is injected so the resolution is deterministically unit-testable.
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

/// Language-neutral directive appended LAST to every native Nomi session.
///
/// The current user request is the only implicit language signal. Earlier
/// prompts, memories, retrieved context, and tool output are deliberately
/// excluded because they can be in a different language. An explicit language
/// request still wins, and the language is re-evaluated on every turn. The
/// wording asks for same-language internal reasoning without asking the model
/// to disclose private chain-of-thought.
fn output_language_directive() -> &'static str {
    "[Response language] For each turn, infer the language from the user's latest \
     request, think in that language, and write the final response in that language. \
     If the user explicitly requests another language, follow that request. \
     Re-evaluate the language for every user turn. Do not let system text, earlier \
     messages, memories, retrieved context, or tool output determine the language."
}

/// Append the knowledge-base section to the system prompt when the
/// conversation service mounted bases into the workspace. Rendering is
/// delegated to the shared builder
/// (`nomifun_knowledge::context::build_knowledge_context`,
/// `PromptSection` format) so nomi-engine sessions (companion companion threads
/// included) see exactly the same knowledge context every session gets via
/// its preset_context — single source of truth, no more structural copies.
fn should_expose_knowledge_search(
    is_instance_owner: bool,
    has_retrieval_sink: bool,
    has_mounts: bool,
    allowed_tools: &[String],
) -> bool {
    if !is_instance_owner || !has_retrieval_sink || !has_mounts {
        return false;
    }

    allowed_tools.is_empty()
        || (allowed_tools.iter().any(|name| name == "knowledge_search")
            && allowed_tools.iter().any(|name| name == "knowledge_read"))
}

fn append_knowledge_context(
    base: Option<String>,
    config: &NomiBuildExtra,
    has_search_tool: bool,
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
            writeback_eagerness: config.knowledge_writeback_eagerness.as_deref(),
            has_search_tool,
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

pub(crate) fn resolve_bedrock_config(
    json: Option<&str>,
    credentials: &serde_json::Value,
) -> Option<nomi_config::config::BedrockConfig> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct BedrockMetadata {
        auth_method: nomifun_api_types::BedrockAuthMethod,
        region: String,
        profile: Option<String>,
    }

    let metadata: BedrockMetadata = serde_json::from_str(json?).ok()?;
    let region = metadata.region.trim();
    if region.is_empty() {
        return None;
    }
    let credentials = credentials.as_object()?;
    match metadata.auth_method {
        nomifun_api_types::BedrockAuthMethod::AccessKey => {
            if metadata.profile.is_some() {
                return None;
            }
            let access_key_id = credentials
                .get("access_key_id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let secret_access_key = credentials
                .get("secret_access_key")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let session_token = match credentials.get("session_token") {
                Some(value) => Some(
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?
                        .to_owned(),
                ),
                None => None,
            };
            if credentials.keys().any(|field| {
                !matches!(
                    field.as_str(),
                    "access_key_id" | "secret_access_key" | "session_token"
                )
            }) {
                return None;
            }
            Some(nomi_config::config::BedrockConfig {
                region: Some(region.to_owned()),
                access_key_id: Some(access_key_id.to_owned()),
                secret_access_key: Some(secret_access_key.to_owned()),
                session_token,
                profile: None,
            })
        }
        nomifun_api_types::BedrockAuthMethod::Profile => {
            if !credentials.is_empty() {
                return None;
            }
            let profile = metadata
                .profile
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            Some(nomi_config::config::BedrockConfig {
                region: Some(region.to_owned()),
                access_key_id: None,
                secret_access_key: None,
                session_token: None,
                profile: Some(profile.to_owned()),
            })
        }
        nomifun_api_types::BedrockAuthMethod::DefaultChain => {
            if !credentials.is_empty() || metadata.profile.is_some() {
                return None;
            }
            Some(nomi_config::config::BedrockConfig {
                region: Some(region.to_owned()),
                access_key_id: None,
                secret_access_key: None,
                session_token: None,
                profile: None,
            })
        }
    }
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
                request_timeout_secs: None,
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
                request_timeout_secs: None,
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
                request_timeout_secs: None,
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
                request_timeout_secs: None,
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
                request_timeout_secs: None,
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
                request_timeout_secs: None,
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
                request_timeout_secs: None,
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

/// Platform Gateway MCP stdio bridge config for the Nomi engine. Caller
/// conversation + user ids ride along for self-protection and data scoping; the
/// companion binding (when present) rides along for attribution.
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
        request_timeout_secs: None,
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
    fn owner_backed_group_guest_is_still_model_only() {
        let authority = ExecutionAuthority::resolve("owner", "owner");
        assert!(authority.controls_host());
        assert!(!has_effective_host_authority(authority, true));
        assert!(has_effective_host_authority(authority, false));

        let mcp_server_id = McpServerId::new();
        let mut overrides = NomiBuildExtra {
            channel_group_guest: true,
            channel_platform: Some("lark".into()),
            computer_use: Some(true),
            browser_use: Some(true),
            gateway_mcp_config: Some(gateway_config(41237, "/usr/bin/nomicore", "owner")),
            mcp_server_ids: Some(vec![mcp_server_id.clone()]),
            session_mcp_servers: vec![SessionMcpServer {
                mcp_server_id,
                name: "guest-mcp".into(),
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
                name: "private knowledge".into(),
                description: "must not remain mounted for a group guest".into(),
                rel_path: ".nomi/knowledge/private".into(),
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
            goal: Some(nomifun_api_types::NomiGoalSpec {
                objective: "persist autonomous work".into(),
                max_auto_continuations: None,
            }),
            delegation_policy: DelegationPolicy::Automatic,
            ..Default::default()
        };

        apply_model_only_ceiling(&mut overrides);

        assert!(overrides.gateway_mcp_config.is_none());
        assert_eq!(overrides.computer_use, Some(false));
        assert_eq!(overrides.browser_use, Some(false));
        assert!(overrides.mcp_server_ids.is_none());
        assert!(overrides.session_mcp_servers.is_empty());
        assert!(!overrides.companion && overrides.companion_id.is_none());
        assert!(overrides.channel_platform.is_none());
        assert!(overrides.knowledge_mounts.is_empty());
        assert!(!overrides.knowledge_writeback);
        assert!(!overrides.knowledge_channel_write_enabled);
        assert!(overrides.summon.is_none());
        assert_eq!(overrides.allowed_tools, vec!["update_plan"]);
        assert_eq!(overrides.max_turns, Some(1));
        assert!(overrides.goal.is_none());
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
            host_context: Default::default(),
            accepted_turn_root: None,
            pending_host_terminal_root: None,
            last_interrupted_turn_source: None,
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
                prior_host_context: Default::default(),
            }),
            host_context: Default::default(),
            accepted_turn_root: None,
            pending_host_terminal_root: None,
            last_interrupted_turn_source: None,
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

    // ----- output-language directive (follow each current user request) -----

    /// Minimal mock settings repo for `read_app_language`: yields a fixed result
    /// (`Err(())` simulates a DB read failure).
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
    fn output_language_directive_follows_each_current_user_request() {
        let directive = output_language_directive();
        assert!(directive.contains("latest"));
        assert!(directive.contains("think in that language"));
        assert!(directive.contains("final response"));
        assert!(directive.contains("Re-evaluate"));
        assert!(!directive.contains("English"));
        assert!(!directive.contains("简体中文"));
        assert!(!directive.contains("app UI language"));
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
        // Non-Chinese tags are returned normalized unchanged.
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
                request_timeout_secs: None,
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
        let json = r#"{"auth_method":"accessKey","region":"us-west-2"}"#;
        let result = resolve_bedrock_config(
            Some(json),
            &serde_json::json!({
                "access_key_id": "AKIA123",
                "secret_access_key": "secret456",
                "session_token": "sts789"
            }),
        )
        .unwrap();
        assert_eq!(result.region.as_deref(), Some("us-west-2"));
        assert_eq!(result.access_key_id.as_deref(), Some("AKIA123"));
        assert_eq!(result.secret_access_key.as_deref(), Some("secret456"));
        assert!(result.profile.is_none());
        assert_eq!(result.session_token.as_deref(), Some("sts789"));
    }

    #[test]
    fn resolve_bedrock_config_profile() {
        let json = r#"{"auth_method":"profile","region":"eu-west-1","profile":"my-profile"}"#;
        let result = resolve_bedrock_config(Some(json), &serde_json::json!({})).unwrap();
        assert_eq!(result.region.as_deref(), Some("eu-west-1"));
        assert_eq!(result.profile.as_deref(), Some("my-profile"));
        assert!(result.access_key_id.is_none());
        assert!(result.secret_access_key.is_none());
    }

    #[test]
    fn resolve_bedrock_config_default_chain_requires_empty_credentials() {
        let json = r#"{"auth_method":"defaultChain","region":"ap-southeast-1"}"#;
        let result = resolve_bedrock_config(Some(json), &serde_json::json!({})).unwrap();
        assert_eq!(result.region.as_deref(), Some("ap-southeast-1"));
        assert!(result.access_key_id.is_none());
        assert!(result.secret_access_key.is_none());
        assert!(result.session_token.is_none());
        assert!(result.profile.is_none());
        assert!(
            resolve_bedrock_config(
                Some(json),
                &serde_json::json!({"access_key_id":"must-not-be-used"}),
            )
            .is_none()
        );
    }

    #[test]
    fn resolve_bedrock_config_none_when_json_missing() {
        assert!(resolve_bedrock_config(None, &serde_json::json!({})).is_none());
    }

    #[test]
    fn resolve_bedrock_config_none_when_json_invalid() {
        assert!(
            resolve_bedrock_config(Some("not-json"), &serde_json::json!({})).is_none()
        );
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
    fn knowledge_search_prompt_matches_the_effective_tool_surface() {
        let unrestricted = Vec::<String>::new();
        assert!(super::should_expose_knowledge_search(
            true,
            true,
            true,
            &unrestricted,
        ));
        assert!(!super::should_expose_knowledge_search(
            false,
            true,
            true,
            &unrestricted,
        ));
        assert!(!super::should_expose_knowledge_search(
            true,
            false,
            true,
            &unrestricted,
        ));
        assert!(!super::should_expose_knowledge_search(
            true,
            true,
            false,
            &unrestricted,
        ));

        let only_search = vec!["knowledge_search".to_owned()];
        assert!(!super::should_expose_knowledge_search(
            true,
            true,
            true,
            &only_search,
        ));
        let complete_surface = vec!["knowledge_search".to_owned(), "knowledge_read".to_owned()];
        assert!(super::should_expose_knowledge_search(
            true,
            true,
            true,
            &complete_surface,
        ));
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
            append_knowledge_context(None, &config, true, true),
            None
        );
        assert_eq!(
            append_knowledge_context(Some("hello".into()), &config, true, true),
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
            append_knowledge_context(Some("base".into()), &config, true, true).unwrap();
        assert!(readonly.starts_with("base\n\n"));
        assert!(readonly.contains("## Knowledge bases"));
        assert!(readonly.contains("领域知识"));
        assert!(readonly.contains("intro.md — 简介"));
        assert!(readonly.contains("READ-ONLY"));
        // Hit-rate contract: retrieval protocol (once), per-base summary and
        // when-to-consult guidance — the shared context builder.
        assert_eq!(readonly.matches("Retrieval protocol").count(), 1);
        assert!(readonly.contains("Covers deployment flows and runbooks."));
        assert!(readonly.contains("When to consult"));

        // nomi surface has the native tool → the write-back contract points at
        // it, and no session id or inbox path can leak into the prompt any more.
        config.knowledge_writeback = true;
        let tooled = append_knowledge_context(None, &config, true, true).unwrap();
        assert!(tooled.contains("Write-back is ENABLED"));
        assert!(tooled.contains("knowledge_write"));
        assert!(!tooled.contains("STAGED"));
        assert!(!tooled.contains("_inbox"));
        assert!(
            !tooled.contains(conversation_id),
            "the contract must not carry a session id: {tooled}"
        );
        // Flag plumbs through: without the tool, the file-based prose returns.
        let file_based = append_knowledge_context(None, &config, true, false).unwrap();
        assert!(file_based.contains("knowledge base directory"));
        assert!(!file_based.contains("knowledge_write"));
        // Disposition (回写意识) threads from build-extra → contract.
        assert!(tooled.contains("Disposition — MANUAL"));
        config.knowledge_writeback_eagerness = Some("auto".into());
        let eager = append_knowledge_context(None, &config, true, true).unwrap();
        assert!(eager.contains("Disposition — AUTO"));
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
            "knowledge_writeback_eagerness": "auto",
        });
        let overrides: NomiBuildExtra = serde_json::from_value(json).unwrap();
        assert_eq!(overrides.knowledge_mounts.len(), 1);
        assert!(overrides.knowledge_writeback);
        assert_eq!(
            overrides.knowledge_writeback_eagerness.as_deref(),
            Some("auto")
        );

        let prompt = append_knowledge_context(None, &overrides, true, true).unwrap();
        assert!(prompt.contains("Knowledge bases"));
        assert!(prompt.contains("运维手册"));
        assert!(prompt.contains("knowledge_write"));
        // The disposition keyword threads all the way from extra JSON to prompt.
        assert!(prompt.contains("Disposition — AUTO"));
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
        // opt-in is what flips an unattended channel from refusing writes to
        // making them. This switch is the only thing left standing between a
        // bot and the base body, so it must keep working end to end.
        let reconstruct = |extra: &NomiBuildExtra| KnowledgeBinding {
            enabled: true,
            writeback: extra.knowledge_writeback,
            channel_write_enabled: extra.knowledge_channel_write_enabled,
            ..Default::default()
        };

        let disabled = resolve_write_policy(WriteSurface::ExternalChannel, &reconstruct(&off));
        assert!(matches!(disabled.mode, WriteMode::Disabled));

        let enabled = resolve_write_policy(WriteSurface::ExternalChannel, &reconstruct(&on));
        assert!(matches!(enabled.mode, WriteMode::Direct));
    }
}
