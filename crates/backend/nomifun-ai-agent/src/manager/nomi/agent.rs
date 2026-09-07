use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nomi_agent::bootstrap::AgentBootstrap;
use nomi_agent::companion_tools::{
    CompanionMemorySink, CompanionSkillContributor, CompanionSkillSink, CompanionSkillTool, ListRecentEventsTool,
    RecallMemoriesTool, SaveMemoryTool,
};
use nomi_agent::computer_history_tools::register_computer_history_tools;
use nomi_agent::summon_tools::{
    SummonContextContributor, SummonContextSink,
};
use nomi_agent::engine::{AgentEngine, CompletionEvidenceContext};
use nomi_agent::knowledge_tools::{KnowledgeReadTool, KnowledgeSearchTool, KnowledgeWriteTool};
use nomi_agent::output::OutputSink;
use nomi_agent::requirement_tools::{RequirementCompleteTool, RequirementSink, RequirementUpdateStatusTool};
use nomi_agent::cron_tools::{CronCreateTool, CronDeleteTool, CronListTool, CronSink};
use nomi_agent::session::Session;
use nomi_config::config::{CliArgs, Config};
use nomi_mcp::manager::McpManager;
use nomi_protocol::commands::SessionMode;
#[cfg(feature = "browser-use")]
use nomi_protocol::events::ToolCategory;
use nomi_protocol::{ToolApprovalManager, ToolApprovalResult};
use nomi_types::message::ContentBlock;
use nomifun_api_types::{
    AgentErrorCode, AgentErrorOwnership, AgentErrorResolution, AgentErrorResolutionKind,
    AgentErrorResolutionTarget, AgentModeResponse, SlashCommandItem,
};
use nomifun_common::{
    AgentKillReason, AgentType, AppError, Confirmation, ConversationStatus, ErrorChain, TimestampMs, now_ms,
};
use serde_json::Value;
use tokio::sync::{Mutex, Notify, broadcast};
use tracing::{debug, error, info};

use crate::runtime_state::AgentRuntimeState;
use crate::runtime_handle::SystemResourceNoticeDelivery;
use crate::capability::backend_output_sink::{
    AsyncArtifactDeliveryOutcome, BackendOutputSink,
};
use crate::capability::backend_protocol_sink::BackendProtocolSink;
use crate::image_generation::{
    IMAGE_GEN_TOOL_NAME, ImageGenerationIntent, ImageGenerationToolDiscovery,
    classify_image_generation_intent, classify_image_generation_intent_with_model,
    explicitly_requests_external_image_execution, image_intent_attachment_summary,
};
use crate::protocol::events::{AgentStreamEvent, TurnCompletedEventData, TurnStopReason};
use crate::protocol::send_error::AgentSendError;
use crate::types::{NomiResolvedConfig, SendMessageData};

use super::image_attachments::{ImageAttachmentError, load_image_blocks};

/// Process-level memory of which `(provider, model)` pairs have already been
/// reported as running on an assumed context window.
///
/// `apply_provider_token_budget` runs on every runtime build, and a runtime is
/// rebuilt whenever the registry has to recreate one for a turn — so an
/// unannotated capability would repeat the same diagnostic indefinitely. Keying
/// on provider + model keeps a second, differently-sized model on the same
/// provider reportable while collapsing the repeats for one session.
///
/// This mirrors [`nomifun_common::VisionUnsupportedRegistry`]: a plain
/// `std::sync::Mutex<HashSet<_>>` behind a `OnceLock`, with `new()` for tests.
/// It deliberately owns no reference to `Mutex<AgentEngine>` and is only ever
/// locked for a single `HashSet::insert` — never across an await and never while
/// an engine lock is being acquired — so it cannot participate in the engine
/// mutex's non-reentrant ordering.
#[derive(Default)]
struct AssumedContextWindowLog {
    reported: std::sync::Mutex<HashSet<String>>,
}

impl AssumedContextWindowLog {
    fn new() -> Self {
        Self::default()
    }

    /// True the first time this `(provider, model)` pair is seen, false
    /// afterwards. A poisoned lock reports `true`: a repeated diagnostic is
    /// strictly better than a silenced one, which is the bug being fixed.
    fn claim(&self, provider: &str, model: &str) -> bool {
        self.reported
            .lock()
            .map(|mut reported| reported.insert(format!("{provider}\u{1f}{model}")))
            .unwrap_or(true)
    }

    fn global() -> &'static Self {
        static GLOBAL: std::sync::OnceLock<AssumedContextWindowLog> = std::sync::OnceLock::new();
        GLOBAL.get_or_init(AssumedContextWindowLog::new)
    }
}

/// Whether a capability's `context_limit` leaves the engine running on the
/// resolved default window instead of the model's real one.
///
/// This is deliberately the same predicate as
/// `nomi_config::compact::resolve_context_window`, which also treats an explicit
/// `0` as unset. Anything that silently falls back must be reported.
fn assumes_default_context_window(context_limit: Option<u64>) -> bool {
    !matches!(context_limit, Some(limit) if limit > 0)
}

/// Report — once per provider+model — that the engine is compacting against an
/// assumed window rather than a declared one.
///
/// Without this line the failure mode is invisible: autocompact and the
/// emergency `ContextTooLong` guard are both calibrated to
/// `assumed_context_window`, so a model whose real window is smaller is rejected
/// by the provider with a non-retryable error before either can fire.
///
/// Returns whether a line was emitted, so the dedupe is observable in tests.
fn report_assumed_context_window(
    log: &AssumedContextWindowLog,
    context_limit: Option<u64>,
    provider: &str,
    model: &str,
    assumed_context_window: usize,
) -> bool {
    if !assumes_default_context_window(context_limit) || !log.claim(provider, model) {
        return false;
    }
    tracing::warn!(
        provider,
        model,
        assumed_context_window,
        "chat capability declares no context window; compacting against the assumed default. \
         Autocompact and the emergency context guard are calibrated to this value, so a model \
         with a smaller real window is rejected by the provider instead of compacted. Set \
         Context limit on this model's chat capability in Settings -> Models."
    );
    true
}

fn apply_provider_token_budget(
    config: &mut Config,
    context_limit: Option<u64>,
    declared_output_limit: Option<u32>,
) -> Result<(), AppError> {
    config.compact.context_window = nomi_config::compact::resolve_context_window(
        context_limit,
        config.compact.context_window,
    );
    report_assumed_context_window(
        AssumedContextWindowLog::global(),
        context_limit,
        &config.provider_label,
        &config.model,
        config.compact.context_window,
    );
    // The capability is authoritative on the desktop path. In particular,
    // None must erase a legacy ~/.nomi/config.toml value rather than revive it.
    config.output_max_tokens =
        nomi_config::compact::fit_context_budget(&mut config.compact, declared_output_limit);
    if config.output_max_tokens.is_none() && config.provider.requires_output_ceiling() {
        return Err(AppError::BadRequest(format!(
            "the {} protocol requires an explicit output ceiling; set Max output tokens on the {} chat capability in Settings -> Models",
            config.provider_label, config.model
        )));
    }
    Ok(())
}

struct TurnTeardownFence {
    pending: AtomicBool,
    changed: Notify,
}

/// A successor is never allowed to wait forever on cleanup owned by a prior
/// unwound turn. Exact teardown still owns the underlying resources, but this
/// bound turns a stuck fence into a structured broken-runtime failure so the
/// registry can quarantine/replace the manager instead of reusing it.
const TURN_TEARDOWN_FENCE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(7);

impl TurnTeardownFence {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            changed: Notify::new(),
        }
    }

    fn begin(&self) {
        self.pending.store(true, Ordering::Release);
    }

    fn complete(&self) {
        self.pending.store(false, Ordering::Release);
        self.changed.notify_waiters();
    }

    async fn wait_until_clear(&self, timeout: std::time::Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                let changed = self.changed.notified();
                if !self.pending.load(Ordering::Acquire) {
                    return;
                }
                changed.await;
            }
        })
        .await
        .is_ok()
    }
}

pub struct NomiAgentManager {
    runtime: AgentRuntimeState,
    backend_output_sink: Arc<BackendOutputSink>,
    engine: Mutex<AgentEngine>,
    /// Shared authority for every shell/tool process owned by this runtime.
    ///
    /// Kept outside the engine mutex so an explicit stop (which deliberately
    /// races an in-flight `execute_turn`) can fence process-tree teardown
    /// without first waiting for the turn to release the engine.
    process_supervisor: Option<Arc<nomi_process_runtime::ProcessSupervisor>>,
    /// Synchronous tombstone for an abnormal turn unwind. The old guard sets
    /// this before `send_message` can release `turn_gate`; a successor must not
    /// admit work until exact process cleanup and terminalization clear it.
    turn_teardown_fence: Arc<TurnTeardownFence>,
    /// Static slash command metadata captured at bootstrap so UI lookups do
    /// not wait behind an active `engine.execute_turn()` turn.
    slash_commands: Vec<SlashCommandItem>,
    /// Holds `Arc<McpManager>` instances alive for the duration of this agent's
    /// lifetime. The managers are not accessed after construction — they exist
    /// solely so their underlying MCP connections outlive the engine's event
    /// loop. Rust drops them here, in field-declaration order, after `engine`
    /// and `runtime` are dropped. See the explicit `Drop` impl below.
    #[allow(dead_code)] // intentional: lifetime-extension only; see Drop impl
    mcp_managers: Vec<Arc<McpManager>>,
    /// Main-process backstop for renewable loopback MCP capabilities. Bridge
    /// children revoke on clean exit; this guard covers abrupt child/runtime
    /// teardown and construction failure.
    loopback_capability_leases: nomifun_common::LoopbackCapabilityLeaseSet,
    /// Main-process `BrowserSessionHub` owner binding. The contained
    /// `BrowserLaneClient` grants this runtime scoped access without transferring
    /// Chromium/profile ownership. Explicit kill/revoke closes every Lane for
    /// this runtime; final Drop is the construction/abrupt-teardown backstop.
    #[cfg(feature = "browser-use")]
    browser_lane_binding: Option<crate::BrowserLaneBinding>,
    /// This runtime's claim on the pooled SSH link behind an SSH-bound session.
    /// Held for the runtime's whole life and released — never closed — at
    /// teardown; the link belongs to the conversation, which outlives every
    /// runtime the operator's model switches create and destroy.
    ssh_lease: Option<Arc<dyn crate::SshSessionLease>>,
    approval_manager: Arc<ToolApprovalManager>,
    confirmations: Arc<std::sync::RwLock<Vec<Confirmation>>>,
    /// Durable per-turn cancellation token. Unlike `Notify`, cancellation is
    /// retained when kill arrives before `send_message` reaches its select.
    turn_cancel: std::sync::Mutex<tokio_util::sync::CancellationToken>,
    active_turn: Arc<std::sync::Mutex<Option<crate::runtime_state::AgentRuntimeTurn>>>,
    /// Serializes turn admission, steering admission, terminal transition and
    /// permanent task shutdown. Terminal cleanup and `steer()` must share this
    /// exact gate: checking Running and pushing into the inbox as two separate
    /// operations lets a late interjection escape into the next explicit turn.
    lifecycle_gate: Arc<std::sync::Mutex<()>>,
    /// Holds for the complete send lifecycle. A second send waits here and
    /// re-checks `closing` only after the active turn has unwound, preventing
    /// it from replacing the active turn's cancellation token.
    turn_gate: Mutex<()>,
    /// Permanent once `kill` is requested; prevents a raced clone from
    /// admitting another turn after runtime-registry eviction.
    closing: AtomicBool,
    /// Mid-turn steering interjections pushed by `steer()` and drained by the
    /// engine at its loop boundaries. Shared (clone of this Arc handed to the
    /// engine via `set_steering_inbox` each turn). Entries belong exclusively
    /// to the current `active_turn`; terminal transition and the next turn's
    /// admission clear leftovers under `lifecycle_gate`, so an old generation
    /// can never be replayed by a later explicit turn. `std::sync::Mutex` —
    /// locked only for brief push/drain, never across an await.
    steering_inbox: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    /// Trusted host resource state. This queue is mounted on the engine for
    /// every turn but is injected only through the provider's top-level system
    /// context, never through a user message. It persists while the runtime is
    /// idle so notices do not need to start a synthetic turn.
    system_resource_inbox: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    /// Target directory for post-session memory distillation. `None` =
    /// this session never distills (companion red line, or no base dir).
    /// Set once at construction.
    distill_dir: Option<PathBuf>,
    /// Optional attachment-read boundary for restricted sessions. This mirrors
    /// the native file tools' `write_root`: channel/remote/public sessions are
    /// confined to their workspace, while a local desktop session (`None`)
    /// may choose any absolute local file through the OS file picker.
    image_read_root: Option<PathBuf>,
    /// Provider config snapshot reused by the exact post-turn distillation child.
    distill_cfg: Arc<nomi_config::config::Config>,
    /// One-shot knowledge reminder prepended to the FIRST user turn of a session
    /// that has bound bases — keeps the retrieval protocol adjacent to the task
    /// (the system-prompt section alone is too far from the user message to
    /// reliably fire). `None` once
    /// consumed or when no bases are mounted.
    knowledge_prelude: std::sync::Mutex<Option<String>>,
    /// When set, each user message is augmented with auto-retrieved KB hits
    /// (proactive RAG) keyed on the message text. `(retrieval sink, bound kb_ids)`.
    knowledge_auto_rag: Option<(
        Arc<dyn nomi_agent::knowledge_tools::KnowledgeRetrievalSink>,
        Vec<nomifun_common::KnowledgeBaseId>,
    )>,
    /// Refreshed from the authoritative local catalog before every admitted
    /// turn. The engine registry is changed under its mutex before this state is
    /// published, so route readiness and tool presence cannot disagree.
    image_generation_availability: std::sync::RwLock<ImageGenerationAvailability>,
    image_generation_discovery: Option<Arc<dyn ImageGenerationToolDiscovery>>,
    image_generation_response_in_chinese: bool,
}

impl Drop for NomiAgentManager {
    fn drop(&mut self) {
        self.backend_output_sink.cancel_active_tool_calls(
            "The agent manager was dropped before this tool call reached a terminal state.",
        );
        self.loopback_capability_leases.revoke_all();
        #[cfg(feature = "browser-use")]
        if let Some(binding) = &self.browser_lane_binding {
            binding.revoke();
        }
    }
}

/// Whether the knowledge_search tool should be registered for this session.
pub(crate) fn should_register_knowledge_search(
    has_sink: bool,
    kb_ids: &[nomifun_common::KnowledgeBaseId],
) -> bool {
    has_sink && !kb_ids.is_empty()
}

/// Whether the native knowledge_write (回血) tool should be registered: a
/// write-back sink was wired AND the session actually has bound bases to write
/// to. The factory only passes a sink when write-back is enabled on the
/// binding, so this also gates on the user's opt-in.
pub(crate) fn should_register_knowledge_write(
    has_sink: bool,
    bases: &[(nomifun_common::KnowledgeBaseId, String)],
) -> bool {
    has_sink && !bases.is_empty()
}

/// Tool name of the native knowledge write-back tool. Allow-listed past the
/// approval gate (DIRECT/STAGED writes go to the user's own managed base, and
/// companion/channel sessions have no confirmation UI), mirroring the companion
/// memory tools.
pub(crate) const KNOWLEDGE_WRITE_TOOL_NAME: &str = "knowledge_write";

/// Cap on race-tail re-runs within a single turn-claim. The race window is
/// sub-millisecond, so a tiny bound guarantees termination even if a steerer
/// pushes during every pass. Any leftover after the cap is absorbed by this
/// turn's terminal transition; steering is never transferred to another turn.
const MAX_STEERING_RACE_TAIL_RERUNS: usize = 3;

const IMAGE_MODEL_MANAGEMENT_LINK: &str = "nomifun://model-management/image";
const IMAGE_ROUTE_CONTEXT_KEY: &str = "nomifun.image_generation.route";
const IMAGE_ROUTE_NATIVE: &str = "native";
const IMAGE_ROUTE_EXTERNAL: &str = "explicit_external";
/// Browser screenshots and arbitrary website downloads are context, not a
/// durable generated-image artifact. Keep external execution closed until a
/// bridge can persist, verify, and commit those bytes through the same receipt
/// protocol as the native image tool.
const EXTERNAL_IMAGE_ARTIFACT_BRIDGE_AVAILABLE: bool = false;

fn image_route_from_context(value: Option<&str>) -> Option<ImageGenerationIntent> {
    match value {
        Some(IMAGE_ROUTE_NATIVE) => Some(ImageGenerationIntent::Creation),
        Some(IMAGE_ROUTE_EXTERNAL) => Some(ImageGenerationIntent::ExplicitExternal),
        _ => None,
    }
}

fn image_route_context_value(intent: ImageGenerationIntent) -> Option<&'static str> {
    match intent {
        ImageGenerationIntent::Creation => Some(IMAGE_ROUTE_NATIVE),
        ImageGenerationIntent::ExplicitExternal => Some(IMAGE_ROUTE_EXTERNAL),
        ImageGenerationIntent::Discussion | ImageGenerationIntent::None => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageGenerationAvailability {
    Ready,
    NoConfiguredModel,
    DiscoveryFailed,
    NotEntitled,
}

fn image_generation_unavailable_message(
    availability: ImageGenerationAvailability,
    respond_in_chinese: bool,
) -> String {
    match (availability, respond_in_chinese) {
        (ImageGenerationAvailability::NoConfiguredModel, true) => format!(
            "尚未配置可用的生图模型。请先前往[模型管理 · 图片模型]({IMAGE_MODEL_MANAGEMENT_LINK})，启用并完整配置一个支持图片生成的模型。此次没有生成图片，也没有调用浏览器或第三方生图网站。"
        ),
        (ImageGenerationAvailability::NoConfiguredModel, false) => format!(
            "No usable image-generation model is configured. Open [Model Management · Image models]({IMAGE_MODEL_MANAGEMENT_LINK}) and enable a fully configured image-generation model. No image was generated, and no browser or third-party image site was used."
        ),
        (ImageGenerationAvailability::DiscoveryFailed, true) =>
            "暂时无法核验本地生图模型配置。请刷新 Agent 会话后重试；此次没有生成图片，也没有调用浏览器或第三方生图网站。".to_owned(),
        (ImageGenerationAvailability::DiscoveryFailed, false) =>
            "The local image-model configuration could not be verified. Retry on the next turn; this capability refreshes automatically. No image was generated, and no browser or third-party image site was used.".to_owned(),
        (ImageGenerationAvailability::NotEntitled, true) =>
            "当前受限 Agent 会话没有生图权限。请在完整的本地会话中重试，或由会话所有者开放原生生图能力。此次没有生成图片，也没有调用浏览器或第三方生图网站。".to_owned(),
        (ImageGenerationAvailability::NotEntitled, false) =>
            "Image generation is not permitted in this restricted Agent session. Retry in a full local session or ask the session owner to enable the native capability. No image was generated, and no browser or third-party image site was used.".to_owned(),
        (ImageGenerationAvailability::Ready, _) =>
            "Image generation is available.".to_owned(),
    }
}

fn external_image_generation_unavailable_message(
    availability: ImageGenerationAvailability,
    respond_in_chinese: bool,
) -> String {
    let bridge_message = if respond_in_chinese {
        "当前版本尚未提供可将浏览器或第三方网站生成结果持久化并核验为图片制品的通道，因此此次没有打开浏览器，也没有开始外部生图。"
    } else {
        "This version cannot yet persist and verify an image generated by a browser or third-party website, so no browser was opened and no external image generation was started."
    };
    if availability == ImageGenerationAvailability::Ready {
        return if respond_in_chinese {
            format!("{bridge_message}请改为直接要求使用已配置的原生生图模型。")
        } else {
            format!(
                "{bridge_message} Ask to use the configured native image model instead."
            )
        };
    }
    format!(
        "{bridge_message} {}",
        image_generation_unavailable_message(availability, respond_in_chinese)
    )
}

fn ambiguous_visual_clarification_message(
    availability: ImageGenerationAvailability,
    respond_in_chinese: bool,
) -> String {
    let clarification = if respond_in_chinese {
        "我无法可靠判断你是否希望现在生成一张新图片。请明确说明“生成/绘制一张图片”及画面要求。"
    } else {
        "I could not reliably determine whether you want a new image now. Please explicitly ask to generate or draw an image and include the visual requirements."
    };
    if availability == ImageGenerationAvailability::Ready {
        return if respond_in_chinese {
            format!(
                "{clarification}此次没有生成图片，也没有调用浏览器或第三方生图网站。"
            )
        } else {
            format!(
                "{clarification} No image was generated, and no browser or third-party image site was used."
            )
        };
    }
    format!(
        "{clarification} {}",
        image_generation_unavailable_message(availability, respond_in_chinese)
    )
}

/// Context-only edits such as "make it 16:9" are image requests only when the
/// immediately preceding accepted turn was itself image generation. Keeping
/// this state in the serialized manager turn lane avoids both Browser escape
/// on real follow-ups and false positives such as a standalone "one more".
fn contextual_image_generation_followup(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    const HINTS: &[&str] = &[
        "再来一张",
        "再来一个",
        "再来个",
        "换成",
        "改成",
        "按刚才",
        "同样风格",
        "相同风格",
        "重新做",
        "重做",
        "竖版",
        "横版",
        "another one",
        "one more",
        "make it",
        "change it",
        "same style",
        "redo it",
        "portrait version",
        "landscape version",
    ];
    HINTS.iter().any(|hint| normalized.contains(hint))
}

fn is_context_only_image_followup(input: &str) -> bool {
    if !contextual_image_generation_followup(input) {
        return false;
    }
    const EXPLICIT_TERMS: &[&str] = &[
        "生成", "创建", "创作", "绘制", "图片", "图像", "插画", "海报", "照片", "头像",
        "壁纸", "封面", "图标", "漫画", "generate", "create", "draw", "render", "image",
        "picture", "photo", "poster", "illustration", "wallpaper", "avatar", "artwork", "logo",
    ];
    let normalized = input.to_lowercase();
    !EXPLICIT_TERMS.iter().any(|term| normalized.contains(term))
}

fn image_followup_route(
    direct: ImageGenerationIntent,
    input: &str,
    prior: Option<ImageGenerationIntent>,
) -> Option<ImageGenerationIntent> {
    if direct == ImageGenerationIntent::Creation && is_context_only_image_followup(input) {
        return prior;
    }
    if direct == ImageGenerationIntent::None && contextual_image_generation_followup(input) {
        return prior;
    }
    None
}

/// The model may recognize creation intent, but it can never grant Browser
/// authority. External execution requires an independent affirmative host
/// check over the current user text; otherwise the safe interpretation is the
/// native image route.
fn host_validated_image_intent(
    classified: ImageGenerationIntent,
    direct: ImageGenerationIntent,
    input: &str,
    prior: Option<ImageGenerationIntent>,
    plan_mode_active: bool,
) -> ImageGenerationIntent {
    if plan_mode_active {
        return ImageGenerationIntent::None;
    }
    if is_code_native_visual_request(input) {
        // Canvas, SVG, Mermaid, UI icons, and chart/diagram source are coding
        // outputs. A semantic classifier may not upgrade them into a billable
        // raster-image operation merely because the request contains 图/画 or
        // an English visual noun.
        return ImageGenerationIntent::None;
    }
    if direct == ImageGenerationIntent::None && explicitly_requests_visual_discussion(input) {
        // The semantic pass may recognize long-tail creation wording, but it
        // may not upgrade an explicit host-recognized explanation, analysis,
        // comparison, or inspection request into a billable image operation.
        return ImageGenerationIntent::None;
    }
    if let Some(route) = image_followup_route(direct, input, prior) {
        return route;
    }
    match classified {
        ImageGenerationIntent::ExplicitExternal
            if direct == ImageGenerationIntent::ExplicitExternal
                || explicitly_requests_external_image_execution(input) =>
        {
            ImageGenerationIntent::ExplicitExternal
        }
        ImageGenerationIntent::ExplicitExternal => ImageGenerationIntent::Creation,
        ImageGenerationIntent::Creation => ImageGenerationIntent::Creation,
        ImageGenerationIntent::Discussion | ImageGenerationIntent::None => {
            ImageGenerationIntent::None
        }
    }
}

fn contains_ascii_word(input: &str, candidates: &[&str]) -> bool {
    input
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .any(|word| candidates.contains(&word))
}

/// Code-native visuals belong to the coding agent. Requiring both a coding
/// output signal and a visual/source signal avoids treating ordinary prose
/// containing a single 图 or 画 as image-generation intent.
fn is_code_native_visual_request(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }

    const CN_CODE_OUTPUTS: &[&str] = &[
        "代码",
        "源码",
        "组件",
        "编程",
        "编码",
        "开发",
        "实现",
        "重构",
        "修复",
        "合并需求",
        "项目助手",
    ];
    const CN_CODE_VISUALS: &[&str] = &[
        "画布", "图表", "流程图", "架构图", "时序图", "关系图", "图标", "界面", "图形",
        "可视化", "svg", "mermaid", "canvas", "ui",
    ];
    if CN_CODE_OUTPUTS
        .iter()
        .any(|signal| normalized.contains(signal))
        && CN_CODE_VISUALS
            .iter()
            .any(|signal| normalized.contains(signal))
    {
        return true;
    }

    const EN_CODE_OUTPUTS: &[&str] = &[
        "code",
        "coding",
        "source",
        "component",
        "components",
        "program",
        "programming",
        "implement",
        "implementation",
        "refactor",
        "debug",
        "fix",
        "module",
        "function",
        "class",
        "typescript",
        "javascript",
        "react",
        "vue",
        "html",
        "css",
    ];
    const EN_CODE_VISUALS: &[&str] = &[
        "canvas",
        "chart",
        "charts",
        "diagram",
        "diagrams",
        "flowchart",
        "flowcharts",
        "icon",
        "icons",
        "image",
        "images",
        "graphic",
        "graphics",
        "visual",
        "visuals",
        "ui",
        "svg",
        "mermaid",
    ];
    contains_ascii_word(&normalized, EN_CODE_OUTPUTS)
        && contains_ascii_word(&normalized, EN_CODE_VISUALS)
}

fn has_ambiguous_visual_signal(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    if normalized.is_empty() || is_code_native_visual_request(&normalized) {
        return false;
    }
    const CN_ACTIONS: &[&str] = &[
        "生成", "创建", "创作", "绘制", "设计", "制作", "做一张", "来一张", "给我", "弄一张",
        "出一张", "画一张", "画个", "重绘", "渲染", "惊喜",
    ];
    const CN_VISUAL_OBJECTS: &[&str] = &[
        "图片", "图像", "视觉", "美术", "艺术", "配图", "海报", "封面", "头像", "壁纸", "插画",
        "照片", "图标", "徽标", "画面", "画作", "图稿", "漫画", "概念图", "效果图",
    ];
    if CN_ACTIONS
        .iter()
        .any(|action| normalized.contains(action))
        && CN_VISUAL_OBJECTS
            .iter()
            .any(|object| normalized.contains(object))
    {
        return true;
    }
    const EN_ACTIONS: &[&str] = &[
        "give", "show", "surprise", "make", "create", "generate", "draw", "render", "design",
        "produce", "paint", "want", "need",
    ];
    const EN_VISUAL_OBJECTS: &[&str] = &[
        "image", "images", "picture", "pictures", "visual", "visuals", "visually", "art",
        "artwork", "artworks", "graphic", "graphics", "illustration", "illustrations", "photo",
        "photos", "poster", "posters", "banner", "banners", "logo", "logos", "icon", "icons",
        "wallpaper", "wallpapers", "avatar", "avatars", "render", "rendering", "draw", "drawing",
        "paint", "painting", "sketch", "sketches", "thumbnail", "thumbnails", "sprite", "sprites",
    ];
    contains_ascii_word(&normalized, EN_ACTIONS)
        && contains_ascii_word(&normalized, EN_VISUAL_OBJECTS)
}

fn attachment_suggests_visual_creation(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    if is_code_native_visual_request(&normalized) {
        return false;
    }
    const CN_ACTIONS: &[&str] = &[
        "生成", "画", "绘", "做", "重做", "改成", "换成", "变成", "风格化", "修图", "重绘",
        "去背景", "换背景",
    ];
    if CN_ACTIONS.iter().any(|action| normalized.contains(action)) {
        return true;
    }
    const EN_ACTIONS: &[&str] = &[
        "make", "create", "generate", "draw", "render", "edit", "transform", "convert",
        "stylize", "restyle", "redo", "change", "remove", "replace", "add",
    ];
    normalized
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .any(|word| EN_ACTIONS.contains(&word))
}

fn explicitly_requests_visual_discussion(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    const CN_DISCUSSION: &[&str] = &[
        "解释", "说明", "讨论", "分析", "比较", "对比", "描述", "评价", "点评", "审查", "为什么",
        "如何", "怎么", "什么是", "识别", "读取", "提取",
    ];
    if CN_DISCUSSION
        .iter()
        .any(|phrase| normalized.contains(phrase))
    {
        return true;
    }
    const EN_DISCUSSION_PHRASES: &[&str] = &[
        "tell me about",
        "tell me what",
        "what is",
        "what's",
        "why is",
        "how does",
        "how do",
    ];
    if EN_DISCUSSION_PHRASES
        .iter()
        .any(|phrase| normalized.contains(phrase))
    {
        return true;
    }
    const EN_DISCUSSION_WORDS: &[&str] = &[
        "discuss", "explain", "analyze", "analyse", "compare", "describe", "review", "critique",
        "inspect", "identify", "read", "extract",
    ];
    normalized
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .any(|word| EN_DISCUSSION_WORDS.contains(&word))
}

fn should_run_image_intent_model(
    direct: ImageGenerationIntent,
    prior: Option<ImageGenerationIntent>,
    input: &str,
    has_attachments: bool,
    plan_mode_active: bool,
) -> bool {
    !plan_mode_active
        && !is_code_native_visual_request(input)
        && direct == ImageGenerationIntent::None
        && image_followup_route(direct, input, prior).is_none()
        // The broad signal only decides whether to spend a small semantic
        // classification call; it never decides the route. This keeps clearly
        // unrelated chat single-call while creation wording outside the strict
        // host shortcuts (including when no image model is configured) still
        // reaches the typed conversation-model decision.
        && (has_ambiguous_visual_signal(input)
            || (has_attachments && attachment_suggests_visual_creation(input)))
}

fn route_allows_knowledge_context(tool_allowlist: Option<&HashSet<String>>) -> bool {
    tool_allowlist.is_none_or(|allowed| allowed.contains("knowledge_search"))
}

/// Atomically close one exact manager turn and absorb every steering entry
/// that was admitted for it.
///
/// `AgentRuntimeState` already fences terminal events by `AgentRuntimeTurn`,
/// but the steering queue lives outside that state. Without this outer gate a
/// steerer can observe Running, lose the race to `terminal`, and then enqueue
/// after the engine's final drain. The next explicit turn would mount the same
/// queue and execute stale work. Holding `lifecycle_gate` across the exact
/// active-turn check, runtime terminal transition, queue clear and owner
/// release makes that outcome impossible.
fn terminalize_exact_nomi_turn(
    runtime: &AgentRuntimeState,
    lifecycle_gate: &Arc<std::sync::Mutex<()>>,
    active_turn: &Arc<std::sync::Mutex<Option<crate::runtime_state::AgentRuntimeTurn>>>,
    steering_inbox: &Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    turn: crate::runtime_state::AgentRuntimeTurn,
    terminal: impl FnOnce(
        &AgentRuntimeState,
        crate::runtime_state::AgentRuntimeTurn,
    ) -> bool,
) -> bool {
    let _lifecycle = lifecycle_gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut active = active_turn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if active.as_ref() != Some(&turn) {
        return false;
    }

    let emitted = terminal(runtime, turn);
    steering_inbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    *active = None;
    emitted
}

/// Prepend the one-shot knowledge prelude to the first user turn, if present.
pub(crate) fn apply_knowledge_prelude(prelude: Option<String>, content: &str) -> String {
    match prelude {
        Some(p) if !p.is_empty() => format!("{p}\n\n{content}"),
        _ => content.to_owned(),
    }
}

/// Prepend auto-retrieved knowledge-base hits to the user's message so the model
/// has relevant domain context without first having to call `knowledge_search`
/// (proactive RAG). Pure; returns `content` unchanged when there are no hits.
pub(crate) fn prepend_knowledge_context(
    hits: &[nomi_agent::knowledge_tools::KnowledgeHit],
    content: String,
) -> String {
    if hits.is_empty() {
        return content;
    }
    let mut block = String::from(
        "[Relevant knowledge-base context, retrieved automatically for this message \
         — to open a full document, call knowledge_read with the exact opaque handle shown below; \
         copy the handle unchanged and do not rebuild it from the path:]\n",
    );
    for h in hits {
        block.push_str(&format!(
            "- {}/{} § {}\n  {}\n  handle: {}\n",
            h.kb_name, h.rel_path, h.heading, h.snippet, h.handle
        ));
    }
    format!("{block}\n{content}")
}

/// Normalize the engine's [`StopReason`](nomi_types::message::StopReason) into
/// the cross-backend [`TurnStopReason`] carried on `TurnCompleted` / `Finish`.
/// `EndTurn`/`ToolUse` are clean completions; ceilings, request exhaustion and
/// refusal remain explicit non-success terminals for AutoWork / IDMM.
pub(crate) fn map_engine_stop_reason(
    reason: nomi_types::message::StopReason,
) -> TurnStopReason {
    use nomi_types::message::StopReason;
    match reason {
        StopReason::EndTurn | StopReason::ToolUse => TurnStopReason::EndTurn,
        StopReason::MaxTokens => TurnStopReason::MaxTokens,
        StopReason::MaxTurns => TurnStopReason::MaxTurnRequests,
        StopReason::Refusal => TurnStopReason::Refusal,
    }
}

/// Process-host wiring kept deliberately separate from [`NomiResolvedConfig`].
///
/// None of these values can be deserialized from model/config JSON. The app
/// factory constructs this bundle only after resolving trusted runtime
/// ownership.
pub(crate) struct NomiHostWiring {
    #[cfg(feature = "browser-use")]
    pub browser_lane_binding: Option<crate::BrowserLaneBinding>,
    /// A ready remote backend when the session is SSH-bound (the factory already
    /// connected it via the SshBackendProvider). Selects the remote tool family.
    pub ssh_backend: Option<Arc<dyn crate::SshBackend>>,
    /// This runtime's claim on that remote session. Retained (not moved into the
    /// engine) so teardown has something to report on: a runtime that dropped its
    /// lease could never say whether the operator's shell survived it.
    pub ssh_lease: Option<Arc<dyn crate::SshSessionLease>>,
    /// Native image generation is a trusted process capability assembled from
    /// the live model catalog. `None` means no enabled, fully configured image
    /// model exists, so the provider must never see an `image_gen` schema.
    pub image_generation_tool: Option<Box<dyn nomi_tools::Tool>>,
    /// Retained local discovery authority. Entitled runtimes use it before
    /// every turn so catalog/default/connection edits do not require teardown.
    pub image_generation_discovery: Option<Arc<dyn ImageGenerationToolDiscovery>>,
    /// Whether this process-owned principal may receive the native capability.
    /// Kept separate from tool presence so a restricted session never lies
    /// that the installation has no configured model.
    pub image_generation_entitled: bool,
    /// Catalog/repository failures are not equivalent to an empty catalog.
    pub image_generation_discovery_failed: bool,
    /// The app's normalized UI language captured on this runtime build.
    pub image_generation_response_in_chinese: bool,
}

impl Default for NomiHostWiring {
    fn default() -> Self {
        Self {
            #[cfg(feature = "browser-use")]
            browser_lane_binding: None,
            ssh_backend: None,
            ssh_lease: None,
            image_generation_tool: None,
            image_generation_discovery: None,
            image_generation_entitled: true,
            image_generation_discovery_failed: false,
            image_generation_response_in_chinese: false,
        }
    }
}

/// Wiring for a summoned-companion work session (spec §设计 B2/B3):
/// read-only recall over the summoned companion's memories plus the per-turn
/// live memory-snapshot contributor. Every write path is deliberately absent:
/// `save_memory` is never registered under summon, the memory sink refuses
/// writes, and the confirmation-style `propose_companion_memory` tool retired
/// with the 建议 feature.
pub struct NomiSummonWiring {
    pub memory_sink: Arc<dyn CompanionMemorySink>,
    pub context_sink: Arc<dyn SummonContextSink>,
}

impl NomiAgentManager {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        conversation_id: String,
        workspace: String,
        config_extra: NomiResolvedConfig,
        resume_session: Option<Session>,
        requirement_sink: Option<Arc<dyn RequirementSink>>,
        companion_sink: Option<Arc<dyn CompanionMemorySink>>,
        knowledge_retrieval_sink: Option<
            Arc<dyn nomi_agent::knowledge_tools::KnowledgeRetrievalSink>,
        >,
        knowledge_kb_ids: Vec<nomifun_common::KnowledgeBaseId>,
        knowledge_prelude: Option<String>,
        knowledge_writeback_sink: Option<
            Arc<dyn nomi_agent::knowledge_tools::KnowledgeWritebackSink>,
        >,
        knowledge_write_bases: Vec<(nomifun_common::KnowledgeBaseId, String)>,
        companion_skill_sink: Option<Arc<dyn CompanionSkillSink>>,
    ) -> Result<Self, AppError> {
        Self::new_with_host_wiring(
            conversation_id,
            workspace,
            config_extra,
            resume_session,
            requirement_sink,
            companion_sink,
            knowledge_retrieval_sink,
            knowledge_kb_ids,
            knowledge_prelude,
            knowledge_writeback_sink,
            knowledge_write_bases,
            companion_skill_sink,
            None,
            None,
            NomiHostWiring::default(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn new_with_host_wiring(
        conversation_id: String,
        workspace: String,
        config_extra: NomiResolvedConfig,
        resume_session: Option<Session>,
        requirement_sink: Option<Arc<dyn RequirementSink>>,
        companion_sink: Option<Arc<dyn CompanionMemorySink>>,
        knowledge_retrieval_sink: Option<Arc<dyn nomi_agent::knowledge_tools::KnowledgeRetrievalSink>>,
        knowledge_kb_ids: Vec<nomifun_common::KnowledgeBaseId>,
        knowledge_prelude: Option<String>,
        knowledge_writeback_sink: Option<Arc<dyn nomi_agent::knowledge_tools::KnowledgeWritebackSink>>,
        knowledge_write_bases: Vec<(nomifun_common::KnowledgeBaseId, String)>,
        companion_skill_sink: Option<Arc<dyn CompanionSkillSink>>,
        computer_history_sink: Option<Arc<dyn nomi_agent::computer_history_tools::ComputerHistorySink>>,
        summon_wiring: Option<NomiSummonWiring>,
        host_wiring: NomiHostWiring,
    ) -> Result<Self, AppError> {
        let runtime = AgentRuntimeState::new(conversation_id.clone(), workspace.clone(), 128);
        let loopback_capability_leases = config_extra.loopback_capability_leases.clone();
        #[cfg(feature = "browser-use")]
        let browser_lane_binding = host_wiring.browser_lane_binding;
        let ssh_lease = host_wiring.ssh_lease;
        let image_generation_entitled = host_wiring.image_generation_entitled;
        let image_generation_discovery = host_wiring.image_generation_discovery;
        let image_generation_tool = host_wiring.image_generation_tool;
        let image_generation_availability = if image_generation_tool.is_some() {
            ImageGenerationAvailability::Ready
        } else if host_wiring.image_generation_discovery_failed {
            ImageGenerationAvailability::DiscoveryFailed
        } else if host_wiring.image_generation_entitled {
            ImageGenerationAvailability::NoConfiguredModel
        } else {
            ImageGenerationAvailability::NotEntitled
        };
        let image_generation_response_in_chinese =
            host_wiring.image_generation_response_in_chinese;
        let image_read_root = config_extra
            .write_root
            .as_deref()
            .map(str::trim)
            .filter(|root| !root.is_empty())
            .map(PathBuf::from);

        // Companion red line: companion-companion sessions (companion_sink present)
        // NEVER distill into file-based memory — their persona memory belongs
        // to the companion SQLite store + learner. Otherwise the target is the
        // project-level auto-memory dir (same resolution as the engine's
        // bootstrap `auto_memory_dir(cwd)`). A run-time origin check in
        // `send_message` is the second gate (cron/autowork/idmm turns).
        let distill_dir: Option<PathBuf> = if companion_sink.is_some() {
            None
        } else {
            nomi_memory::paths::auto_memory_dir(std::path::Path::new(&workspace))
        };

        let backend_output_sink = Arc::new(
            BackendOutputSink::new(runtime.event_sender())
                .with_distill_dir(distill_dir.clone())
                .with_artifact_workspace(&workspace),
        );
        let sink: Arc<dyn OutputSink> = backend_output_sink.clone();

        let cli_args = CliArgs {
            provider: Some(config_extra.provider.clone()),
            api_key: Some(config_extra.api_key.clone()),
            base_url: config_extra.base_url.clone(),
            model: Some(config_extra.model.clone()),
            // Capability data is applied authoritatively below. Do not let a
            // process-local CLI/TOML value become this model's hidden ceiling.
            max_tokens: None,
            max_turns: config_extra.max_turns,
            system_prompt: config_extra.system_prompt.clone(),
            profile: None,
            auto_approve: config_extra.session_mode.as_deref() == Some("yolo"),
            project_dir: Some(PathBuf::from(&workspace)),
        };

        let mut config =
            Config::resolve(&cli_args).map_err(|e| AppError::Internal(format!("Config resolve failed: {e}")))?;

        // Backend-specific overrides
        config.bedrock = config_extra.bedrock_config;
        config.session.enabled = true;
        config.session.directory = config_extra.session_directory.to_string_lossy().into_owned();

        if let Some(field) = config_extra.compat_overrides.max_tokens_field {
            config.compat.max_tokens_field = Some(field);
        }
        if let Some(path) = config_extra.compat_overrides.api_path {
            config.compat.api_path = Some(path);
        }
        if let Some(required) = config_extra.compat_overrides.require_reasoning_content {
            config.compat.require_reasoning_content = Some(required);
        }
        if let Some(chain_rounds) = config_extra.compat_overrides.chain_rounds {
            config.compat.chain_rounds = Some(chain_rounds);
        }
        config.compat.extra_body = config_extra.compat_overrides.extra_body;
        // 图片支持 override(主动剔除):工厂据 VisionUnsupportedRegistry 命中注入
        // Some(false),灌进 compat.supports_image → build_messages 发送时剔图。
        // None → 保持 Config::resolve 的默认(supports_image()==true),行为不变。
        if let Some(supports_image) = config_extra.compat_overrides.supports_image {
            config.compat.supports_image = Some(supports_image);
        }

        // Make the engine compact against the provider's declared context
        // window when set (else keep the resolved default). Same value the
        // context-usage gauge reports as the denominator.
        apply_provider_token_budget(
            &mut config,
            config_extra.context_limit,
            config_extra.output_ceiling,
        )?;

        if !config_extra.extra_mcp_servers.is_empty() {
            config.mcp.servers.extend(config_extra.extra_mcp_servers.clone());
        }

        // Session-level opt-in for desktop/browser automation tools. The
        // bootstrap registers them only when these flags are set.
        if config_extra.computer_use {
            config.tools.computer.enabled = true;
        }
        if config_extra.browser_use {
            config.tools.browser.enabled = true;
        }
        // Per-session 工具白名单（工厂已算好；bootstrap 的 retain_named
        // 会安装持久注册策略，后续 post-build / dynamic 工具也受同一策略约束）。
        // Embedded AgentExecution 的 host composition 不写入 ToolsConfig，
        // 而是在 bootstrap builder 上单独注入。
        config.tools.builtin_allowlist = config_extra.allowed_tools.clone();
        // 原生文件工具写根钳制（Write/Edit/ApplyPatch），按会话信任面由工厂解析：
        // 本地桌面 = None（不钳制，OS 用户全权，今日行为）；渠道/远程/对外 =
        // Some(workspace)（收窄到会话工作区）。仅在有非空值时覆盖，故桌面会话保留
        // Config::resolve 的默认（空 = 不钳制），且用户在 config.toml 里显式设置的
        // write_root 不被无谓清空。与 gateway file-service 的 PathAuthority 同源。
        if let Some(root) = config_extra.write_root.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            config.tools.write_root = root.to_owned();
        }
        // F1-sec: 把会话的 evaluate「全权模式」LIVE 值灌进 BrowserConfig.full_power。
        // Hub-backed Browser tool adapter 在调用 BrowserLaneClient 前执行 evaluate gate；
        // Hub 仍是最终权威。默认 false（default-deny）。
        config.tools.browser.full_power = config_extra.browser_full_power;
        // SD-6: 把会话的持久登录 LIVE 值灌进 BrowserConfig.persistent_login，供
        // Hub-backed Browser tool adapter 执行 evaluate 互斥门。Primary 实时身份由
        // BrowserSessionHub 的应用管理 profile 提供。产品默认 true（由 factory host_default 实现）。
        config.tools.browser.persistent_login = config_extra.browser_persistent_login;
        // P7A: 把会话的 site-memory LIVE 值灌进 BrowserConfig.site_memory（默认 OFF，opt-in）。
        // bootstrap 据它给 Hub-backed Browser tool adapter 注入文件型 SiteMemorySink。
        config.tools.browser.site_memory = config_extra.browser_site_memory;
        // P7B: 把会话的 visual-fallback LIVE 值灌进 BrowserConfig.visual_fallback（默认 OFF，opt-in）。
        // bootstrap 据它给 Hub-backed Browser tool adapter 注入会话模型的 VisualLocator。
        config.tools.browser.visual_fallback = config_extra.browser_visual_fallback;
        config.tools.browser.unrestricted_approval = config_extra.browser_unrestricted_approval;
        // Browser is status-only: Primary runs in the external managed window.
        // BrowserSessionHub owns every Host/profile; this runtime never
        // creates a private/headless one.
        config.tools.browser.headless = false;
        config.tools.browser.source = config_extra.browser_source.clone();

        // Companion memory tools only touch the companion's own memory.db — never
        // user files — so they skip the approval gate in every session mode
        // (Default mode auto-approves nothing by category, which would park
        // every save_memory call on a confirmation the companion bubble can't show).
        if companion_sink.is_some() {
            config.tools.allow_list.extend([
                "recall_memories".to_owned(),
                "save_memory".to_owned(),
                "list_recent_events".to_owned(),
            ]);
        }
        // Summoned-companion work session (spec §设计 B): the read-only recall
        // touches only the companion's own memory.db — never user files — so it
        // skips the approval gate like the companion tools above. `save_memory`
        // is intentionally NOT allow-listed and never registered under summon.
        // Must be set BEFORE bootstrap; registration happens after build().
        if summon_wiring.is_some() {
            config.tools.allow_list.push("recall_memories".to_owned());
        }
        // Companion self-evolved skill invocation (yolo, no approval UI) — must be
        // allow-listed BEFORE bootstrap or the call parks forever. Registration of
        // the tool + the per-turn skill ContextContributor happens after build().
        if companion_skill_sink.is_some() {
            config.tools.allow_list.push("companion_skill".to_owned());
        }

        // Read-only computer-history tools (status/recent/apps/urls/find_chats/
        // count_activity/get_settings) only read the local history store and
        // bypass the approval lane like the knowledge tools above; the write
        // tools (pause/resume/update_settings) stay behind the normal approval
        // gate. Must be set BEFORE bootstrap so the advertised prompt and the
        // later registration cannot drift.
        if computer_history_sink.is_some() {
            for tool_name in [
                "computer_history_recent",
                "computer_history_apps",
                "computer_history_urls",
                "computer_history_find_chats",
                "computer_history_count_activity",
                "computer_history_status",
                "computer_history_get_settings",
            ] {
                if !config.tools.allow_list.iter().any(|name| name == tool_name) {
                    config.tools.allow_list.push(tool_name.to_owned());
                }
            }
        }

        // Read-only knowledge tools must bypass the approval lane whenever the
        // same sink/base truth permits their later registration. Do this before
        // bootstrap so the engine policy and the advertised prompt cannot drift.
        let register_knowledge_search = should_register_knowledge_search(
            knowledge_retrieval_sink.is_some(),
            &knowledge_kb_ids,
        );
        if register_knowledge_search {
            for tool_name in ["knowledge_search", "knowledge_read"] {
                if !config.tools.allow_list.iter().any(|name| name == tool_name) {
                    config.tools.allow_list.push(tool_name.to_owned());
                }
            }
        }

        // The native knowledge_write (回血) tool writes only into the user's own
        // bound knowledge base (DIRECT → base body; STAGED → review inbox) via
        // the backend service. Allow-list it so it bypasses the per-call approval
        // gate — under SessionMode::Default nothing is auto-approved, which would
        // park every write-back on a confirmation many surfaces (channel /
        // companion) cannot even show. Same posture as the companion memory
        // tools above. Must be set BEFORE bootstrap so it reaches the engine's
        // allow_list. Registration of the tool itself happens after build().
        let register_knowledge_write =
            should_register_knowledge_write(knowledge_writeback_sink.is_some(), &knowledge_write_bases);
        if register_knowledge_write {
            config.tools.allow_list.push(KNOWLEDGE_WRITE_TOOL_NAME.to_owned());
        }
        // The native image tool writes only through the verified artifact
        // store and is the expected path for an ordinary generation request.
        // Add it before bootstrap so the registry's persistent allow policy
        // accepts the late registration and the turn never stalls on an
        // approval UI while a provider request is already in flight.
        if image_generation_entitled
            && !config
                .tools
                .allow_list
                .iter()
                .any(|name| name == IMAGE_GEN_TOOL_NAME)
        {
            config
                .tools
                .allow_list
                .push(IMAGE_GEN_TOOL_NAME.to_owned());
        }

        let is_resume = resume_session.is_some();
        let provider_label = config.provider_label.clone();
        let goal_spec = config_extra.goal.clone();

        // Snapshot the resolved provider config for the exact distillation child
        // (the engine consumes `config` next). Cheap one-time clone.
        let distill_cfg = Arc::new(config.clone());

        // Create the session's shared approval manager before bootstrap. The
        // Hub-backed Browser tool adapter and ordinary tool execution receive
        // the same Arc, so a mid-session mode change updates the live redline
        // gate instead of using a construction-time snapshot. This policy
        // wiring does not own a Browser Host; all browser work still enters the
        // main-process BrowserSessionHub through BrowserLaneClient.
        let approval_manager = Arc::new(ToolApprovalManager::new());
        if let Some(mode_str) = &config_extra.session_mode {
            let mode = parse_session_mode(mode_str);
            approval_manager.set_mode(mode);
            info!(
                conversation_id = %conversation_id,
                session_mode = mode_str,
                "Nomi initial session mode applied"
            );
        }

        // Phase D: the session's confirmation store, created BEFORE bootstrap so the desktop
        // approval gate can share the SAME Arc the `BackendProtocolSink` (below) uses. Holds
        // pending tool-approvals + browser takeover/egress approvals; the frontend renders
        // them (MessagePermission) and resolves via `confirm`.
        let confirmations = Arc::new(std::sync::RwLock::new(Vec::new()));

        let mut bootstrap = AgentBootstrap::new(config, &workspace, sink)
            .goal(goal_spec)
            .install_embedded_agent_execution(
                config_extra.install_embedded_agent_execution,
            )
            .approval_manager(approval_manager.clone());
        if let Some(key) = config_extra.persistent_login_key {
            bootstrap = bootstrap.persistent_login_key(key);
        }
        // Phase D: when the user opted into takeover/approval (`agent.browserUse.takeover`),
        // give bootstrap a desktop approval gate sharing the session's confirmation store +
        // approval manager — it surfaces irreversible actions / gated cross-origin POSTs
        // (SD-5) to the user via the existing confirmation UI and awaits a decision. Absent →
        // fail-closed (irreversible stays Blocked, gated egress fails). Threaded into the
        // Hub-backed Browser tool adapter (no-op if browser-use is off).
        #[cfg(feature = "browser-use")]
        if should_install_browser_approval_gate(
            config_extra.browser_takeover,
            config_extra.browser_unrestricted_approval,
            approval_manager.as_ref(),
        ) {
            let gate = crate::manager::nomi::browser_approval::DesktopApprovalGate::new(
                runtime.event_sender(),
                confirmations.clone(),
                approval_manager.clone(),
                config_extra.browser_unrestricted_approval,
            );
            bootstrap = bootstrap.approval_gate(Arc::new(gate));
        }
        #[cfg(feature = "browser-use")]
        if let Some(binding) = &browser_lane_binding {
            // The runtime receives only the scoped client. Chromium, profiles,
            // Host restart, and Lane inventory remain owned by BrowserSessionHub.
            bootstrap = bootstrap.browser_lane_client(binding.client());
        }
        if let Some(session) = resume_session {
            info!(
                conversation_id = %conversation_id,
                session_id = %session.id,
                message_count = session.messages.len(),
                "Resuming nomi session"
            );
            bootstrap = bootstrap.resume(session);
        }

        // SSH-bound session: hand the runtime the pre-connected remote backend so
        // the remote tool family takes over Read/Write/Edit/Bash/Grep/Glob. The
        // backend is cloned rather than moved: the engine gets one handle, and the
        // lease kept above is what reports on the link when this runtime dies.
        if let Some(ssh_backend) = &host_wiring.ssh_backend {
            info!(
                conversation_id = %conversation_id,
                "Nomi session bound to a remote SSH host"
            );
            bootstrap = bootstrap.ssh_session(Arc::clone(ssh_backend));
        }

        let result = bootstrap
            .build()
            .await
            .map_err(|e| AppError::Internal(format!("Agent bootstrap failed: {e}")))?;

        let mut engine = result.engine;
        if let Some(sink) = requirement_sink {
            engine
                .registry_mut()
                .register(Box::new(RequirementCompleteTool::new(
                    sink.clone(),
                    conversation_id.clone(),
                )));
            engine
                .registry_mut()
                .register(Box::new(RequirementUpdateStatusTool::new(
                    sink,
                    conversation_id.clone(),
                )));
            debug!(conversation_id = %conversation_id, "Registered requirement native tools");
        }
        if let Some(sink) = companion_sink {
            engine
                .registry_mut()
                .register(Box::new(RecallMemoriesTool::new(sink.clone(), conversation_id.clone())));
            engine
                .registry_mut()
                .register(Box::new(SaveMemoryTool::new(sink.clone(), conversation_id.clone())));
            engine
                .registry_mut()
                .register(Box::new(ListRecentEventsTool::new(sink)));
            debug!(conversation_id = %conversation_id, "Registered companion memory tools");
        }
        // Summoned-companion session (spec §设计 B2/B3): read-only recall over
        // the summoned companion's memories, confirmation-style propose, and
        // the per-turn live memory-snapshot contributor. The factory gates this
        // to owner-authority non-companion sessions, so it never collides with
        // the companion registration above (duplicate names would be refused
        // by the registry anyway).
        if let Some(summon) = summon_wiring {
            engine
                .registry_mut()
                .register(Box::new(RecallMemoriesTool::new(summon.memory_sink, conversation_id.clone())));
            engine.register_context_contributor(Arc::new(SummonContextContributor::new(
                summon.context_sink,
            )));
            debug!(conversation_id = %conversation_id, "Registered summoned-companion tools + snapshot contributor");
        }
        // Companion self-evolved skills (design §7): the native `companion_skill`
        // tool resolves a learned skill's body on demand, and the per-turn
        // ContextContributor injects the active skills' when_to_use index so the
        // model knows what it can invoke. Only present for companion sessions
        // (factory gates on overrides.companion). Empty skill set → the
        // contributor is a no-op (returns None each turn).
        if let Some(skill_sink) = companion_skill_sink {
            engine
                .registry_mut()
                .register(Box::new(CompanionSkillTool::new(skill_sink.clone())));
            engine.register_context_contributor(Arc::new(CompanionSkillContributor::new(skill_sink)));
            debug!(conversation_id = %conversation_id, "Registered companion skill tool + contributor");
        }
        // Computer-history native tools (design §4): read-only Info tools
        // (status / recent / apps / urls / find_chats / count_activity /
        // get_settings) plus write tools (pause / resume / update_settings)
        // that ride the normal approval lane. Registered only when the host
        // injected a sink (installation-owner sessions); hosts without one
        // register nothing.
        register_computer_history_tools(engine.registry_mut(), computer_history_sink.clone());
        if computer_history_sink.is_some() {
            debug!(conversation_id = %conversation_id, "Registered computer-history native tools");
        }
        // Capture a handle for proactive RAG before the sink/ids are consumed
        // by tool registration below (only when bound bases make search valid).
        let knowledge_auto_rag = knowledge_retrieval_sink
            .as_ref()
            .filter(|_| register_knowledge_search)
            .map(|s| (s.clone(), knowledge_kb_ids.clone()));
        if let Some(sink) = knowledge_retrieval_sink {
            if register_knowledge_search {
                let search_registered = engine
                    .registry_mut()
                    .register(Box::new(KnowledgeSearchTool::new(
                        sink.clone(),
                        knowledge_kb_ids.clone(),
                    )));
                let read_registered = engine
                    .registry_mut()
                    .register(Box::new(KnowledgeReadTool::new(sink, knowledge_kb_ids)));
                if !search_registered || !read_registered {
                    return Err(AppError::Internal(format!(
                        "knowledge tool registration disagreed with the advertised session surface (knowledge_search={search_registered}, knowledge_read={read_registered})"
                    )));
                }
                debug!(conversation_id = %conversation_id, "Registered knowledge_search + knowledge_read tools");
            }
        }
        // Native knowledge_write (回血): registered only when the binding has
        // write-back enabled (factory passes the sink) AND there are bound bases.
        // The tool was already added to the engine allow_list above so it
        // bypasses the approval gate. Where a write lands and whether it may
        // land at all is enforced entirely in the service (write_document), so
        // the tool carries no placement of its own.
        if let Some(sink) = knowledge_writeback_sink {
            if register_knowledge_write {
                let bound_kb_ids: Vec<nomifun_common::KnowledgeBaseId> =
                    knowledge_write_bases.iter().map(|(id, _)| id.clone()).collect();
                engine
                    .registry_mut()
                    .register(Box::new(KnowledgeWriteTool::new(sink, knowledge_write_bases, bound_kb_ids)));
                debug!(
                    conversation_id = %conversation_id,
                    "Registered knowledge_write tool"
                );
            }
        }
        if let Some(tool) = image_generation_tool {
            let registered = engine.registry_mut().register(tool);
            if !registered {
                return Err(AppError::Internal(
                    "image_gen could not be registered under the session tool policy".to_owned(),
                ));
            }
            debug!(conversation_id = %conversation_id, "Registered native image_gen tool");
        }
        if !is_resume && let Err(e) = engine.init_session(&provider_label, &workspace, Some(&conversation_id)) {
            error!(
                conversation_id = %conversation_id,
                error = %ErrorChain(&*e),
                "Failed to init session, continuing without persistence"
            );
        }

        // Stamp the owning-conversation identity onto the session so a future
        // conversation that reuses this integer id cannot resume it (the factory
        // rejects a mismatching `owner_token` on load). Idempotent; no-op for a
        // resumed session the factory already migrated, and for None (no token).
        engine.stamp_owner_token(config_extra.owner_token.clone());

        let protocol_sink = BackendProtocolSink::new(runtime.event_sender(), confirmations.clone());
        engine.set_approval_manager(approval_manager.clone());
        engine.set_protocol_writer(Arc::new(protocol_sink));
        let slash_commands = engine
            .slash_command_list()
            .into_iter()
            .map(|(command, description)| SlashCommandItem { command, description })
            .collect();

        runtime.transition_to(ConversationStatus::Pending);
        let process_supervisor = engine.process_supervisor_handle();

        Ok(Self {
            runtime,
            backend_output_sink,
            engine: Mutex::new(engine),
            process_supervisor,
            turn_teardown_fence: Arc::new(TurnTeardownFence::new()),
            slash_commands,
            mcp_managers: result.mcp_managers,
            loopback_capability_leases,
            #[cfg(feature = "browser-use")]
            browser_lane_binding,
            ssh_lease,
            approval_manager,
            confirmations,
            turn_cancel: std::sync::Mutex::new(tokio_util::sync::CancellationToken::new()),
            active_turn: Arc::new(std::sync::Mutex::new(None)),
            lifecycle_gate: Arc::new(std::sync::Mutex::new(())),
            turn_gate: Mutex::new(()),
            closing: AtomicBool::new(false),
            steering_inbox: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            system_resource_inbox: Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            distill_dir,
            image_read_root,
            distill_cfg,
            knowledge_prelude: std::sync::Mutex::new(knowledge_prelude),
            knowledge_auto_rag,
            image_generation_availability: std::sync::RwLock::new(
                image_generation_availability,
            ),
            image_generation_discovery,
            image_generation_response_in_chinese,
        })
    }

    fn image_generation_availability(&self) -> ImageGenerationAvailability {
        *self
            .image_generation_availability
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Refresh the process-owned image route before a new turn becomes active.
    /// Discovery is local-only; registry replacement happens under the engine
    /// mutex and the route state is published last.
    async fn refresh_image_generation_capability(&self) -> ImageGenerationAvailability {
        let Some(discovery) = self.image_generation_discovery.as_ref() else {
            return self.image_generation_availability();
        };

        let discovered = discovery.discover_tool().await;
        let mut engine = self.engine.lock().await;
        engine.registry_mut().unregister(IMAGE_GEN_TOOL_NAME);
        let availability = match discovered {
            Ok(Some(tool)) if tool.name() == IMAGE_GEN_TOOL_NAME => {
                if engine.registry_mut().register(tool) {
                    ImageGenerationAvailability::Ready
                } else {
                    error!(
                        conversation_id = %self.runtime.conversation_id(),
                        "image_gen: refreshed tool could not be registered under the persistent policy"
                    );
                    ImageGenerationAvailability::DiscoveryFailed
                }
            }
            Ok(Some(tool)) => {
                error!(
                    conversation_id = %self.runtime.conversation_id(),
                    tool = %tool.name(),
                    "image_gen: discovery returned an unexpected tool route"
                );
                ImageGenerationAvailability::DiscoveryFailed
            }
            Ok(None) => ImageGenerationAvailability::NoConfiguredModel,
            Err(error) => {
                error!(
                    conversation_id = %self.runtime.conversation_id(),
                    error = %ErrorChain(&error),
                    "image_gen: live catalog refresh failed closed"
                );
                ImageGenerationAvailability::DiscoveryFailed
            }
        };
        *self
            .image_generation_availability
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = availability;
        drop(engine);
        availability
    }

    /// Roll back a model-complete engine pass when the host's deferred
    /// artifact/cancellation transaction did not commit. The engine restores
    /// the accepted-turn root in memory first and performs a checked session
    /// write; a failed write permanently quarantines this manager and is
    /// surfaced through the dedicated session-consistency code.
    async fn restore_uncommitted_completion_turn(
        &self,
        completion_context: &CompletionEvidenceContext,
        detail: impl Into<String>,
    ) -> Result<(), AgentSendError> {
        let persisted = self
            .engine
            .lock()
            .await
            .restore_uncommitted_completion_turn(completion_context);
        // OutputDiscarded must observe the provider-pass checkpoint before
        // abort clears held prose/artifact bookkeeping; reversing these calls
        // can turn a valid checkpoint into a sink consistency Error that masks
        // the actual delivery/session terminal.
        self.backend_output_sink.abort_artifact_delivery_turn();
        if persisted {
            return Ok(());
        }

        self.runtime.mark_transport_broken();
        Err(AgentSendError::agent_session_inconsistent(detail))
    }

    /// Provider failure/cancellation keeps any earlier valid race-tail prefix
    /// visible as failure evidence, while the resumable engine transcript is
    /// still restored to the exact accepted-turn root.
    async fn restore_uncommitted_completion_attempt(
        &self,
        completion_context: &CompletionEvidenceContext,
        detail: impl Into<String>,
    ) -> Result<(), AgentSendError> {
        let persisted = self
            .engine
            .lock()
            .await
            .restore_uncommitted_completion_attempt(completion_context);
        self.backend_output_sink.abort_artifact_delivery_turn();
        if persisted {
            return Ok(());
        }

        self.runtime.mark_transport_broken();
        Err(AgentSendError::agent_session_inconsistent(detail))
    }

    fn request_stop(
        &self,
        reason: Option<AgentKillReason>,
        operation: &'static str,
        close_permanently: bool,
    ) -> bool {
        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if close_permanently {
            self.closing.store(true, Ordering::Release);
        }
        let was_running = self.runtime.status() == Some(ConversationStatus::Running);
        let runtime_turn = *self.active_turn.lock().unwrap_or_else(|e| e.into_inner());

        self.turn_cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel();
        // Stop rejects all queued interjections for this generation. A steer
        // that races after this point takes the same lifecycle gate, observes
        // the cancelled token, and returns false instead of claiming delivery.
        self.steering_inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();

        if let Ok(mut confs) = self.confirmations.write() {
            confs.clear();
        }

        if was_running {
            // The durable token above wakes the active turn's select branch.
            // That branch drops the in-flight engine/tool future before it
            // settles frontend tool state, so a late success cannot race a
            // cancellation Error.
        } else if !close_permanently {
            // Idle / Pending / between turns: there is no in-flight run to wake,
            // so notify_waiters would be a no-op AND no terminal event would ever
            // be broadcast — a relay subscribed to this conversation would hang
            // forever in a 'running' spinner. Emit the terminal event directly.
            // Idempotent via AgentRuntimeState's absorbing-state guard (a later real
            // Finish is absorbed). A later reusable turn receives a fresh token.
            self.backend_output_sink.cancel_active_tool_calls(
                "The tool call was cancelled before the turn could finish.",
            );
            if let Some(runtime_turn) = runtime_turn {
                self.runtime.emit_finish_for_turn(
                    runtime_turn,
                    None,
                    Some(TurnStopReason::Cancelled),
                );
            } else {
                // Initial Pending/idle managers have no accepted turn token,
                // but their stop contract still requires a terminal boundary.
                // Between real turns the absorbing Finished state makes this
                // a no-op, so it cannot terminate a later reset turn.
                self.runtime
                    .emit_finish_with_reason(None, Some(TurnStopReason::Cancelled));
            }
            *self.active_turn.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }

        info!(
            conversation_id = %self.runtime.conversation_id(),
            ?reason,
            was_running,
            operation,
            "Nomi stop signal requested"
        );
        was_running
    }
}

#[async_trait::async_trait]
impl crate::runtime_handle::AgentRuntimeControl for NomiAgentManager {
    fn agent_type(&self) -> AgentType {
        AgentType::Nomi
    }

    fn conversation_id(&self) -> &str {
        self.runtime.conversation_id()
    }

    fn workspace(&self) -> &str {
        self.runtime.workspace()
    }

    fn status(&self) -> Option<ConversationStatus> {
        self.runtime.status()
    }

    fn is_transport_healthy(&self) -> bool {
        self.runtime.is_transport_healthy()
    }

    fn last_activity_at(&self) -> TimestampMs {
        self.runtime.last_activity_at()
    }

    fn touch_activity(&self) {
        self.runtime.bump_activity();
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.runtime.subscribe()
    }

    async fn send_message(&self, data: SendMessageData) -> Result<(), AgentSendError> {
        let started_at = now_ms();
        let source_message_id = data
            .source_message_id
            .as_deref()
            .unwrap_or(&data.msg_id)
            .to_owned();
        info!(
            conversation_id = %self.runtime.conversation_id(),
            msg_id = %data.msg_id,
            "Nomi send_message started"
        );
        let _turn = self.turn_gate.lock().await;
        if !self
            .turn_teardown_fence
            .wait_until_clear(TURN_TEARDOWN_FENCE_WAIT_TIMEOUT)
            .await
        {
            // Do not publish a manager terminal here: exact cleanup still owns
            // the old generation. The result-bearing send-error path emits the
            // structured stream failure, while this irreversible health bit
            // makes registry reuse impossible until teardown is proven.
            self.runtime.mark_transport_broken();
            return Err(AgentSendError::stream_broken(
                format!(
                    "A prior Nomi turn did not prove exact process/tool cleanup within {} seconds; the cached runtime is quarantined",
                    TURN_TEARDOWN_FENCE_WAIT_TIMEOUT.as_secs()
                ),
            ));
        }
        // Capture the exact process authority before admitting the new runtime
        // generation. The turn termination guard can then fence subprocesses
        // even if the engine future panics and its mutex guard unwinds.
        let process_supervisor = self.process_supervisor.clone();
        let (turn_cancel, runtime_turn) = {
            let _lifecycle = self
                .lifecycle_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if self.closing.load(Ordering::Acquire) {
                return Err(AgentSendError::from_app_error(AppError::Conflict(
                    "Agent runtime is shutting down; retry on the replacement runtime".to_owned(),
                )));
            }
            // Backstop for abnormal teardown and data written by older builds:
            // a fresh explicit turn never inherits steering from a prior
            // generation. Normal terminalization performs the same clear.
            self.steering_inbox
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            let token = tokio_util::sync::CancellationToken::new();
            *self
                .turn_cancel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = token.clone();
            self.runtime.bump_activity();
            let runtime_turn = self.runtime.reset_for_new_turn(ConversationStatus::Running);
            *self.active_turn.lock().unwrap_or_else(|e| e.into_inner()) = Some(runtime_turn);
            (token, runtime_turn)
        };

        // Backstop: guarantee a terminal event even if this turn unwinds
        // abnormally (engine panic / early-return). Disarmed on the normal path
        // after the real terminal event is emitted below. (Phase 0 F0.2)
        let accepted_turn_recovery_required = Arc::new(AtomicBool::new(false));
        let mut term_guard = TurnTerminationGuard {
            runtime: self.runtime.clone(),
            turn: runtime_turn,
            active_turn: Arc::clone(&self.active_turn),
            lifecycle_gate: Arc::clone(&self.lifecycle_gate),
            steering_inbox: Arc::clone(&self.steering_inbox),
            backend_output_sink: self.backend_output_sink.clone(),
            process_supervisor: process_supervisor.clone(),
            mcp_managers: self.mcp_managers.clone(),
            turn_teardown_fence: Arc::clone(&self.turn_teardown_fence),
            accepted_turn_recovery_required: Arc::clone(
                &accepted_turn_recovery_required,
            ),
            #[cfg(feature = "browser-use")]
            browser_lane_binding: self.browser_lane_binding.clone(),
            armed: true,
        };

        // Catalog refresh belongs to this accepted turn's cancellation
        // domain. Previously it ran before `reset_for_new_turn`, so Stop could
        // observe an idle runtime, cancel the old token, and still lose to a
        // slow local discovery that went on to start a fresh turn. Discovery
        // itself is local-only, but it may wait on SQLite/decryption locks.
        let image_generation_availability = tokio::select! {
            biased;
            _ = turn_cancel.cancelled() => {
                self.backend_output_sink.cancel_active_tool_calls(
                    "The turn was cancelled during image-model capability refresh.",
                );
                term_guard.fence_cancelled_processes().await?;
                term_guard
                    .terminalize(|runtime, turn| {
                        runtime.emit_finish_for_turn(
                            turn,
                            None,
                            Some(TurnStopReason::Cancelled),
                        )
                    })
                    .await
                    .map_err(AgentSendError::from_app_error)?;
                return Ok(());
            }
            availability = self.refresh_image_generation_capability() => availability,
        };

        let direct_image_intent = classify_image_generation_intent(&data.content);
        let (prior_image_intent, plan_mode_active, intent_provider, intent_model, intent_history) =
            tokio::select! {
                biased;
                _ = turn_cancel.cancelled() => {
                    self.backend_output_sink.cancel_active_tool_calls(
                        "The turn was cancelled before image-intent routing preparation completed.",
                    );
                    term_guard.fence_cancelled_processes().await?;
                    term_guard.terminalize(|runtime, turn| {
                        runtime.emit_finish_for_turn(
                            turn,
                            None,
                            Some(TurnStopReason::Cancelled),
                        )
                    }).await.map_err(AgentSendError::from_app_error)?;
                    return Ok(());
                }
                snapshot = async {
                    let engine = self.engine.lock().await;
                    let marker = engine.host_context_value(IMAGE_ROUTE_CONTEXT_KEY);
                    (
                        image_route_from_context(marker.as_deref()),
                        engine.is_plan_mode_active(),
                        Arc::clone(engine.provider()),
                        engine.model_name().to_owned(),
                        engine.messages_transcript(),
                    )
                } => snapshot,
            };
        let should_classify_ambiguous_visual = should_run_image_intent_model(
            direct_image_intent,
            prior_image_intent,
            &data.content,
            !data.files.is_empty(),
            plan_mode_active,
        );
        let mut image_intent_classification_failed = false;
        let classified_image_intent = if let Some(route) = image_followup_route(
            direct_image_intent,
            &data.content,
            prior_image_intent,
        ) {
            route
        } else if should_classify_ambiguous_visual {
            let attachment_summary = image_intent_attachment_summary(&data.files);
            let classification = tokio::select! {
                biased;
                _ = turn_cancel.cancelled() => {
                    self.backend_output_sink.cancel_active_tool_calls(
                        "The turn was cancelled during image-intent classification.",
                    );
                    term_guard.fence_cancelled_processes().await?;
                    term_guard.terminalize(|runtime, turn| {
                        runtime.emit_finish_for_turn(
                            turn,
                            None,
                            Some(TurnStopReason::Cancelled),
                        )
                    }).await.map_err(AgentSendError::from_app_error)?;
                    return Ok(());
                }
                result = classify_image_generation_intent_with_model(
                    intent_provider,
                    intent_model,
                    &data.content,
                    prior_image_intent,
                    &intent_history,
                    &attachment_summary,
                ) => result,
            };
            match classification {
                Ok(intent) => intent,
                Err(error) => {
                    // Classification is an optimization/safety router, not the
                    // user's task. A malformed or unavailable classifier must
                    // not fail unrelated chat. The exact empty allowlist below
                    // keeps this ambiguous visual turn tool-less, so failure
                    // can never reopen Browser or a third-party generator.
                    tracing::warn!(
                        conversation_id = %self.runtime.conversation_id(),
                        error = %error,
                        "isolated image-intent classification failed closed"
                    );
                    image_intent_classification_failed = true;
                    direct_image_intent
                }
            }
        } else {
            direct_image_intent
        };
        let image_generation_intent = host_validated_image_intent(
            classified_image_intent,
            direct_image_intent,
            &data.content,
            prior_image_intent,
            plan_mode_active,
        );
        let user_authorized_external_image_execution =
            explicitly_requests_external_image_execution(&data.content);
        let ambiguous_visual_needs_host_clarification = should_classify_ambiguous_visual
            && image_generation_intent == ImageGenerationIntent::None
            && !user_authorized_external_image_execution
            && !explicitly_requests_visual_discussion(&data.content);
        let deterministic_image_response = if image_generation_intent
            == ImageGenerationIntent::ExplicitExternal
            && !EXTERNAL_IMAGE_ARTIFACT_BRIDGE_AVAILABLE
        {
            Some((
                external_image_generation_unavailable_message(
                    image_generation_availability,
                    self.image_generation_response_in_chinese,
                ),
                false,
            ))
        } else if image_generation_intent == ImageGenerationIntent::Creation
            && image_generation_availability != ImageGenerationAvailability::Ready
        {
            Some((
                image_generation_unavailable_message(
                    image_generation_availability,
                    self.image_generation_response_in_chinese,
                ),
                true,
            ))
        } else if ambiguous_visual_needs_host_clarification {
            Some((
                ambiguous_visual_clarification_message(
                    image_generation_availability,
                    self.image_generation_response_in_chinese,
                ),
                false,
            ))
        } else {
            None
        };
        if let Some((response, commits_native_image_route)) = deterministic_image_response {
            // This is a deterministic capability response, not model-authored
            // prose. With no native image model, or without a durable external
            // artifact bridge, there is nothing valid the chat model can call.
            // An ambiguous visual request whose classifier did not establish
            // creation is likewise clarified by the host instead of giving a
            // second model pass an opportunity to browse or claim false success.
            // Persist the exchange so a later runtime retains honest history.
            let mut engine = tokio::select! {
                biased;
                _ = turn_cancel.cancelled() => {
                    self.backend_output_sink.cancel_active_tool_calls(
                        "The turn was cancelled before the image capability response was recorded.",
                    );
                    term_guard.fence_cancelled_processes().await?;
                    term_guard.terminalize(|runtime, turn| {
                        runtime.emit_finish_for_turn(
                            turn,
                            None,
                            Some(TurnStopReason::Cancelled),
                        )
                    }).await.map_err(AgentSendError::from_app_error)?;
                    return Ok(());
                }
                engine = self.engine.lock() => engine,
            };
            if let Err(error) = engine.record_host_text_turn(
                data.content.clone(),
                response.clone(),
                &source_message_id,
            ) {
                drop(engine);
                self.runtime.mark_transport_broken();
                let send_error = AgentSendError::agent_session_inconsistent(format!(
                    "The deterministic image capability response could not be persisted safely: {error}"
                ));
                let stream_error = send_error.stream_error().clone();
                term_guard
                    .terminalize(move |runtime, turn| {
                        runtime.emit_error_data_for_turn(turn, stream_error)
                    })
                    .await
                    .map_err(AgentSendError::from_app_error)?;
                return Err(send_error);
            }
            let context_tokens = engine.context_tokens();
            let context_window = engine.context_window();
            drop(engine);
            let published = term_guard
                .publish_host_text_if_not_cancelled(
                    &turn_cancel,
                    &data.msg_id,
                    &response,
                    TurnCompletedEventData {
                        elapsed_ms: now_ms() - started_at,
                        input_tokens: 0,
                        output_tokens: 0,
                        reasoning_tokens: 0,
                        context_tokens,
                        context_window,
                    },
                )
                .await
                .map_err(AgentSendError::from_app_error)?;
            if !published {
                let rewind = self
                    .engine
                    .lock()
                    .await
                    .rewind_last_turn(&source_message_id);
                if !matches!(rewind, Ok(true)) {
                    self.runtime.mark_transport_broken();
                    let detail = match rewind {
                        Ok(false) => "the deterministic response no longer owned the exact rewind checkpoint".to_owned(),
                        Err(error) => format!("the deterministic response rewind could not be persisted: {error}"),
                        Ok(true) => unreachable!(),
                    };
                    let send_error = AgentSendError::agent_session_inconsistent(detail);
                    let stream_error = send_error.stream_error().clone();
                    term_guard
                        .terminalize(move |runtime, turn| {
                            runtime.emit_error_data_for_turn(turn, stream_error)
                        })
                        .await
                        .map_err(AgentSendError::from_app_error)?;
                    return Err(send_error);
                }
                self.backend_output_sink.cancel_active_tool_calls(
                    "The image capability response was cancelled before publication.",
                );
                term_guard.fence_cancelled_processes().await?;
                term_guard
                    .terminalize(|runtime, turn| {
                        runtime.emit_finish_for_turn(
                            turn,
                            None,
                            Some(TurnStopReason::Cancelled),
                        )
                    })
                    .await
                    .map_err(AgentSendError::from_app_error)?;
            } else if commits_native_image_route {
                self.engine.lock().await.set_host_context_value(
                    IMAGE_ROUTE_CONTEXT_KEY,
                    Some(IMAGE_ROUTE_NATIVE),
                );
            }
            return Ok(());
        }

        let ambiguous_visual_requires_tool_gate = should_classify_ambiguous_visual
            && image_generation_intent == ImageGenerationIntent::None
            && !user_authorized_external_image_execution;
        let plan_mode_image_request_requires_tool_gate = plan_mode_active
            && matches!(
                direct_image_intent,
                ImageGenerationIntent::Creation | ImageGenerationIntent::ExplicitExternal
            );
        let image_tool_allowlist = if image_generation_intent == ImageGenerationIntent::Creation {
            Some(HashSet::from([IMAGE_GEN_TOOL_NAME.to_owned()]))
        } else if image_intent_classification_failed
            || ambiguous_visual_requires_tool_gate
            || plan_mode_image_request_requires_tool_gate
        {
            // `Some(empty)` is deliberately different from `None`: the
            // request-scoped engine authority advertises and dispatches no
            // tools. A visual-candidate classifier returning `none` or
            // `discussion` is not authority to reopen Browser: only an
            // affirmative user request for external execution may do so.
            // Plan mode likewise cannot execute a native or external image
            // route. The global image policy tells the main response to
            // clarify without fabricating an image.
            Some(HashSet::new())
        } else {
            None
        };
        let route_allows_knowledge =
            route_allows_knowledge_context(image_tool_allowlist.as_ref());

        // Every asynchronous operation after entering Running belongs to this
        // durable cancellation domain. Preparation and execution converge on
        // one cancellation terminal below.
        let mut cancelled_session_error = None;
        let accepted_turn = 'accepted: {
            let prepare_turn = async {
                // A provider already known not to support vision receives the
                // text turn unchanged. This capability lock and all attachment
                // work remain cancellable.
                let supports_image = self.engine.lock().await.compat().supports_image();
                let image_blocks = if supports_image {
                    load_image_blocks(&data.files, self.image_read_root.as_deref()).await?
                } else {
                    Vec::new()
                };

                // Proactive RAG is best-effort, but never allowed to make a
                // cancelled turn wait forever.
                let knowledge_hits = if route_allows_knowledge
                    && let Some((sink, kb_ids)) = &self.knowledge_auto_rag
                {
                    match sink.search(kb_ids, &data.content, 3).await {
                        Ok(hits) if !hits.is_empty() => Some(hits),
                        _ => None,
                    }
                } else {
                    None
                };

                let engine = self.engine.lock().await;
                Ok::<_, ImageAttachmentError>((image_blocks, knowledge_hits, engine))
            };
            let preparation = tokio::select! {
                biased;
                _ = turn_cancel.cancelled() => break 'accepted None,
                prepared = prepare_turn => prepared,
            };

            let (image_blocks, knowledge_hits, mut engine) = match preparation {
                Ok(prepared) => prepared,
                Err(error) => {
                    let send_error = AgentSendError::from_app_error(AppError::BadRequest(format!(
                        "Invalid parameters: {error}"
                    )));
                    self.backend_output_sink.fail_active_tool_calls(
                        "The turn failed while loading its attachments.",
                    );
                    let stream_error = send_error.stream_error().clone();
                    term_guard.terminalize(move |runtime, turn| {
                        runtime.emit_error_data_for_turn(turn, stream_error)
                    }).await.map_err(AgentSendError::from_app_error)?;
                    return Err(send_error);
                }
            };

            // Consume the one-shot prelude only after every cancellable
            // preparation await has completed successfully.
            let prelude = route_allows_knowledge.then(|| {
                self.knowledge_prelude
                    .lock()
                    .expect("knowledge_prelude lock poisoned")
                    .take()
            }).flatten();
            let content = apply_knowledge_prelude(prelude, &data.content);
            let content = match knowledge_hits {
                Some(hits) => prepend_knowledge_context(&hits, content),
                None => content,
            };

            self.backend_output_sink
                .begin_deferred_artifact_delivery_turn_for(
                    self.runtime.conversation_id(),
                    &data.msg_id,
                )
                .map_err(|error| {
                    AgentSendError::from_app_error(AppError::Internal(format!(
                        "failed to begin recoverable artifact delivery: {error}"
                    )))
                })?;
            if matches!(
                image_generation_intent,
                ImageGenerationIntent::Creation | ImageGenerationIntent::ExplicitExternal
            ) {
                self.backend_output_sink
                    .require_image_artifact_for_turn()
                    .map_err(|error| {
                        AgentSendError::from_app_error(AppError::Internal(format!(
                            "failed to register image-generation artifact requirement: {error}"
                        )))
                    })?;
            }
            engine.set_steering_inbox(Some(self.steering_inbox.clone()));
            engine.set_system_resource_inbox(Some(self.system_resource_inbox.clone()));
            // Completion adjudication is bound to trusted user-authored input,
            // never the RAG/knowledge prelude sent to the provider. This
            // host-owned scope survives bounded steering race-tail engine calls
            // that still belong to this one admitted turn.
            let mut completion_context =
                CompletionEvidenceContext::with_host_recovery_signal(
                    vec![ContentBlock::Text {
                        text: data.content.clone(),
                    }],
                    Arc::clone(&accepted_turn_recovery_required),
                );
            // Each iteration runs one engine pass inside the same accepted
            // Agent turn. Re-run only for steering race-tail interjections; the
            // engine owns output-truncation recovery, because only it can drop
            // the truncated draft, re-push the original requirement, and carry a
            // machine-built ledger forward without resetting its own loop guard.
            let mut run_content = Vec::with_capacity(1 + image_blocks.len());
            run_content.push(ContentBlock::Text { text: content });
            run_content.extend(image_blocks);
            let mut race_tail_reruns = 0usize;
            let result = loop {
                let current_content = std::mem::take(&mut run_content);
                // Cancellation has one fail-closed lifecycle: drop the in-flight
                // engine/tool future immediately, then roll back the provisional
                // turn state. Awaiting arbitrary tool code here is unsafe because a
                // tool is not required to observe a cancellation token.
                let r = tokio::select! {
                    biased;
                    _ = turn_cancel.cancelled() => {
                        info!(
                            conversation_id = %self.runtime.conversation_id(),
                            "Nomi engine.execute_turn() cancelled by stop signal"
                        );
                        if completion_context.turn_root_captured() {
                            engine.abort_current_turn("Tool execution canceled by user");
                            if !engine
                                .restore_uncommitted_completion_attempt(&completion_context)
                            {
                                self.runtime.mark_transport_broken();
                                cancelled_session_error = Some(
                                    AgentSendError::agent_session_inconsistent(
                                        "The accepted Agent turn was cancelled in flight, but its durable session root could not be restored",
                                    ),
                                );
                            }
                        }
                        engine.set_steering_inbox(None);
                        engine.set_system_resource_inbox(None);
                        break 'accepted None;
                    }
                    res = engine.execute_turn_with_completion_evidence_context(
                        current_content,
                        &data.msg_id,
                        &source_message_id,
                        image_tool_allowlist.as_ref(),
                        Some(&mut completion_context),
                    ) => res,
                };

                // Race-tail: only a clean Ok can carry leftover steering worth a
                // re-run (a cancel/abort intentionally drops the turn). Bounded so a
                // continuous steerer cannot spin this forever; leftover past the cap
                // is absorbed by this exact turn's terminal transition.
                if let Ok(agent_result) = &r
                    && agent_result.stop_reason == nomi_types::message::StopReason::EndTurn
                    && agent_result.completion_adjudication.is_none()
                {
                    if race_tail_reruns < MAX_STEERING_RACE_TAIL_RERUNS {
                        let leftover: Vec<String> = {
                            let mut q = self.steering_inbox.lock().unwrap_or_else(|e| e.into_inner());
                            q.drain(..).collect()
                        };
                        if !leftover.is_empty() {
                            race_tail_reruns += 1;
                            info!(
                                conversation_id = %self.runtime.conversation_id(),
                                count = leftover.len(),
                                "Nomi steering race-tail: re-running with leftover interjection(s)"
                            );
                            // NOTE: the re-run reuses `data.msg_id`, so the engine emits a
                            // second StreamStart under the same id for this logical turn.
                            // Benign — the UI keeps the same assistant bubble; a fresh id
                            // would instead spawn a new bubble. Intentional for this rare tail.
                            for text in &leftover {
                                completion_context.requirement.push(ContentBlock::Text {
                                    text: text.clone(),
                                });
                            }
                            run_content = vec![ContentBlock::Text {
                                text: leftover.join("\n\n"),
                            }];
                            continue;
                        }
                    } else {
                        tracing::warn!(
                            conversation_id = %self.runtime.conversation_id(),
                            "Nomi steering race-tail cap reached; leftover belongs to this turn and will be discarded at terminal"
                        );
                    }
                }
                break r;
            };

            engine.set_steering_inbox(None);
            engine.set_system_resource_inbox(None);
            Some((result, engine, completion_context))
        };

        let Some((result, engine, completion_context)) = accepted_turn else {
            self.backend_output_sink.cancel_active_tool_calls(
                "The tool call was cancelled because the user stopped the turn.",
            );
            if let Some(session_error) = cancelled_session_error {
                let stream_error = session_error.stream_error().clone();
                term_guard
                    .terminalize(move |runtime, turn| {
                        runtime.emit_error_data_for_turn(turn, stream_error)
                    })
                    .await
                    .map_err(AgentSendError::from_app_error)?;
                return Err(session_error);
            }
            // A stopped turn may have dropped an arbitrary hook/skill/tool
            // future. Its registered child process remains owned by the shared
            // supervisor, so do not publish the business terminal until every
            // such tree has an exact reap outcome. This fence intentionally
            // runs only for explicit/abnormal cancellation: a successful turn
            // may leave a user-requested background exec session alive.
            term_guard.fence_cancelled_processes().await?;
            term_guard.terminalize(|runtime, turn| {
                runtime.emit_finish_for_turn(
                    turn,
                    None,
                    Some(TurnStopReason::Cancelled),
                )
            }).await.map_err(AgentSendError::from_app_error)?;
            return Ok(());
        };

        let elapsed_ms = now_ms() - started_at;
        self.runtime.bump_activity();

        let outcome = match result {
            Ok(agent_result) => {
                let stop_reason = map_engine_stop_reason(agent_result.stop_reason);
                info!(
                    conversation_id = %self.runtime.conversation_id(),
                    elapsed_ms,
                    turns = agent_result.turns,
                    input_tokens = agent_result.usage.input_tokens,
                    output_tokens = agent_result.usage.output_tokens,
                    ?stop_reason,
                    "Nomi engine.execute_turn() completed; closing exact post-turn effects before Finish"
                );

                self.backend_output_sink.fail_active_tool_calls(&format!(
                    "The model turn ended with {stop_reason:?} before this tool call reached a terminal state."
                ));

                // Phase 3 observability: a per-turn metrics event the UI shows as
                // duration / token cost and telemetry records. Purely additive and
                // non-terminal — emitted via `emit()` so it does NOT flip the
                // absorbing Finished state before the real `Finish` below.
                let context_tokens = engine.context_tokens();
                let context_window = engine.context_window();
                let completed_event = TurnCompletedEventData {
                    elapsed_ms,
                    input_tokens: agent_result.usage.input_tokens,
                    output_tokens: agent_result.usage.output_tokens,
                    reasoning_tokens: agent_result.usage.reasoning_tokens,
                    context_tokens,
                    context_window,
                };

                if let Some(issue) = agent_result.completion_adjudication.as_ref() {
                    let rollback_succeeded = issue.history_rollback_succeeded();
                    let detail = issue.detail();
                    let send_error = if rollback_succeeded {
                        AgentSendError::provider_unbacked_completion(detail)
                    } else {
                        AgentSendError::agent_session_inconsistent(detail)
                    };
                    if !rollback_succeeded {
                        // Persistence failed after the in-memory root restore.
                        // Quarantine this reusable manager before releasing turn
                        // admission so no successor can observe divergent state.
                        self.runtime.mark_transport_broken();
                    }
                    self.backend_output_sink.fail_active_tool_calls(
                        "The model reported completion without durable evidence for the requested deliverable.",
                    );
                    self.backend_output_sink.abort_artifact_delivery_turn();
                    drop(engine);

                    let stream_error = send_error.stream_error().clone();
                    if term_guard
                        .fail_adjudicated_turn_if_not_cancelled(
                            &turn_cancel,
                            completed_event,
                            stream_error,
                        )
                        .await
                        .map_err(AgentSendError::from_app_error)?
                    {
                        return Err(send_error);
                    }

                    // Stop won the lifecycle race before A2's metrics+Error
                    // pair. Preserve the existing cancellation contract; no
                    // A2 event escaped the locked transition above.
                    term_guard.fence_cancelled_processes().await?;
                    term_guard
                        .terminalize(|runtime, turn| {
                            runtime.emit_finish_for_turn(
                                turn,
                                None,
                                Some(TurnStopReason::Cancelled),
                            )
                        })
                        .await
                        .map_err(AgentSendError::from_app_error)?;
                    return Ok(());
                }

                // —— Post-session memory distillation (exact turn child) ——
                // Eligibility gates, cheapest first:
                //   1. host opt-in flag (token cost; default off)
                //   2. this session distills at all (distill_dir set; companion
                //      red line already zeroed it at construction)
                //   3. run-time origin empty (cron/autowork/idmm turns excluded,
                //      same rule as the collector's payload_origin red line)
                // All satisfied → snapshot the just-saved transcript, release
                // the engine lock, then await the complete provider+apply
                // effect. Finish is forbidden until this child closes; stop
                // drops the provider future before it can reach the synchronous
                // apply stage. Distill failures remain best-effort and never
                // masquerade as a failed model turn.
                let origin_is_human = data
                    .origin
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty();
                let distill_job = if agent_result.stop_reason
                    == nomi_types::message::StopReason::EndTurn
                    && origin_is_human
                    && super::distill::distill_enabled()
                    && let Some(dir) = self.distill_dir.clone()
                {
                    let transcript = engine.messages_transcript();
                    let cfg = self.distill_cfg.clone();
                    Some((cfg, dir, transcript))
                } else {
                    None
                };
                drop(engine); // no engine mutex across artifact I/O or provider work

                match self
                    .backend_output_sink
                    .verify_artifact_delivery_turn_async(&turn_cancel)
                    .await
                {
                    AsyncArtifactDeliveryOutcome::Verified(_) => {}
                    AsyncArtifactDeliveryOutcome::Cancelled => {
                        if let Err(session_error) = self
                            .restore_uncommitted_completion_turn(
                                &completion_context,
                                "Artifact delivery was cancelled after model completion, and the Agent session root could not be restored",
                            )
                            .await
                        {
                            let stream_error = session_error.stream_error().clone();
                            term_guard
                                .terminalize(move |runtime, turn| {
                                    runtime.emit_error_data_for_turn(turn, stream_error)
                                })
                                .await
                                .map_err(AgentSendError::from_app_error)?;
                            return Err(session_error);
                        }
                        term_guard.fence_cancelled_processes().await?;
                        term_guard
                            .terminalize(|runtime, turn| {
                                runtime.emit_finish_for_turn(
                                    turn,
                                    None,
                                    Some(TurnStopReason::Cancelled),
                                )
                            })
                            .await
                            .map_err(AgentSendError::from_app_error)?;
                        return Ok(());
                    }
                    AsyncArtifactDeliveryOutcome::Failed(delivery_error) => {
                        error!(
                            conversation_id = %self.runtime.conversation_id(),
                            elapsed_ms,
                            error = %delivery_error,
                            "Nomi turn ended without satisfying every artifact-delivery obligation"
                        );
                        let mut send_error = image_artifact_delivery_error_to_send_error(
                            image_generation_intent,
                            &delivery_error,
                        );
                        if let Err(session_error) = self
                            .restore_uncommitted_completion_turn(
                                &completion_context,
                                format!(
                                    "Artifact delivery failed after model completion ({delivery_error}), and the Agent session root could not be restored"
                                ),
                            )
                            .await
                        {
                            send_error = session_error;
                        }
                        let stream_error = send_error.stream_error().clone();
                        term_guard
                            .terminalize(move |runtime, turn| {
                                runtime.emit_error_data_for_turn(turn, stream_error)
                            })
                            .await
                            .map_err(AgentSendError::from_app_error)?;
                        return Err(send_error);
                    }
                }

                // Observability for the one shape B1's restart could in
                // principle launder into a false success: a turn that restarted
                // after the ceiling, was cut off mid state-changing call, had
                // such tools advertised, and still completed no state-changing
                // effect — while its prose says otherwise. `EndTurn` with
                // non-empty text is precisely what the receipt's stop-reason
                // check cannot see.
                //
                // Recorded, NOT enforced. Turning this into a terminal failure
                // would convert a completed turn into a hard error, and review
                // found a real class it would misjudge: a fork-mode `Skill`
                // delegate does genuine file and exec work while the tool itself
                // is `Info`-categorised, so a turn whose deliverable arrived that
                // way scores no state-changing effect. Enforcing it also skips
                // the TurnCompleted metrics event and needs error-card i18n that
                // does not exist yet. Adjudicating a completion claim belongs
                // with the workstream that owns unbacked claims generally; B1's
                // obligation is only to not manufacture the shape, and to make it
                // measurable.
                if agent_result.stop_reason == nomi_types::message::StopReason::EndTurn
                    && agent_result.rounds > 1
                    && agent_result.state_changing_tools_advertised
                    && agent_result.cutoff_state_changing > 0
                    && agent_result.effects_ok == 0
                {
                    tracing::warn!(
                        conversation_id = %self.runtime.conversation_id(),
                        elapsed_ms,
                        rounds = agent_result.rounds,
                        cutoff_state_changing = agent_result.cutoff_state_changing,
                        "Nomi turn restarted after the output ceiling and completed no state-changing tool call"
                    );
                }

                let prepared_distill = match distill_job {
                    Some((cfg, dir, transcript)) => {
                        super::distill::prepare_distill_exact_turn(
                            &turn_cancel,
                            cfg,
                            dir,
                            transcript,
                        )
                        .await
                    }
                    None => (!turn_cancel.is_cancelled()).then_some(None),
                };
                let Some(prepared_distill) = prepared_distill else {
                    if let Err(session_error) = self
                        .restore_uncommitted_completion_turn(
                            &completion_context,
                            "The accepted Agent turn was cancelled after model completion, and its session root could not be restored",
                        )
                        .await
                    {
                        let stream_error = session_error.stream_error().clone();
                        term_guard
                            .terminalize(move |runtime, turn| {
                                runtime.emit_error_data_for_turn(turn, stream_error)
                            })
                            .await
                            .map_err(AgentSendError::from_app_error)?;
                        return Err(session_error);
                    }
                    term_guard.fence_cancelled_processes().await?;
                    term_guard.terminalize(|runtime, turn| {
                        runtime.emit_finish_for_turn(
                            turn,
                            None,
                            Some(TurnStopReason::Cancelled),
                        )
                    }).await.map_err(AgentSendError::from_app_error)?;
                    return Ok(());
                };

                let commit_outcome = match term_guard
                    .commit_verified_turn_if_not_cancelled(
                        &turn_cancel,
                        completed_event,
                        stop_reason,
                        prepared_distill,
                        &self.engine,
                        &completion_context,
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        let mut send_error = AgentSendError::from_app_error(error);
                        if let Err(session_error) = self
                            .restore_uncommitted_completion_turn(
                                &completion_context,
                                "The verified turn could not enter its terminal commit, and the Agent session root could not be restored",
                            )
                            .await
                        {
                            send_error = session_error;
                        }
                        let stream_error = send_error.stream_error().clone();
                        term_guard
                            .terminalize(move |runtime, turn| {
                                runtime.emit_error_data_for_turn(turn, stream_error)
                            })
                            .await
                            .map_err(AgentSendError::from_app_error)?;
                        return Err(send_error);
                    }
                };
                match commit_outcome {
                    VerifiedTurnCommitOutcome::Committed => {
                        self.engine.lock().await.set_host_context_value(
                            IMAGE_ROUTE_CONTEXT_KEY,
                            image_route_context_value(image_generation_intent),
                        );
                        Ok(())
                    }
                    VerifiedTurnCommitOutcome::Cancelled => {
                        if let Err(session_error) = self
                            .restore_uncommitted_completion_turn(
                                &completion_context,
                                "The verified delivery was cancelled before commit, and the Agent session root could not be restored",
                            )
                            .await
                        {
                            let stream_error = session_error.stream_error().clone();
                            term_guard
                                .terminalize(move |runtime, turn| {
                                    runtime.emit_error_data_for_turn(turn, stream_error)
                                })
                                .await
                                .map_err(AgentSendError::from_app_error)?;
                            return Err(session_error);
                        }
                        term_guard.fence_cancelled_processes().await?;
                        term_guard
                            .terminalize(|runtime, turn| {
                                runtime.emit_finish_for_turn(
                                    turn,
                                    None,
                                    Some(TurnStopReason::Cancelled),
                                )
                            })
                            .await
                            .map_err(AgentSendError::from_app_error)?;
                        Ok(())
                    }
                    VerifiedTurnCommitOutcome::DeliveryFailed(delivery_error) => {
                        error!(
                            conversation_id = %self.runtime.conversation_id(),
                            elapsed_ms,
                            error = %delivery_error,
                            "artifact delivery changed before the exact turn commit"
                        );
                        let mut send_error = image_artifact_delivery_error_to_send_error(
                            image_generation_intent,
                            &delivery_error,
                        );
                        if let Err(session_error) = self
                            .restore_uncommitted_completion_turn(
                                &completion_context,
                                format!(
                                    "Artifact delivery changed before commit ({delivery_error}), and the Agent session root could not be restored"
                                ),
                            )
                            .await
                        {
                            send_error = session_error;
                        }
                        let stream_error = send_error.stream_error().clone();
                        term_guard
                            .terminalize(move |runtime, turn| {
                                runtime.emit_error_data_for_turn(turn, stream_error)
                            })
                            .await
                            .map_err(AgentSendError::from_app_error)?;
                        Err(send_error)
                    }
                    VerifiedTurnCommitOutcome::SessionCommitFailed => {
                        let detail = "The verified Agent turn could not be committed to its resumable session";
                        let _ = self
                            .restore_uncommitted_completion_turn(
                                &completion_context,
                                format!(
                                    "{detail}, and the accepted-turn root could not be restored"
                                ),
                            )
                            .await;
                        self.runtime.mark_transport_broken();
                        let send_error = AgentSendError::agent_session_inconsistent(detail);
                        let stream_error = send_error.stream_error().clone();
                        term_guard
                            .terminalize(move |runtime, turn| {
                                runtime.emit_error_data_for_turn(turn, stream_error)
                            })
                            .await
                            .map_err(AgentSendError::from_app_error)?;
                        Err(send_error)
                    }
                }
            }
            Err(e) => {
                // Release the turn's engine guard before the restore helper,
                // which re-acquires it. `Mutex` is not reentrant, so keeping the
                // guard here deadlocks every provider-error turn.
                drop(engine);
                let error_msg = format!("Nomi agent error: {e}");
                error!(
                    conversation_id = %self.runtime.conversation_id(),
                    elapsed_ms,
                    error = %ErrorChain(&e),
                    "Nomi engine.execute_turn() failed, emitting terminal Error"
                );
                let mut send_error = nomi_engine_error_to_send_error(error_msg);
                self.backend_output_sink.fail_active_tool_calls(&format!(
                    "The model/provider turn failed before this tool call completed: {e}"
                ));
                if let Err(session_error) = self
                    .restore_uncommitted_completion_attempt(
                        &completion_context,
                        format!(
                            "The model/provider failed before the accepted turn committed ({e}), and the Agent session root could not be restored"
                        ),
                    )
                    .await
                {
                    send_error = session_error;
                }
                let stream_error = send_error.stream_error().clone();
                term_guard.terminalize(move |runtime, turn| {
                    runtime.emit_error_data_for_turn(turn, stream_error)
                }).await.map_err(AgentSendError::from_app_error)?;
                Err(send_error)
            }
        };

        // Every normal branch above atomically emitted its exact terminal and
        // absorbed that generation's remaining steering before reaching here.
        outcome
    }

    async fn cancel(&self) -> Result<(), AppError> {
        self.request_stop(None, "cancel", false);
        Ok(())
    }

    fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        let was_running = self.request_stop(reason, "kill", true);
        self.loopback_capability_leases.revoke_all();
        #[cfg(feature = "browser-use")]
        if let Some(binding) = &self.browser_lane_binding {
            binding.revoke();
        }
        if !was_running {
            schedule_nomi_cancelled_terminal_after_process_fence(
                self.runtime.clone(),
                Arc::clone(&self.active_turn),
                Arc::clone(&self.lifecycle_gate),
                Arc::clone(&self.steering_inbox),
                self.backend_output_sink.clone(),
                self.process_supervisor.clone(),
                self.mcp_managers.clone(),
                #[cfg(feature = "browser-use")]
                self.browser_lane_binding.clone(),
            )?;
        }
        Ok(())
    }
}

/// RAII backstop guaranteeing a terminal event is broadcast for a turn even if
/// `send_message` unwinds abnormally (engine panic / unexpected early-return).
/// On the normal path `send_message` emits the real terminal event and then
/// disarms the guard; on an abnormal unwind the still-armed guard fires on drop.
/// The emit is idempotent via `AgentRuntimeState`'s absorbing-state guard, so this can
/// never leak a spurious terminal event past a real one. (Phase 0 F0.2)
struct TurnTerminationGuard {
    runtime: AgentRuntimeState,
    turn: crate::runtime_state::AgentRuntimeTurn,
    active_turn: Arc<std::sync::Mutex<Option<crate::runtime_state::AgentRuntimeTurn>>>,
    lifecycle_gate: Arc<std::sync::Mutex<()>>,
    steering_inbox: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    backend_output_sink: Arc<BackendOutputSink>,
    process_supervisor: Option<Arc<nomi_process_runtime::ProcessSupervisor>>,
    mcp_managers: Vec<Arc<McpManager>>,
    turn_teardown_fence: Arc<TurnTeardownFence>,
    /// Set by the engine only after it has durably registered the accepted
    /// session root. If an unwind happens before the host terminal commits,
    /// the backstop must publish the typed inconsistency terminal so the owner
    /// retires the runtime and exactly rewinds that root.
    accepted_turn_recovery_required: Arc<AtomicBool>,
    /// Reusable runtime binding. Turn cleanup closes its current Lanes but must
    /// not revoke the owner lease; the next turn lazily opens a fresh Lane.
    #[cfg(feature = "browser-use")]
    browser_lane_binding: Option<crate::BrowserLaneBinding>,
    armed: bool,
}

enum VerifiedTurnCommitOutcome {
    Committed,
    Cancelled,
    DeliveryFailed(String),
    SessionCommitFailed,
}

impl TurnTerminationGuard {
    async fn terminalize(
        &mut self,
        terminal: impl FnOnce(
            &AgentRuntimeState,
            crate::runtime_state::AgentRuntimeTurn,
        ) -> bool,
    ) -> Result<bool, AppError> {
        #[cfg(feature = "browser-use")]
        if let Some(binding) = &self.browser_lane_binding {
            // A terminal event is the externally observable proof that a turn
            // has ended. Do not publish it while that turn still owns browser
            // pages or Chromium capacity. `close_turn_lanes` preserves the
            // owner lease, so this manager remains reusable.
            binding.close_turn_lanes().await?;
        }

        let emitted = terminalize_exact_nomi_turn(
            &self.runtime,
            &self.lifecycle_gate,
            &self.active_turn,
            &self.steering_inbox,
            self.turn,
            terminal,
        );
        self.armed = false;
        Ok(emitted)
    }

    /// Linearize a deterministic host response against Stop. Browser cleanup
    /// happens first; then the cancellation token is rechecked while holding
    /// the same lifecycle gate used by `request_stop`. If Stop won, no response
    /// event is published and the caller can rewind the provisional engine
    /// history before emitting the normal Cancelled terminal.
    async fn publish_host_text_if_not_cancelled(
        &mut self,
        turn_cancel: &tokio_util::sync::CancellationToken,
        msg_id: &str,
        response: &str,
        completed: TurnCompletedEventData,
    ) -> Result<bool, AppError> {
        #[cfg(feature = "browser-use")]
        if let Some(binding) = &self.browser_lane_binding {
            binding.close_turn_lanes().await?;
        }

        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut active = self
            .active_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.as_ref() != Some(&self.turn) || turn_cancel.is_cancelled() {
            return Ok(false);
        }

        self.backend_output_sink.emit_stream_start(msg_id);
        self.backend_output_sink.emit_text_delta(response, msg_id);
        self.runtime.emit(AgentStreamEvent::TurnCompleted(completed));
        let emitted = self.runtime.emit_finish_for_turn(
            self.turn,
            None,
            Some(TurnStopReason::EndTurn),
        );
        self.steering_inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        *active = None;
        self.armed = false;
        Ok(emitted)
    }

    /// Publish A2's accounting event and typed failure as one lifecycle
    /// transition. Stop takes the same gate, so observers can never see
    /// TurnCompleted followed by Cancelled, nor a double terminal.
    async fn fail_adjudicated_turn_if_not_cancelled(
        &mut self,
        turn_cancel: &tokio_util::sync::CancellationToken,
        completed: TurnCompletedEventData,
        stream_error: crate::protocol::events::ErrorEventData,
    ) -> Result<bool, AppError> {
        #[cfg(feature = "browser-use")]
        if let Some(binding) = &self.browser_lane_binding {
            binding.close_turn_lanes().await?;
        }

        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut active = self
            .active_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.as_ref() != Some(&self.turn) || turn_cancel.is_cancelled() {
            return Ok(false);
        }

        self.runtime.emit(AgentStreamEvent::TurnCompleted(completed));
        let emitted = self
            .runtime
            .emit_error_data_for_turn(self.turn, stream_error);
        self.steering_inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        *active = None;
        self.armed = false;
        Ok(emitted)
    }

    /// Atomically transfer verified artifacts/held prose to the visible turn
    /// and publish its terminal event. Stop uses the same lifecycle gate, so it
    /// must linearize either before this commit (no prose or receipt escapes) or
    /// after the exact EndTurn has already absorbed the active generation.
    async fn commit_verified_turn_if_not_cancelled(
        &mut self,
        turn_cancel: &tokio_util::sync::CancellationToken,
        completed: TurnCompletedEventData,
        stop_reason: TurnStopReason,
        prepared_distill: Option<super::distill::PreparedDistill>,
        engine: &Mutex<AgentEngine>,
        completion_context: &CompletionEvidenceContext,
    ) -> Result<VerifiedTurnCommitOutcome, AppError> {
        #[cfg(feature = "browser-use")]
        if let Some(binding) = &self.browser_lane_binding {
            binding.close_turn_lanes().await?;
        }

        let verified = match self
            .backend_output_sink
            .verify_artifact_delivery_turn_async(turn_cancel)
            .await
        {
            AsyncArtifactDeliveryOutcome::Verified(verified) => verified,
            AsyncArtifactDeliveryOutcome::Cancelled => {
                return Ok(VerifiedTurnCommitOutcome::Cancelled);
            }
            AsyncArtifactDeliveryOutcome::Failed(error) => {
                return Ok(VerifiedTurnCommitOutcome::DeliveryFailed(error));
            }
        };

        // Acquire the async engine lock before the synchronous lifecycle gate.
        // Stop never needs the engine lock, and the final cancellation check
        // below ensures a stop that won while we waited remains authoritative.
        let mut engine = engine.lock().await;
        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut active = self
            .active_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.as_ref() != Some(&self.turn) || turn_cancel.is_cancelled() {
            return Ok(VerifiedTurnCommitOutcome::Cancelled);
        }
        if !engine.seal_completion_for_host_terminal(completion_context) {
            return Ok(VerifiedTurnCommitOutcome::SessionCommitFailed);
        }
        if let Err(error) = self
            .backend_output_sink
            .finish_verified_artifact_delivery_turn(verified)
        {
            return Ok(VerifiedTurnCommitOutcome::DeliveryFailed(error));
        }

        super::distill::apply_prepared_distill(prepared_distill);
        self.runtime.emit(AgentStreamEvent::TurnCompleted(completed));
        self.runtime
            .emit_finish_for_turn(self.turn, None, Some(stop_reason));
        self.steering_inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        *active = None;
        self.armed = false;
        Ok(VerifiedTurnCommitOutcome::Committed)
    }

    async fn fence_cancelled_processes(&self) -> Result<(), AppError> {
        // Attempt both fences even when the first is not exact: an MCP
        // shutdown failure must not leave the process tree unquiesced, and
        // vice versa. The aggregated error preserves the first failure.
        let mut failures = NomiTeardownFailures::default();
        failures.record("MCP", shutdown_mcp_managers_exact(&self.mcp_managers).await);
        if let Some(supervisor) = &self.process_supervisor {
            let report = supervisor.quiesce().await;
            if !report.is_exact() {
                failures.record(
                    "process tree",
                    Err(AppError::Internal(format!(
                        "Nomi process-tree teardown for conversation {} was not exact: {}",
                        self.runtime.conversation_id(),
                        describe_quiesce_failure(&report),
                    ))),
                );
            }
        }
        failures.finish()
    }
}

impl Drop for TurnTerminationGuard {
    fn drop(&mut self) {
        if self.armed {
            // This store happens synchronously while the old send still owns
            // `turn_gate`. Therefore a queued successor can never slip between
            // the unwind and the asynchronous process-tree fence.
            self.turn_teardown_fence.begin();
            // The exact cleanup task below retains resource authority. Marking
            // only the event transport broken is not a cleanup proof and does
            // not release that authority; it makes the abnormal manager
            // ineligible for cache reuse and routes the owner through the
            // registry's bounded quarantine teardown path.
            self.runtime.mark_transport_broken();
            let session_error = self
                .accepted_turn_recovery_required
                .load(Ordering::Acquire)
                .then(|| {
                    AgentSendError::agent_session_inconsistent(
                        "The Agent turn ended unexpectedly after registering a durable session recovery root",
                    )
                    .stream_error()
                    .clone()
                });
            self.backend_output_sink.cancel_active_tool_calls(
                "The turn ended unexpectedly before this tool call reached a terminal state.",
            );
            let runtime = self.runtime.clone();
            let conversation_id = runtime.conversation_id().to_owned();
            let turn = self.turn;
            let active_turn = Arc::clone(&self.active_turn);
            let lifecycle_gate = Arc::clone(&self.lifecycle_gate);
            let steering_inbox = Arc::clone(&self.steering_inbox);
            let process_supervisor = self.process_supervisor.clone();
            let mcp_managers = self.mcp_managers.clone();
            let turn_teardown_fence = Arc::clone(&self.turn_teardown_fence);
            #[cfg(feature = "browser-use")]
            let browser_lane_binding = self.browser_lane_binding.clone();
            let terminalize = move || {
                terminalize_exact_nomi_turn(
                    &runtime,
                    &lifecycle_gate,
                    &active_turn,
                    &steering_inbox,
                    turn,
                    move |runtime, turn| {
                        if let Some(error) = session_error {
                            runtime.emit_error_data_for_turn(turn, error)
                        } else {
                            runtime.emit_finish_for_turn(
                                turn,
                                None,
                                Some(TurnStopReason::Cancelled),
                            )
                        }
                    },
                );
                turn_teardown_fence.complete();
            };

            let Ok(runtime_handle) = tokio::runtime::Handle::try_current() else {
                error!(
                    conversation_id = %self.runtime.conversation_id(),
                    "Nomi turn unwound outside a Tokio runtime; refusing to publish a terminal state before process-tree teardown"
                );
                return;
            };
            runtime_handle.spawn(async move {
                // Every fence below is attempted unconditionally. An inexact
                // MCP or process teardown must never skip the Browser Lane
                // cleanup (or vice versa); only the terminal publication is
                // conditioned on the aggregate proof.
                let mut exact = true;
                if let Err(error) = shutdown_mcp_managers_exact(&mcp_managers).await {
                    error!(
                        conversation_id = %conversation_id,
                        error = %error,
                        "Nomi turn MCP teardown was not exact; retaining non-terminal quarantine"
                    );
                    exact = false;
                }
                if let Some(supervisor) = process_supervisor {
                    let report = supervisor.quiesce().await;
                    if !report.is_exact() {
                        error!(
                            conversation_id = %conversation_id,
                            failure = %describe_quiesce_failure(&report),
                            "Nomi turn process-tree teardown was not exact; retaining non-terminal quarantine"
                        );
                        exact = false;
                    }
                }
                #[cfg(feature = "browser-use")]
                if let Some(binding) = browser_lane_binding
                    && let Err(error) = binding.close_turn_lanes().await
                {
                    error!(
                        conversation_id = %conversation_id,
                        error = %ErrorChain(&error),
                        "Nomi turn Browser Lane cleanup was not exact; retaining non-terminal quarantine"
                    );
                    exact = false;
                }
                if exact {
                    terminalize();
                }
                // A non-exact teardown deliberately withholds the terminal
                // event: the runtime-registry quarantine remains authoritative
                // until a later kill/teardown path proves exact cleanup.
            });
        }
    }
}

async fn shutdown_mcp_managers_exact(
    managers: &[Arc<McpManager>],
) -> Result<(), AppError> {
    let mut failures = Vec::new();
    for manager in managers {
        if let Err(error) = manager.shutdown().await {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Internal(format!(
            "Nomi MCP shutdown was not exact: {}",
            failures.join(" | ")
        )))
    }
}

struct NomiTeardownResults {
    kill: Result<(), AppError>,
    mcp: Result<(), AppError>,
    process: Result<(), AppError>,
    #[cfg(feature = "browser-use")]
    browser_lane_binding: Option<crate::BrowserLaneBinding>,
    ssh_lease: Option<Arc<dyn crate::SshSessionLease>>,
}

#[derive(Default)]
struct NomiTeardownFailures {
    failures: Vec<(&'static str, AppError)>,
}

impl NomiTeardownFailures {
    fn record(&mut self, stage: &'static str, result: Result<(), AppError>) {
        if let Err(error) = result {
            self.failures.push((stage, error));
        }
    }

    /// Preserve the exact AppError when there is only one failed stage. When
    /// multiple independent cleanup stages fail, the public error type has no
    /// multi-error variant, so retain the first failure's text as the primary
    /// error and append every other failure for diagnosis.
    fn finish(mut self) -> Result<(), AppError> {
        match self.failures.len() {
            0 => Ok(()),
            1 => Err(self.failures.pop().expect("failure count checked").1),
            _ => {
                let (primary_stage, primary_error) = self.failures.remove(0);
                let additional = self
                    .failures
                    .iter()
                    .map(|(stage, error)| format!("{stage}: {error}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                let suffix = format!(
                    "Nomi runtime teardown also failed after primary stage \
                     {primary_stage}: {additional}"
                );
                Err(match primary_error {
                    AppError::Internal(message) => {
                        AppError::Internal(format!("{message}; {suffix}"))
                    }
                    AppError::Timeout(message) => {
                        AppError::Timeout(format!("{message}; {suffix}"))
                    }
                    other => {
                        error!(
                            primary_stage,
                            primary_error = %ErrorChain(&other),
                            additional_failures = %additional,
                            "Nomi runtime teardown had multiple failures; preserving the primary error variant"
                        );
                        other
                    }
                })
            }
        }
    }
}

/// Turn a released SSH lease into a teardown verdict.
///
/// A link the pool deliberately kept for the conversation is the ordinary outcome
/// of a model switch, and a proven close is a clean one — both are success. A link
/// that went away without proof its remote shell died is a genuine failure, for
/// the same reason `PtyExit::Lost` is never accepted as one: an unproven cleanup
/// is indistinguishable from a leaked process on someone else's machine.
fn describe_ssh_release(release: crate::SshLeaseRelease) -> Result<(), AppError> {
    match release {
        crate::SshLeaseRelease::Retained { detail } => {
            debug!(detail = %detail, "SSH session link retained for the conversation");
            Ok(())
        }
        crate::SshLeaseRelease::Reaped { detail } => {
            debug!(detail = %detail, "SSH session link closed with exit evidence");
            Ok(())
        }
        crate::SshLeaseRelease::Lost { detail } => Err(AppError::Internal(format!(
            "Nomi SSH session link was let go of without proof the remote shell died: {detail}"
        ))),
    }
}

/// Finish every already-started Nomi teardown stage without allowing an early
/// failure to skip the Browser owner cleanup. In particular, `kill()` already
/// issues a synchronous best-effort revoke, but this function is the
/// result-bearing proof that waits for the Hub-owned owner-lease cleanup flight.
async fn finish_nomi_teardown(results: NomiTeardownResults) -> Result<(), AppError> {
    let mut failures = NomiTeardownFailures::default();
    failures.record("kill", results.kill);
    failures.record("MCP", results.mcp);
    failures.record("process tree", results.process);

    #[cfg(feature = "browser-use")]
    if let Some(binding) = results.browser_lane_binding {
        // Deliberately run this after the other stages, but never condition it
        // on their success. The Hub retains the cleanup flight if this waiter
        // itself reports a timeout, so a later lifecycle sweep can retry it.
        failures.record("Browser owner lease", binding.shutdown().await);
    }

    // Same posture for the remote session: last, and unconditional. A failed kill
    // must not cost us the one report that says whether the operator's shell is
    // still there — releasing is not closing, so this is safe to do late.
    if let Some(lease) = results.ssh_lease {
        failures.record("SSH session link", describe_ssh_release(lease.release().await));
    }

    failures.finish()
}

fn describe_quiesce_failure(report: &nomi_process_runtime::QuiesceReport) -> String {
    let mut details = report.errors.clone();
    for session in &report.sessions {
        if let nomi_process_runtime::ProcessOutcome::Lost { cleanup, .. } = &session.outcome
            && !cleanup.reaped
        {
            details.push(format!(
                "session {} owner {}/{} was not reaped: {}",
                session.session_id,
                session.owner.invocation_id,
                session.owner.call_id,
                cleanup.errors.join("; "),
            ));
        }
    }
    if details.is_empty() {
        "one or more process sessions lack exact reap proof".to_owned()
    } else {
        details.join(" | ")
    }
}

fn schedule_nomi_cancelled_terminal_after_process_fence(
    runtime: AgentRuntimeState,
    active_turn: Arc<std::sync::Mutex<Option<crate::runtime_state::AgentRuntimeTurn>>>,
    lifecycle_gate: Arc<std::sync::Mutex<()>>,
    steering_inbox: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    backend_output_sink: Arc<BackendOutputSink>,
    process_supervisor: Option<Arc<nomi_process_runtime::ProcessSupervisor>>,
    mcp_managers: Vec<Arc<McpManager>>,
    #[cfg(feature = "browser-use")] browser_lane_binding: Option<crate::BrowserLaneBinding>,
) -> Result<(), AppError> {
    let terminalize = move || {
        backend_output_sink.cancel_active_tool_calls(
            "The tool call was cancelled before the turn could finish.",
        );
        let _lifecycle = lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        steering_inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let runtime_turn = active_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(runtime_turn) = runtime_turn {
            runtime.emit_finish_for_turn(
                runtime_turn,
                None,
                Some(TurnStopReason::Cancelled),
            );
        } else {
            runtime.emit_finish_with_reason(None, Some(TurnStopReason::Cancelled));
        }
    };

    #[cfg(feature = "browser-use")]
    let has_browser_binding = browser_lane_binding.is_some();
    #[cfg(not(feature = "browser-use"))]
    let has_browser_binding = false;
    if process_supervisor.is_none() && mcp_managers.is_empty() && !has_browser_binding {
        terminalize();
        return Ok(());
    }
    let runtime_handle = tokio::runtime::Handle::try_current().map_err(|_| {
        AppError::Internal(
            "Cannot schedule Nomi process-tree teardown outside a Tokio runtime".to_owned(),
        )
    })?;
    runtime_handle.spawn(async move {
        // Every fence is attempted unconditionally; only the terminal
        // publication is conditioned on the aggregate exactness proof.
        let mut exact = true;
        if let Err(error) = shutdown_mcp_managers_exact(&mcp_managers).await {
            error!(
                error = %error,
                "Idle Nomi kill could not prove exact MCP teardown; retaining non-terminal quarantine"
            );
            exact = false;
        }
        if let Some(supervisor) = process_supervisor {
            let report = supervisor.quiesce().await;
            if !report.is_exact() {
                error!(
                    failure = %describe_quiesce_failure(&report),
                    "Idle Nomi kill could not prove exact process-tree teardown; retaining non-terminal quarantine"
                );
                exact = false;
            }
        }
        // `kill()` already started the Hub-owned owner-lease revocation
        // flight; join it here so the idle terminal is published only after
        // the bounded Browser cleanup proof, not merely after its request.
        #[cfg(feature = "browser-use")]
        if let Some(binding) = browser_lane_binding
            && let Err(error) = binding.revoke_and_wait().await
        {
            error!(
                error = %ErrorChain(&error),
                "Idle Nomi kill could not prove exact Browser owner cleanup; retaining non-terminal quarantine"
            );
            exact = false;
        }
        if exact {
            terminalize();
        }
    });
    Ok(())
}

impl NomiAgentManager {
    pub fn kill_and_wait(
        &self,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
        let kill_result = crate::runtime_handle::AgentRuntimeControl::kill(self, reason);
        let runtime = self.runtime.clone();
        let process_supervisor = self.process_supervisor.clone();
        let mcp_managers = self.mcp_managers.clone();
        #[cfg(feature = "browser-use")]
        let browser_lane_binding = self.browser_lane_binding.clone();
        let ssh_lease = self.ssh_lease.clone();
        Box::pin(async move {
            // Every cleanup stage is attempted even if an earlier one failed.
            // In particular, a synchronous `kill()` failure or an inexact MCP
            // / process fence must never skip the result-bearing Browser owner
            // lease shutdown below.
            let mcp_result = shutdown_mcp_managers_exact(&mcp_managers).await;
            let process_result = if let Some(supervisor) = process_supervisor {
                let report = supervisor.quiesce().await;
                if !report.is_exact() {
                    Err(AppError::Internal(format!(
                        "Nomi process-tree teardown for conversation {} was not exact: {}",
                        runtime.conversation_id(),
                        describe_quiesce_failure(&report),
                    )))
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            };
            finish_nomi_teardown(NomiTeardownResults {
                kill: kill_result,
                mcp: mcp_result,
                process: process_result,
                #[cfg(feature = "browser-use")]
                browser_lane_binding,
                ssh_lease,
            })
            .await?;
            // No total timeout: runtime-registry quarantine remains authoritative
            // until this exact manager publishes its process-fenced terminal.
            runtime.wait_until_finished_unbounded().await;
            Ok(())
        })
    }

    /// Register the native cron tools (cron_create / cron_list / cron_delete)
    /// backed by `sink`. Called by the factory right after construction, before
    /// the first turn, so the tools are advertised on the first model request.
    pub async fn register_cron_sink(&self, sink: Arc<dyn CronSink>) {
        let mut engine = self.engine.lock().await;
        let reg = engine.registry_mut();
        reg.register(Box::new(CronCreateTool::new(sink.clone())));
        reg.register(Box::new(CronListTool::new(sink.clone())));
        reg.register(Box::new(CronDeleteTool::new(sink)));
        debug!(conversation_id = %self.runtime.conversation_id(), "Registered cron native tools");
    }
}

/// Nomi-specific operations reached through `AgentRuntimeHandle::Nomi(..)`
/// matches in the routes + services.
impl NomiAgentManager {
    /// Push a user interjection into the running turn's steering inbox.
    /// Returns `Ok(true)` if a turn is live and the message was queued for
    /// mid-turn injection; `Ok(false)` if no turn is running (caller should
    /// fall back to a normal send). Never blocks on the engine lock.
    ///
    /// Admission is serialized with exact terminal transition. A steer that
    /// wins the gate belongs to the current active turn and is either consumed
    /// by that turn or absorbed when it terminates. A steer that loses to
    /// terminal returns `false`; it is never queued for the next explicit turn.
    pub fn steer(&self, text: String) -> Result<bool, AppError> {
        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.closing.load(Ordering::Acquire)
            || self.runtime.status() != Some(ConversationStatus::Running)
            || self
                .active_turn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
            || self
                .turn_cancel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_cancelled()
        {
            return Ok(false);
        }
        self.steering_inbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(text);
        self.runtime.bump_activity();
        Ok(true)
    }

    /// Queue trusted host-side resource state without starting a turn.
    ///
    /// The dedicated inbox is mounted on the engine during every real Nomi
    /// turn. The engine drains it into top-level system context immediately
    /// before the next provider request, so an idle runtime retains the notice
    /// until the next user-initiated model call and an active runtime can see it
    /// at its next model boundary. It never enters the user transcript.
    pub fn notify_system_resource(
        &self,
        notice: String,
    ) -> Result<SystemResourceNoticeDelivery, AppError> {
        let notice = notice.trim();
        if notice.is_empty() {
            return Err(AppError::BadRequest(
                "System resource notice must not be empty".into(),
            ));
        }

        let _lifecycle = self
            .lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.closing.load(Ordering::Acquire) {
            return Err(AppError::Conflict(
                "Agent runtime is shutting down and cannot accept resource notifications"
                    .to_owned(),
            ));
        }

        let delivery = if self.runtime.status() == Some(ConversationStatus::Running) {
            SystemResourceNoticeDelivery::ActiveTurn
        } else {
            SystemResourceNoticeDelivery::NextModelCall
        };
        self.system_resource_inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(notice.to_owned());
        self.runtime.bump_activity();
        Ok(delivery)
    }

    pub fn confirm(&self, _msg_id: &str, call_id: &str, data: Value, always_allow: bool) -> Result<(), AppError> {
        if let Ok(mut confs) = self.confirmations.write() {
            confs.retain(|c| c.call_id != call_id);
        }

        let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("cancel");

        let is_cancel = value == "cancel";

        debug!(
            conversation_id = %self.runtime.conversation_id(),
            call_id,
            value,
            always_allow,
            "Nomi confirm"
        );

        if is_cancel {
            self.approval_manager.resolve(
                call_id,
                ToolApprovalResult::Denied {
                    reason: "User denied the tool request".into(),
                },
            );
        } else {
            let scope = if always_allow {
                nomi_protocol::commands::ApprovalScope::Always
            } else {
                nomi_protocol::commands::ApprovalScope::Once
            };
            self.approval_manager.approve(call_id, scope);
        }
        Ok(())
    }

    pub fn get_confirmations(&self) -> Vec<Confirmation> {
        self.confirmations.read().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn check_approval(&self, action: &str, _command_type: Option<&str>) -> bool {
        self.approval_manager.is_auto_approved(action)
    }

    pub async fn mode(&self) -> Result<AgentModeResponse, AppError> {
        Ok(AgentModeResponse {
            mode: self.approval_manager.current_mode(),
            initialized: true,
        })
    }

    pub async fn set_mode(&self, mode: &str) -> Result<(), AppError> {
        let prev = self.approval_manager.current_mode();
        self.approval_manager.set_mode(parse_session_mode(mode));
        info!(
            conversation_id = %self.runtime.conversation_id(),
            from = prev,
            to = mode,
            "Nomi session mode switched"
        );
        Ok(())
    }

    pub async fn get_slash_commands(&self) -> Result<Vec<SlashCommandItem>, AppError> {
        Ok(self.slash_commands.clone())
    }

    /// Clear the conversation context ("release model context"): stop any
    /// in-flight turn, then empty the engine's message history + compaction
    /// state and persist the now-empty session. The agent stays alive; the
    /// next prompt starts from a clean slate.
    pub async fn clear_context(&self) -> Result<(), AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            "Clearing Nomi context"
        );
        // Signal any in-flight engine.execute_turn() to abort so we don't clear
        // mid-turn; the engine lock below then waits for it to release.
        self.request_stop(None, "clear_context", false);
        let mut engine = self.engine.lock().await;
        if let Err(error) = engine.clear_context() {
            self.runtime.mark_transport_broken();
            return Err(AppError::Internal(format!(
                "Agent context could not be cleared durably; the runtime was quarantined: {error}"
            )));
        }
        Ok(())
    }

    /// Read-only preflight for edit/resubmit. This is called before the
    /// Conversation service claims its durable destructive receipt.
    pub async fn ensure_can_rewind_last_turn(
        &self,
        expected_source_message_id: &str,
    ) -> Result<(), AppError> {
        let engine = self.engine.lock().await;
        if !engine.can_rewind_last_turn(expected_source_message_id) {
            return Err(AppError::BadRequest(
                "无法安全回退该历史消息：缺少匹配的持久化编辑检查点，原消息未改变".into(),
            ));
        }
        Ok(())
    }

    /// Rewind the last user turn (edit & resubmit the most recent user message):
    /// stop any in-flight turn, then truncate the engine's in-memory transcript
    /// back to that turn's start so a fresh send re-runs without the stale turn.
    /// Returns BadRequest when there is no valid anchor (e.g. context was
    /// compacted away) so the caller can surface a retriable error.
    pub async fn rewind_last_turn(
        &self,
        expected_source_message_id: &str,
    ) -> Result<(), AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            "Rewinding last Nomi turn"
        );
        self.request_stop(None, "rewind_last_turn", false);
        let mut engine = self.engine.lock().await;
        match engine.rewind_last_turn(expected_source_message_id) {
            Ok(true) => Ok(()),
            Ok(false) => Err(AppError::BadRequest(
                "无法安全回退该历史消息：编辑检查点已失效，原消息未改变".into(),
            )),
            Err(error) => {
                self.runtime.mark_transport_broken();
                Err(AppError::Internal(format!(
                    "Agent turn rewind could not be persisted; the runtime was quarantined: {error}"
                )))
            }
        }
    }

    /// Test-only accessor for the resolved distillation target directory.
    #[cfg(test)]
    pub(crate) fn distill_dir_for_test(&self) -> Option<&std::path::Path> {
        self.distill_dir.as_deref()
    }
}

fn parse_session_mode(s: &str) -> SessionMode {
    match s {
        "auto_edit" => SessionMode::AutoEdit,
        "yolo" => SessionMode::Yolo,
        _ => SessionMode::Default,
    }
}

#[cfg(feature = "browser-use")]
fn should_install_browser_approval_gate(
    browser_takeover: bool,
    browser_unrestricted_approval: bool,
    approval_manager: &ToolApprovalManager,
) -> bool {
    browser_takeover
        || browser_unrestricted_approval
        || approval_manager.is_auto_approved(&ToolCategory::Irreversible.to_string())
}

fn nomi_engine_error_to_send_error(error_msg: String) -> AgentSendError {
    let lower = error_msg.to_ascii_lowercase();
    if lower.contains("provider error") || lower.contains("provider:") || lower.contains("api error:") {
        return AgentSendError::from_app_error(AppError::BadGateway(error_msg));
    }
    AgentSendError::from_app_error(AppError::Internal(error_msg))
}

fn image_artifact_delivery_error_to_send_error(
    intent: ImageGenerationIntent,
    delivery_error: &str,
) -> AgentSendError {
    let lower = delivery_error.trim().to_ascii_lowercase();
    let model_skipped_required_tool =
        lower.starts_with("accepted turn required a verified image artifact")
            && !lower.contains(';');
    let model_or_image_tool_returned_no_artifact = [
        "tool returned an error",
        "tool completed without a verified artifact receipt",
        "artifact-producing tool returned no image artifact",
        "image model completed without an image asset result",
        "image model produced 0 image candidate",
    ]
    .iter()
    .any(|marker| lower.contains(marker));

    if intent == ImageGenerationIntent::Creation
        && (model_skipped_required_tool || model_or_image_tool_returned_no_artifact)
    {
        return AgentSendError::new(
            "The selected model did not deliver the requested image",
            AgentErrorCode::UserLlmProviderEmptyResponse,
            AgentErrorOwnership::UserLlmProvider,
            Some(format!("Artifact delivery failed: {delivery_error}")),
            true,
            false,
            Some(AgentErrorResolution::new(
                AgentErrorResolutionKind::ChangeModel,
                Some(AgentErrorResolutionTarget::ProviderSettings),
            )),
        );
    }

    // Ledger, persistence, final verification, and generation-CAS failures are
    // host integrity failures. Keep those classified as Nomi-owned rather than
    // hiding a real receipt invariant violation behind a provider error.
    AgentSendError::from_app_error(AppError::Internal(format!(
        "Artifact delivery failed: {delivery_error}"
    )))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::events::ToolCallStatus;
    use crate::runtime_handle::AgentRuntimeControl;
    use nomi_protocol::events::ToolCategory;
    use nomi_providers::{LlmProvider, ProviderError};
    use nomi_tools::{registry::ToolRegistry, Tool};
    use nomi_types::llm::{LlmEvent, LlmRequest};
    use nomi_types::message::{ContentBlock, Role, StopReason};
    use nomi_types::tool::ToolResult;
    #[cfg(feature = "browser-use")]
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "browser-use")]
    struct RejectingBrowserHostFactory;

    #[tokio::test]
    async fn turn_teardown_fence_wait_is_bounded_while_cleanup_is_stuck() {
        let fence = TurnTeardownFence::new();
        fence.begin();

        assert!(
            !fence
                .wait_until_clear(std::time::Duration::from_millis(10))
                .await,
            "a successor must not wait forever for an abandoned teardown fence"
        );
    }

    #[tokio::test]
    async fn turn_teardown_fence_wait_observes_exact_completion() {
        let fence = Arc::new(TurnTeardownFence::new());
        fence.begin();
        let completer = Arc::clone(&fence);
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            completer.complete();
        });

        assert!(
            fence
                .wait_until_clear(std::time::Duration::from_secs(1))
                .await,
            "an exact cleanup proof must release the successor fence"
        );
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl nomifun_browser_platform::BrowserHostFactory for RejectingBrowserHostFactory {
        async fn launch(
            &self,
            _request: nomifun_browser_platform::HostLaunchRequest,
        ) -> Result<
            Arc<dyn nomifun_browser_platform::BrowserHostDriver>,
            nomifun_browser_platform::BrowserPlatformError,
        > {
            Err(nomifun_browser_platform::BrowserPlatformError::new(
                nomifun_browser_platform::BrowserErrorCode::BrowserUnavailable,
                "A browser host is not required by this teardown test.",
                false,
                "Do not launch a browser host in this teardown test.",
            ))
        }
    }

    #[cfg(feature = "browser-use")]
    struct BlockingBrowserOwnerLease {
        shutdown_started: tokio::sync::Semaphore,
        shutdown_release: tokio::sync::Semaphore,
        shutdown_calls: AtomicUsize,
        shutdown_error: Option<&'static str>,
    }

    #[cfg(feature = "browser-use")]
    impl BlockingBrowserOwnerLease {
        fn new(shutdown_error: Option<&'static str>) -> Arc<Self> {
            Arc::new(Self {
                shutdown_started: tokio::sync::Semaphore::new(0),
                shutdown_release: tokio::sync::Semaphore::new(0),
                shutdown_calls: AtomicUsize::new(0),
                shutdown_error,
            })
        }

        async fn wait_until_shutdown_started(&self) {
            self.shutdown_started.acquire().await.unwrap().forget();
        }

        fn release_shutdown(&self) {
            self.shutdown_release.add_permits(1);
        }
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl crate::BrowserOwnerLeaseGuard for BlockingBrowserOwnerLease {
        fn revoke(&self) {}

        async fn revoke_and_wait(&self) -> Result<(), AppError> {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            self.shutdown_started.add_permits(1);
            self.shutdown_release.acquire().await.unwrap().forget();
            match self.shutdown_error {
                Some(message) => Err(AppError::Internal(message.to_owned())),
                None => Ok(()),
            }
        }
    }

    #[cfg(feature = "browser-use")]
    struct ControlledBrowserOwnerLease {
        flight_started: AtomicBool,
        flight_starts: AtomicUsize,
        waiter_started: tokio::sync::Semaphore,
        completion: std::sync::OnceLock<Option<&'static str>>,
        completed: tokio::sync::Notify,
    }

    #[cfg(feature = "browser-use")]
    impl ControlledBrowserOwnerLease {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                flight_started: AtomicBool::new(false),
                flight_starts: AtomicUsize::new(0),
                waiter_started: tokio::sync::Semaphore::new(0),
                completion: std::sync::OnceLock::new(),
                completed: tokio::sync::Notify::new(),
            })
        }

        fn start_or_join(&self) {
            if self
                .flight_started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.flight_starts.fetch_add(1, Ordering::SeqCst);
            }
        }

        async fn wait_until_waiter_started(&self) {
            self.waiter_started.acquire().await.unwrap().forget();
        }

        fn complete(&self, error: Option<&'static str>) {
            self.completion
                .set(error)
                .expect("the controlled owner cleanup flight completes once");
            self.completed.notify_waiters();
        }
    }

    #[cfg(feature = "browser-use")]
    #[async_trait::async_trait]
    impl crate::BrowserOwnerLeaseGuard for ControlledBrowserOwnerLease {
        fn revoke(&self) {
            self.start_or_join();
        }

        async fn revoke_and_wait(&self) -> Result<(), AppError> {
            self.start_or_join();
            self.waiter_started.add_permits(1);
            loop {
                let completed = self.completed.notified();
                if let Some(error) = self.completion.get() {
                    return match error {
                        Some(message) => Err(AppError::Internal((*message).to_owned())),
                        None => Ok(()),
                    };
                }
                completed.await;
            }
        }
    }

    #[cfg(feature = "browser-use")]
    fn teardown_test_browser_binding<L>(lease: Arc<L>) -> crate::BrowserLaneBinding
    where
        L: crate::BrowserOwnerLeaseGuard + 'static,
    {
        teardown_test_browser_binding_with_hub(lease).0
    }

    #[cfg(feature = "browser-use")]
    fn teardown_test_browser_binding_with_hub<L>(lease: Arc<L>) -> (
        crate::BrowserLaneBinding,
        nomifun_browser_platform::BrowserSessionHub,
        nomifun_browser_platform::OwnerLeaseId,
    )
    where
        L: crate::BrowserOwnerLeaseGuard + 'static,
    {
        use std::collections::BTreeSet;

        let hub = nomifun_browser_platform::BrowserSessionHub::new(
            Arc::new(RejectingBrowserHostFactory),
            nomifun_browser_platform::HubConfig::default(),
        );
        let owner = hub
            .issue_owner_lease(
                "teardown-user",
                Some("teardown-conversation".to_owned()),
                "teardown-runtime",
            )
            .expect("teardown test owner lease should be issued");
        let lease_id = owner.lease_id.clone();
        let client = hub
            .bind(nomifun_browser_platform::CallerIdentity {
                user_id: owner.user_id,
                conversation_id: owner.conversation_id,
                runtime_instance_id: owner.runtime_instance_id,
                agent_id: None,
                companion_id: None,
                execution_id: None,
                step_id: None,
                attempt_id: None,
                remote_connection_id: None,
                surface: nomifun_browser_platform::BrowserSurface::Native,
                owner_lease_id: owner.lease_id,
                capability_expires_at_ms: owner.expires_at_ms,
                allowed_operations: BTreeSet::from([
                    nomifun_browser_platform::BrowserOperationKind::Manage,
                ]),
            })
            .expect("teardown test caller should bind");
        (
            crate::BrowserLaneBinding::new(client, lease),
            hub,
            lease_id,
        )
    }

    #[cfg(feature = "browser-use")]
    async fn assert_failed_stage_waits_for_browser_shutdown(
        kill: Result<(), AppError>,
        mcp: Result<(), AppError>,
        process: Result<(), AppError>,
        expected_error: &'static str,
    ) {
        let lease = BlockingBrowserOwnerLease::new(None);
        let binding = teardown_test_browser_binding(Arc::clone(&lease));
        let mut teardown = Box::pin(finish_nomi_teardown(NomiTeardownResults {
            kill,
            mcp,
            process,
            browser_lane_binding: Some(binding),
            ssh_lease: None,
        }));

        tokio::select! {
            biased;
            result = &mut teardown => {
                panic!("teardown returned before Browser owner shutdown completed: {result:?}");
            }
            _ = lease.wait_until_shutdown_started() => {}
        }
        assert_eq!(lease.shutdown_calls.load(Ordering::SeqCst), 1);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                teardown.as_mut(),
            )
            .await
            .is_err(),
            "teardown must remain pending while Browser owner shutdown is pending"
        );

        lease.release_shutdown();
        let error = teardown
            .await
            .expect_err("the original teardown stage failure must be returned");
        assert!(
            matches!(&error, AppError::Internal(message) if message == expected_error),
            "single-stage failure must retain its exact AppError: {error:?}"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn finish_nomi_teardown_awaits_browser_after_kill_failure() {
        assert_failed_stage_waits_for_browser_shutdown(
            Err(AppError::Internal("kill failed".to_owned())),
            Ok(()),
            Ok(()),
            "kill failed",
        )
        .await;
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn finish_nomi_teardown_awaits_browser_after_mcp_failure() {
        assert_failed_stage_waits_for_browser_shutdown(
            Ok(()),
            Err(AppError::Internal("MCP failed".to_owned())),
            Ok(()),
            "MCP failed",
        )
        .await;
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn finish_nomi_teardown_awaits_browser_after_process_failure() {
        assert_failed_stage_waits_for_browser_shutdown(
            Ok(()),
            Ok(()),
            Err(AppError::Internal("process failed".to_owned())),
            "process failed",
        )
        .await;
    }

    /// A lease that reports whatever the pool would have told it, and counts how
    /// often it was asked. The real one lives in `nomifun-ssh`; the seam is what
    /// this crate can see.
    struct RecordingSshLease {
        release: crate::SshLeaseRelease,
        releases: AtomicUsize,
    }

    impl RecordingSshLease {
        fn new(release: crate::SshLeaseRelease) -> Arc<Self> {
            Arc::new(Self {
                release,
                releases: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::SshSessionLease for RecordingSshLease {
        async fn release(&self) -> crate::SshLeaseRelease {
            self.releases.fetch_add(1, Ordering::SeqCst);
            self.release.clone()
        }
    }

    fn ssh_teardown_results(
        kill: Result<(), AppError>,
        lease: Arc<RecordingSshLease>,
    ) -> NomiTeardownResults {
        NomiTeardownResults {
            kill,
            mcp: Ok(()),
            process: Ok(()),
            #[cfg(feature = "browser-use")]
            browser_lane_binding: None,
            ssh_lease: Some(lease),
        }
    }

    #[tokio::test]
    async fn ssh_lease_is_released_even_when_kill_fails() {
        let lease = RecordingSshLease::new(crate::SshLeaseRelease::Retained {
            detail: "link still connected".to_owned(),
        });

        let error = finish_nomi_teardown(ssh_teardown_results(
            Err(AppError::Internal("kill failed".to_owned())),
            Arc::clone(&lease),
        ))
        .await
        .expect_err("the kill failure must still be reported");

        assert!(
            matches!(&error, AppError::Internal(message) if message == "kill failed"),
            "a failed kill must keep its exact error: {error:?}"
        );
        assert_eq!(
            lease.releases.load(Ordering::SeqCst),
            1,
            "a failed kill must not skip the SSH lease report"
        );
    }

    #[tokio::test]
    async fn a_retained_link_is_not_a_failure() {
        let lease = RecordingSshLease::new(crate::SshLeaseRelease::Retained {
            detail: "link kept for the conversation".to_owned(),
        });

        finish_nomi_teardown(ssh_teardown_results(Ok(()), Arc::clone(&lease)))
            .await
            .expect("a deliberately retained link is the normal model-switch outcome");
        assert_eq!(lease.releases.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_reaped_link_is_not_a_failure() {
        let lease = RecordingSshLease::new(crate::SshLeaseRelease::Reaped {
            detail: "remote shell exited with status 0".to_owned(),
        });

        finish_nomi_teardown(ssh_teardown_results(Ok(()), Arc::clone(&lease)))
            .await
            .expect("a proven close is a successful teardown");
        assert_eq!(lease.releases.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_lost_ssh_link_is_a_teardown_failure() {
        let lease = RecordingSshLease::new(crate::SshLeaseRelease::Lost {
            detail: "link vanished without exit evidence".to_owned(),
        });

        let error = finish_nomi_teardown(ssh_teardown_results(Ok(()), Arc::clone(&lease)))
            .await
            .expect_err("a link let go of without proof is not a clean teardown");

        let AppError::Internal(message) = error else {
            panic!("a lost SSH link must be an Internal teardown failure");
        };
        assert!(
            message.contains("link vanished without exit evidence"),
            "the failure must say what could not be proven: {message}"
        );
        assert_eq!(lease.releases.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_lost_ssh_link_is_aggregated_with_prior_failures() {
        let lease = RecordingSshLease::new(crate::SshLeaseRelease::Lost {
            detail: "transport gone".to_owned(),
        });

        let error = finish_nomi_teardown(ssh_teardown_results(
            Err(AppError::Internal("kill failed".to_owned())),
            Arc::clone(&lease),
        ))
        .await
        .expect_err("both failures should be reported");

        let AppError::Internal(message) = error else {
            panic!("the primary Internal error variant must be preserved");
        };
        assert!(message.contains("kill failed"));
        assert!(
            message.contains("SSH session link"),
            "the aggregate must name the SSH stage: {message}"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn finish_nomi_teardown_aggregates_browser_and_prior_failures() {
        let lease = BlockingBrowserOwnerLease::new(Some("browser failed"));
        let binding = teardown_test_browser_binding(Arc::clone(&lease));
        lease.release_shutdown();

        let error = finish_nomi_teardown(NomiTeardownResults {
            kill: Err(AppError::Internal("kill failed".to_owned())),
            mcp: Err(AppError::Internal("MCP failed".to_owned())),
            process: Err(AppError::Internal("process failed".to_owned())),
            browser_lane_binding: Some(binding),
            ssh_lease: None,
        })
        .await
        .expect_err("all teardown failures should be reported");

        let AppError::Internal(message) = error else {
            panic!("the primary Internal error variant must be preserved");
        };
        assert!(message.contains("kill failed"));
        assert!(message.contains("MCP: Internal error: MCP failed"));
        assert!(message.contains("process tree: Internal error: process failed"));
        assert!(message.contains("Browser owner lease: Internal error: browser failed"));
        assert_eq!(lease.shutdown_calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn turn_boundary_close_joins_in_flight_owner_revocation() {
        // The kill() race: the exact-owner revocation flight has invalidated
        // the lease, but its Chromium cleanup proof is still pending when the
        // turn boundary attempts owner-scoped close_all.
        let lease = ControlledBrowserOwnerLease::new();
        let (binding, hub, lease_id) =
            teardown_test_browser_binding_with_hub(Arc::clone(&lease));
        binding.revoke();
        hub.close_owner_lease(&lease_id)
            .await
            .expect("the simulated revocation flight should invalidate the owner lease");

        let mut close_turn_lanes = Box::pin(binding.close_turn_lanes());
        tokio::select! {
            biased;
            result = &mut close_turn_lanes => {
                panic!("turn cleanup returned before exact-owner revocation completed: {result:?}");
            }
            _ = lease.wait_until_waiter_started() => {}
        }
        assert_eq!(
            lease.flight_starts.load(Ordering::SeqCst),
            1,
            "the expired-lease branch must join the existing exact-owner flight"
        );

        lease.complete(None);
        close_turn_lanes
            .await
            .expect("the completed exact-owner cleanup proof should satisfy the turn boundary");
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn termination_guard_drop_emits_finish_after_revoked_lease_browser_cleanup() {
        // End-to-end kill race at the guard level: an armed guard drop whose
        // close_turn_lanes hits the already-revoked owner lease must still
        // publish the terminal event and complete the teardown fence, because
        // the Hub-owned revocation flight is the cleanup authority.
        let lease = ControlledBrowserOwnerLease::new();
        let (binding, hub, lease_id) =
            teardown_test_browser_binding_with_hub(Arc::clone(&lease));
        binding.revoke();
        hub.close_owner_lease(&lease_id)
            .await
            .expect("the simulated revocation flight should invalidate the owner lease");

        let rt = AgentRuntimeState::new("c-guard-kill-race", "/w", 16);
        let mut rx = rt.subscribe();
        let backend_output_sink = Arc::new(BackendOutputSink::new(rt.event_sender()));
        let turn = rt.reset_for_new_turn(ConversationStatus::Running);
        let active_turn = Arc::new(std::sync::Mutex::new(Some(turn)));
        let fence = Arc::new(TurnTeardownFence::new());
        {
            let _g = TurnTerminationGuard {
                runtime: rt.clone(),
                turn,
                active_turn: Arc::clone(&active_turn),
                lifecycle_gate: Arc::new(std::sync::Mutex::new(())),
                steering_inbox: Arc::new(std::sync::Mutex::new(
                    std::collections::VecDeque::new(),
                )),
                backend_output_sink,
                process_supervisor: None,
                mcp_managers: Vec::new(),
                turn_teardown_fence: Arc::clone(&fence),
                accepted_turn_recovery_required: Arc::new(AtomicBool::new(false)),
                browser_lane_binding: Some(binding),
                armed: true,
            };
        }
        lease.wait_until_waiter_started().await;
        assert!(
            matches!(rx.try_recv(), Err(broadcast::error::TryRecvError::Empty)),
            "the terminal must not be published while owner cleanup is pending"
        );
        lease.complete(None);
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            rx.recv(),
        )
        .await
        .expect("armed drop must publish a terminal event despite the revoked lease")
        .expect("terminal event channel closed unexpectedly");
        assert!(
            matches!(event, AgentStreamEvent::Finish(_)),
            "expected Finish after browser cleanup, got {event:?}"
        );
        assert_eq!(rt.status(), Some(ConversationStatus::Finished));
        assert!(active_turn.lock().unwrap().is_none());
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn termination_guard_withholds_finish_when_expired_owner_cleanup_fails() {
        let lease = ControlledBrowserOwnerLease::new();
        let (binding, hub, lease_id) =
            teardown_test_browser_binding_with_hub(Arc::clone(&lease));
        binding.revoke();
        hub.close_owner_lease(&lease_id)
            .await
            .expect("the simulated revocation flight should invalidate the owner lease");

        let rt = AgentRuntimeState::new("c-guard-kill-race-failure", "/w", 16);
        let mut rx = rt.subscribe();
        let backend_output_sink = Arc::new(BackendOutputSink::new(rt.event_sender()));
        let turn = rt.reset_for_new_turn(ConversationStatus::Running);
        let active_turn = Arc::new(std::sync::Mutex::new(Some(turn)));
        let mut guard = TurnTerminationGuard {
            runtime: rt.clone(),
            turn,
            active_turn: Arc::clone(&active_turn),
            lifecycle_gate: Arc::new(std::sync::Mutex::new(())),
            steering_inbox: Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            backend_output_sink,
            process_supervisor: None,
            mcp_managers: Vec::new(),
            turn_teardown_fence: Arc::new(TurnTeardownFence::new()),
            accepted_turn_recovery_required: Arc::new(AtomicBool::new(false)),
            browser_lane_binding: Some(binding),
            armed: true,
        };

        let mut terminalize = Box::pin(guard.terminalize(|runtime, turn| {
            runtime.emit_finish_for_turn(
                turn,
                None,
                Some(TurnStopReason::Cancelled),
            )
        }));
        tokio::select! {
            biased;
            result = &mut terminalize => {
                panic!("terminalization returned before exact-owner revocation completed: {result:?}");
            }
            _ = lease.wait_until_waiter_started() => {}
        }
        assert!(
            matches!(rx.try_recv(), Err(broadcast::error::TryRecvError::Empty)),
            "the terminal must not be published while owner cleanup is pending"
        );

        lease.complete(Some("owner cleanup failed"));
        let error = terminalize
            .as_mut()
            .await
            .expect_err("failed exact-owner cleanup must fail terminalization");
        assert!(
            matches!(&error, AppError::Internal(message) if message == "owner cleanup failed"),
            "the exact owner cleanup failure must be propagated: {error:?}"
        );
        assert!(
            matches!(rx.try_recv(), Err(broadcast::error::TryRecvError::Empty)),
            "a failed owner cleanup proof must withhold Finish"
        );
        assert_eq!(rt.status(), Some(ConversationStatus::Running));
        assert_eq!(*active_turn.lock().unwrap(), Some(turn));

        // The result-bearing path was exercised directly. Suppress the Drop
        // backstop so it cannot schedule a second terminalization attempt.
        drop(terminalize);
        guard.armed = false;
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn idle_kill_terminal_waits_for_browser_cleanup_proof() {
        // The idle-kill fence must publish its cancelled terminal only after
        // the Hub-owned Browser owner cleanup proof, not merely after the
        // cleanup request was issued.
        let lease = BlockingBrowserOwnerLease::new(None);
        let binding = teardown_test_browser_binding(Arc::clone(&lease));
        let rt = AgentRuntimeState::new("c-idle-kill", "/w", 16);
        let mut rx = rt.subscribe();
        let backend_output_sink = Arc::new(BackendOutputSink::new(rt.event_sender()));

        schedule_nomi_cancelled_terminal_after_process_fence(
            rt.clone(),
            Arc::new(std::sync::Mutex::new(None)),
            Arc::new(std::sync::Mutex::new(())),
            Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            backend_output_sink,
            None,
            Vec::new(),
            Some(binding),
        )
        .expect("idle-kill fence should schedule");

        lease.wait_until_shutdown_started().await;
        assert!(
            matches!(rx.try_recv(), Err(broadcast::error::TryRecvError::Empty)),
            "the terminal must not be published while Browser cleanup is pending"
        );

        lease.release_shutdown();
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            rx.recv(),
        )
        .await
        .expect("terminal must follow the completed Browser cleanup proof")
        .expect("terminal event channel closed unexpectedly");
        assert!(
            matches!(event, AgentStreamEvent::Finish(_)),
            "expected Finish after browser proof, got {event:?}"
        );
        assert_eq!(lease.shutdown_calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn idle_kill_withholds_terminal_when_browser_cleanup_fails() {
        // A failed Browser cleanup proof retains the non-terminal quarantine;
        // the result-bearing kill_and_wait path surfaces the error instead.
        let lease = BlockingBrowserOwnerLease::new(Some("browser failed"));
        let binding = teardown_test_browser_binding(Arc::clone(&lease));
        let rt = AgentRuntimeState::new("c-idle-kill-fail", "/w", 16);
        let mut rx = rt.subscribe();
        let backend_output_sink = Arc::new(BackendOutputSink::new(rt.event_sender()));

        schedule_nomi_cancelled_terminal_after_process_fence(
            rt.clone(),
            Arc::new(std::sync::Mutex::new(None)),
            Arc::new(std::sync::Mutex::new(())),
            Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            backend_output_sink,
            None,
            Vec::new(),
            Some(binding),
        )
        .expect("idle-kill fence should schedule");

        lease.wait_until_shutdown_started().await;
        lease.release_shutdown();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                rx.recv(),
            )
            .await
            .is_err(),
            "a failed Browser cleanup proof must withhold the idle terminal"
        );
    }

    fn make_test_config() -> NomiResolvedConfig {
        NomiResolvedConfig {
            provider: "anthropic".into(),
            api_key: "sk-test-key".into(),
            model: "claude-sonnet-4-20250514".into(),
            base_url: None,
            system_prompt: None,
            output_ceiling: Some(4096),
            max_turns: None,
            context_limit: None,
            compat_overrides: Default::default(),
            session_directory: std::env::temp_dir().join("nomi-test-sessions"),
            session_mode: None,
            extra_mcp_servers: std::collections::HashMap::new(),
            loopback_capability_leases: Default::default(),
            bedrock_config: None,
            computer_use: false,
            browser_use: false,
            browser_source: "managed".to_owned(),
            browser_full_power: false,
            browser_persistent_login: false,
            browser_site_memory: false,
            browser_takeover: false,
            browser_unrestricted_approval: false,
            browser_visual_fallback: false,
            goal: None,
            persistent_login_key: None,
            owner_token: None,
            install_embedded_agent_execution: true,
            allowed_tools: Vec::new(),
            write_root: None,
        }
    }

    #[test]
    fn restricted_image_session_does_not_misreport_missing_catalog_configuration() {
        let message = image_generation_unavailable_message(
            ImageGenerationAvailability::NotEntitled,
            true,
        );
        assert!(message.contains("受限 Agent 会话"));
        assert!(!message.contains(IMAGE_MODEL_MANAGEMENT_LINK));
        assert!(!message.contains("尚未配置"));

        let failed = image_generation_unavailable_message(
            ImageGenerationAvailability::DiscoveryFailed,
            true,
        );
        assert!(failed.contains("无法核验"));
        assert!(!failed.contains(IMAGE_MODEL_MANAGEMENT_LINK));
        assert!(!failed.contains("尚未配置"));
    }

    #[test]
    fn model_intent_cannot_grant_browser_authority_and_plan_mode_never_routes_execution() {
        assert_eq!(
            host_validated_image_intent(
                ImageGenerationIntent::ExplicitExternal,
                ImageGenerationIntent::None,
                "create an image for my website",
                None,
                false,
            ),
            ImageGenerationIntent::Creation,
            "an output destination is not Browser execution authority"
        );
        assert_eq!(
            host_validated_image_intent(
                ImageGenerationIntent::ExplicitExternal,
                ImageGenerationIntent::None,
                "Use the browser to make something visually surprising",
                None,
                false,
            ),
            ImageGenerationIntent::ExplicitExternal,
        );
        assert_eq!(
            host_validated_image_intent(
                ImageGenerationIntent::Creation,
                ImageGenerationIntent::Creation,
                "生成一张学生图片",
                None,
                true,
            ),
            ImageGenerationIntent::None,
            "plan mode must not create an impossible execution/artifact obligation"
        );
    }

    #[test]
    fn ambiguous_visual_signals_use_semantic_pass_without_double_calling_unrelated_chat() {
        assert!(should_run_image_intent_model(
            ImageGenerationIntent::None,
            None,
            "surprise me visually",
            false,
            false,
        ));
        assert!(should_run_image_intent_model(
            ImageGenerationIntent::None,
            None,
            "Give me cat art",
            false,
            false,
        ));
        assert!(!should_run_image_intent_model(
            ImageGenerationIntent::Discussion,
            None,
            "explain the image generation route",
            false,
            false,
        ));
        assert!(!should_run_image_intent_model(
            ImageGenerationIntent::None,
            None,
            "How should I structure this Rust module?",
            false,
            false,
        ));
        assert!(!should_run_image_intent_model(
            ImageGenerationIntent::None,
            None,
            "What is shown?",
            true,
            false,
        ));
        assert!(should_run_image_intent_model(
            ImageGenerationIntent::None,
            None,
            "make it watercolor",
            true,
            false,
        ));
        assert!(!should_run_image_intent_model(
            ImageGenerationIntent::None,
            None,
            "surprise me visually",
            false,
            true,
        ));
    }

    #[test]
    fn code_native_visual_requests_never_route_to_image_generation() {
        for request in [
            "画布项目助手合并需求",
            "请实现 canvas 图表渲染代码",
            "绘制这个流程图的 Mermaid 源码",
            "创建一个 UI 图标组件",
            "Implement this canvas chart in TypeScript",
            "Create the Mermaid source for a flowchart",
            "Build a React UI icon component",
            "Fix the SVG diagram rendering code",
        ] {
            let direct = classify_image_generation_intent(request);
            assert_eq!(
                host_validated_image_intent(direct, direct, request, None, false),
                ImageGenerationIntent::None,
                "code-native visual request was routed as image generation: {request}"
            );
            assert!(
                !should_run_image_intent_model(direct, None, request, false, false),
                "code-native visual request must not spend an image-intent model pass: {request}"
            );
        }

        for request in ["生成一张水彩海报", "Create a watercolor poster"] {
            let direct = classify_image_generation_intent(request);
            assert_eq!(
                host_validated_image_intent(direct, direct, request, None, false),
                ImageGenerationIntent::Creation,
                "a real visual-asset request must keep the native image route: {request}"
            );
        }
    }

    #[test]
    fn creative_studio_planning_envelope_never_routes_to_image_generation() {
        let request = r#"{"kind":"nomifun.creative-studio.planning-turn","version":1,"userRequest":"请基于当前画布生成一个简洁的项目规划文本节点提案，并提供可应用到画布的操作。","selectedSkills":["creative-studio-canvas"],"canvasContext":{"kind":"nomifun.creative-studio.canvas-context","version":1,"canvasId":"01a02d92-7bd6-7713-b5bc-8e7bbfdbc15a","canvasRevision":"129","selectedNodeIds":[],"nodes":[],"connections":[],"totalNodeCount":1,"totalConnectionCount":0,"truncated":false},"responseContract":{"mode":"plan-and-propose","allowedArtifactKinds":["nomifun.creative-studio.canvas-ops/v1"],"requiresUserApproval":true,"forbiddenActions":["delete-node","media-generation"]}}"#;
        let direct = classify_image_generation_intent(request);

        assert_eq!(direct, ImageGenerationIntent::None);
        assert!(!should_run_image_intent_model(
            direct,
            Some(ImageGenerationIntent::Creation),
            request,
            false,
            false,
        ));
        assert_eq!(
            host_validated_image_intent(
                direct,
                direct,
                request,
                Some(ImageGenerationIntent::Creation),
                false,
            ),
            ImageGenerationIntent::None,
        );
    }

    #[test]
    fn strict_image_routes_do_not_mount_forced_knowledge_context() {
        let image_only = HashSet::from([IMAGE_GEN_TOOL_NAME.to_owned()]);
        let no_tools = HashSet::new();
        let knowledge_only = HashSet::from(["knowledge_search".to_owned()]);

        assert!(!route_allows_knowledge_context(Some(&image_only)));
        assert!(!route_allows_knowledge_context(Some(&no_tools)));
        assert!(route_allows_knowledge_context(Some(&knowledge_only)));
        assert!(route_allows_knowledge_context(None));
    }

    struct ScriptedProvider {
        calls: AtomicUsize,
        responses: Vec<Vec<LlmEvent>>,
        requests: std::sync::Mutex<Vec<LlmRequest>>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<Vec<LlmEvent>>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                responses,
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn requests(&self) -> Vec<LlmRequest> {
            self.requests
                .lock()
                .expect("request capture lock poisoned")
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn stream(
            &self,
            request: &LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
            self.requests
                .lock()
                .expect("request capture lock poisoned")
                .push(request.clone());
            let index = self.calls.fetch_add(1, Ordering::SeqCst);
            let events = self
                .responses
                .get(index)
                .unwrap_or_else(|| panic!("unexpected provider call {index}"))
                .clone();
            let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
            tokio::spawn(async move {
                for event in events {
                    let _ = tx.send(event).await;
                }
            });
            Ok(rx)
        }
    }

    struct BlockingProvider {
        calls: AtomicUsize,
        called: tokio::sync::Semaphore,
        senders: std::sync::Mutex<Vec<tokio::sync::mpsc::Sender<LlmEvent>>>,
        requests: std::sync::Mutex<Vec<LlmRequest>>,
    }

    impl BlockingProvider {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                called: tokio::sync::Semaphore::new(0),
                senders: std::sync::Mutex::new(Vec::new()),
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<LlmRequest> {
            self.requests
                .lock()
                .expect("request capture lock poisoned")
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for BlockingProvider {
        async fn stream(
            &self,
            request: &LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
            self.requests
                .lock()
                .expect("request capture lock poisoned")
                .push(request.clone());
            self.calls.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.senders.lock().expect("sender lock poisoned").push(tx);
            self.called.add_permits(1);
            Ok(rx)
        }
    }

    struct HangingTool {
        started: Arc<tokio::sync::Semaphore>,
    }

    #[async_trait::async_trait]
    impl Tool for HangingTool {
        fn name(&self) -> &str {
            "hanging_tool"
        }

        fn description(&self) -> &str {
            "A test tool that never returns"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            false
        }

        async fn execute(&self, _input: serde_json::Value) -> ToolResult {
            self.started.add_permits(1);
            std::future::pending::<ToolResult>().await
        }

        fn category(&self) -> ToolCategory {
            ToolCategory::Exec
        }
    }

    /// Advertises the production `Write` route for stream-boundary tests that
    /// never commit or execute the call. Tool progress from an unadvertised
    /// name is intentionally rejected by the engine, so those fixtures must
    /// model the same request authority as a real desktop Nomi session.
    struct PreviewOnlyWriteTool;

    #[async_trait::async_trait]
    impl Tool for PreviewOnlyWriteTool {
        fn name(&self) -> &str {
            "Write"
        }

        fn description(&self) -> &str {
            "Test-only advertised write tool"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["file_path", "content"]
            })
        }

        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            false
        }

        async fn execute(&self, _input: serde_json::Value) -> ToolResult {
            ToolResult::error("PreviewOnlyWriteTool must not be executed")
        }

        fn category(&self) -> ToolCategory {
            ToolCategory::Exec
        }
    }

    struct MissingImageArtifactTool;

    #[async_trait::async_trait]
    impl Tool for MissingImageArtifactTool {
        fn name(&self) -> &str {
            "image_gen"
        }

        fn description(&self) -> &str {
            "Test-only image generator that falsely reports text success"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            true
        }

        async fn execute(&self, _input: serde_json::Value) -> ToolResult {
            ToolResult::text("generated successfully")
        }

        fn category(&self) -> ToolCategory {
            ToolCategory::Info
        }
    }

    struct SequencedImageToolDiscovery {
        ready: std::sync::Mutex<std::collections::VecDeque<bool>>,
    }

    struct BlockingImageToolDiscovery {
        started: Arc<tokio::sync::Semaphore>,
    }

    impl SequencedImageToolDiscovery {
        fn new(ready: impl IntoIterator<Item = bool>) -> Self {
            Self {
                ready: std::sync::Mutex::new(ready.into_iter().collect()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ImageGenerationToolDiscovery for SequencedImageToolDiscovery {
        async fn discover_tool(&self) -> Result<Option<Box<dyn Tool>>, AppError> {
            let ready = self
                .ready
                .lock()
                .expect("image discovery sequence lock poisoned")
                .pop_front()
                .expect("image discovery invoked more often than expected");
            Ok(ready.then(|| Box::new(MissingImageArtifactTool) as Box<dyn Tool>))
        }
    }

    #[async_trait::async_trait]
    impl ImageGenerationToolDiscovery for BlockingImageToolDiscovery {
        async fn discover_tool(&self) -> Result<Option<Box<dyn Tool>>, AppError> {
            self.started.add_permits(1);
            std::future::pending().await
        }
    }

    struct BrowserScreenshotOnlyTool;

    #[async_trait::async_trait]
    impl Tool for BrowserScreenshotOnlyTool {
        fn name(&self) -> &str {
            "browserScreenshot"
        }

        fn description(&self) -> &str {
            "Test-only observational browser screenshot"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            true
        }

        async fn execute(&self, _input: serde_json::Value) -> ToolResult {
            const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
            ToolResult::text("browser screenshot captured").with_images(vec![
                nomi_types::tool::ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                },
            ])
        }

        fn category(&self) -> ToolCategory {
            ToolCategory::Info
        }
    }

    struct HangingKnowledgeSink {
        started: Arc<tokio::sync::Semaphore>,
    }

    #[async_trait::async_trait]
    impl nomi_agent::knowledge_tools::KnowledgeRetrievalSink for HangingKnowledgeSink {
        async fn search(
            &self,
            _kb_ids: &[nomifun_common::KnowledgeBaseId],
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<nomi_agent::knowledge_tools::KnowledgeHit>, String> {
            self.started.add_permits(1);
            std::future::pending().await
        }

        async fn read_document(
            &self,
            _kb_ids: &[nomifun_common::KnowledgeBaseId],
            _handle: &str,
        ) -> Result<String, String> {
            Err("not used by this test".to_owned())
        }
    }

    fn make_test_engine_config() -> Config {
        let mut config = Config::resolve(&CliArgs {
            provider: Some("anthropic".into()),
            api_key: Some("sk-test-key".into()),
            base_url: None,
            model: Some("claude-sonnet-4-20250514".into()),
            max_tokens: Some(4096),
            max_turns: Some(10),
            system_prompt: None,
            profile: None,
            auto_approve: true,
            project_dir: Some(PathBuf::from("/project")),
        })
        .expect("test config should resolve");
        config.session.enabled = false;
        config
    }

    #[test]
    fn declared_provider_token_budget_is_applied_before_engine_bootstrap() {
        let mut config = make_test_engine_config();

        apply_provider_token_budget(&mut config, Some(4096), Some(8192)).unwrap();

        assert_eq!(config.compact.context_window, 4096);
        assert_eq!(config.output_max_tokens, Some(1024));
        assert_eq!(config.compact.output_reserve, 1024);
        assert_eq!(config.compact.autocompact_buffer, 512);
        assert_eq!(config.compact.emergency_buffer, 256);
    }

    #[test]
    fn absent_capability_ceiling_erases_desktop_local_config_for_optional_protocol() {
        let mut config = make_test_engine_config();
        config.provider = nomi_config::config::ProviderType::OpenAI;
        config.output_max_tokens = Some(8192);

        apply_provider_token_budget(&mut config, Some(200_000), None).unwrap();

        assert_eq!(config.output_max_tokens, None);
        assert_eq!(config.compact.output_reserve, 20_000);
    }

    #[test]
    fn required_protocol_rejects_an_absent_capability_ceiling() {
        let mut config = make_test_engine_config();

        let error = apply_provider_token_budget(&mut config, Some(200_000), None)
            .expect_err("Anthropic requires an explicit output ceiling");

        assert!(error.to_string().contains("Max output tokens"));
    }

    #[derive(Clone)]
    struct CapturedLogWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture_logs(run: impl FnOnce()) -> String {
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer_output = Arc::clone(&output);
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_writer(move || CapturedLogWriter(Arc::clone(&writer_output)))
            .finish();
        tracing::subscriber::with_default(subscriber, run);

        let bytes = output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        String::from_utf8(bytes).expect("test tracing output is UTF-8")
    }

    #[test]
    fn an_undeclared_context_window_is_reported_once_per_provider_model() {
        // A dedicated instance rather than the process singleton: the shared
        // `global()` set is consumed by every other test that builds a manager
        // from a capability without a context limit, which would make an
        // order-dependent assertion here.
        let log = AssumedContextWindowLog::new();

        // (a) The silent fallback is now visible, with the identity and the
        // window that will actually be used.
        let logs = capture_logs(|| {
            assert!(report_assumed_context_window(
                &log,
                None,
                "Local vLLM",
                "qwen3-32b",
                200_000,
            ));
        });
        assert!(logs.contains("WARN"), "got: {logs}");
        assert!(logs.contains("Local vLLM"), "got: {logs}");
        assert!(logs.contains("qwen3-32b"), "got: {logs}");
        assert!(logs.contains("assumed_context_window=200000"), "got: {logs}");
        assert!(logs.contains("Context limit"), "got: {logs}");

        // (b) The same pair on a later turn/runtime rebuild stays quiet.
        let repeats = capture_logs(|| {
            for _ in 0..5 {
                assert!(!report_assumed_context_window(
                    &log,
                    None,
                    "Local vLLM",
                    "qwen3-32b",
                    200_000,
                ));
            }
        });
        assert!(repeats.is_empty(), "got: {repeats}");

        // A second, differently sized model behind the same provider is still
        // its own diagnostic, and so is the same model id on another provider.
        for (provider, model) in [("Local vLLM", "qwen3-4b"), ("Other gateway", "qwen3-32b")] {
            let logs = capture_logs(|| {
                assert!(report_assumed_context_window(
                    &log, None, provider, model, 200_000,
                ));
            });
            assert!(logs.contains(model), "got: {logs}");
        }
    }

    #[test]
    fn a_declared_context_window_is_never_reported_as_assumed() {
        let log = AssumedContextWindowLog::new();
        let logs = capture_logs(|| {
            assert!(!report_assumed_context_window(
                &log,
                Some(32_768),
                "Local vLLM",
                "qwen3-32b",
                32_768,
            ));
        });
        assert!(logs.is_empty(), "got: {logs}");

        // The predicate matches `resolve_context_window`, which also treats an
        // explicit zero as unset — that path falls back just as silently.
        assert!(assumes_default_context_window(None));
        assert!(assumes_default_context_window(Some(0)));
        assert!(!assumes_default_context_window(Some(1)));
        assert!(!assumes_default_context_window(Some(200_000)));
    }

    fn make_agent_with_provider(provider: Arc<dyn LlmProvider>) -> NomiAgentManager {
        make_agent_with_provider_and_max_turns(provider, Some(10))
    }

    fn make_agent_with_provider_and_max_turns(
        provider: Arc<dyn LlmProvider>,
        max_turns: Option<usize>,
    ) -> NomiAgentManager {
        let runtime = AgentRuntimeState::new("conv-auto-continue", "/project", 128);
        let backend_output_sink = Arc::new(BackendOutputSink::new(runtime.event_sender()));
        let output: Arc<dyn OutputSink> = backend_output_sink.clone();
        let mut config = make_test_engine_config();
        config.max_turns = max_turns;
        let mut engine = AgentEngine::new_with_provider(
            provider,
            config.clone(),
            ToolRegistry::new(),
            output,
            PathBuf::from("/project"),
        );
        let approval_manager = Arc::new(ToolApprovalManager::new());
        let confirmations = Arc::new(std::sync::RwLock::new(Vec::new()));
        let protocol_sink = BackendProtocolSink::new(
            runtime.event_sender(),
            confirmations.clone(),
        );
        engine.set_approval_manager(approval_manager.clone());
        engine.set_protocol_writer(Arc::new(protocol_sink));

        NomiAgentManager {
            runtime,
            backend_output_sink,
            engine: Mutex::new(engine),
            process_supervisor: None,
            turn_teardown_fence: Arc::new(TurnTeardownFence::new()),
            slash_commands: Vec::new(),
            mcp_managers: Vec::new(),
            loopback_capability_leases: Default::default(),
            #[cfg(feature = "browser-use")]
            browser_lane_binding: None,
            ssh_lease: None,
            approval_manager,
            confirmations,
            turn_cancel: std::sync::Mutex::new(tokio_util::sync::CancellationToken::new()),
            active_turn: Arc::new(std::sync::Mutex::new(None)),
            lifecycle_gate: Arc::new(std::sync::Mutex::new(())),
            turn_gate: Mutex::new(()),
            closing: AtomicBool::new(false),
            steering_inbox: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            system_resource_inbox: Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            distill_dir: None,
            image_read_root: None,
            distill_cfg: Arc::new(config),
            knowledge_prelude: std::sync::Mutex::new(None),
            knowledge_auto_rag: None,
            image_generation_availability: std::sync::RwLock::new(
                ImageGenerationAvailability::NoConfiguredModel,
            ),
            image_generation_discovery: None,
            image_generation_response_in_chinese: false,
        }
    }

    #[tokio::test]
    async fn next_turn_refresh_promotes_none_to_ready_in_the_same_runtime() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        }]]));
        let mut agent = make_agent_with_provider(provider.clone());
        agent.image_generation_discovery = Some(Arc::new(SequencedImageToolDiscovery::new([
            true,
        ])));

        assert_eq!(
            agent.image_generation_availability(),
            ImageGenerationAvailability::NoConfiguredModel
        );
        assert!(
            !agent
                .engine
                .get_mut()
                .tool_names()
                .iter()
                .any(|name| name == IMAGE_GEN_TOOL_NAME)
        );

        let result = agent
            .send_message(SendMessageData {
                content: "generate an image of a lighthouse".into(),
                msg_id: "msg-live-image-model-enabled".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await;

        assert!(
            result.is_err(),
            "the test provider intentionally returns no verified image receipt"
        );
        assert_eq!(provider.calls(), 1);
        assert_eq!(
            provider.requests()[0]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec![IMAGE_GEN_TOOL_NAME]
        );
        assert_eq!(
            agent.image_generation_availability(),
            ImageGenerationAvailability::Ready
        );
    }

    #[tokio::test]
    async fn next_turn_refresh_demotes_ready_to_none_in_the_same_runtime() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let mut agent = make_agent_with_provider(provider.clone());
        *agent.image_generation_availability.get_mut().unwrap() =
            ImageGenerationAvailability::Ready;
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(MissingImageArtifactTool))
        );
        agent.image_generation_discovery = Some(Arc::new(SequencedImageToolDiscovery::new([
            false,
        ])));

        agent
            .send_message(SendMessageData {
                content: "generate an image of a lighthouse".into(),
                msg_id: "msg-live-image-model-disabled".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(provider.calls(), 0);
        assert_eq!(
            agent.image_generation_availability(),
            ImageGenerationAvailability::NoConfiguredModel
        );
        assert!(
            !agent
                .engine
                .lock()
                .await
                .tool_names()
                .iter()
                .any(|name| name == IMAGE_GEN_TOOL_NAME)
        );
    }

    #[tokio::test]
    async fn idle_system_resource_notice_waits_for_next_real_model_call() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        }]]));
        let agent = make_agent_with_provider(provider.clone());

        let delivery = agent
            .notify_system_resource(
                "terminal term-idle transitioned to exited (exit_code=0)".to_owned(),
            )
            .unwrap();
        assert_eq!(delivery, SystemResourceNoticeDelivery::NextModelCall);
        assert_eq!(
            provider.calls(),
            0,
            "queuing resource state must not synthesize a model turn"
        );

        agent
            .send_message(SendMessageData {
                content: "continue".into(),
                msg_id: "msg-after-idle-resource".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .system
                .contains("terminal term-idle transitioned to exited (exit_code=0)")
        );
        assert!(
            requests[0].messages.iter().all(|message| {
                message.content.iter().all(|block| {
                    !matches!(
                        block,
                        ContentBlock::Text { text }
                            if text.contains("terminal term-idle transitioned to exited")
                    )
                })
            }),
            "resource state must not be presented as a user message"
        );
    }

    #[test]
    fn running_system_resource_notice_enters_active_runtime_inbox() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let agent = make_agent_with_provider(provider);
        agent.runtime.transition_to(ConversationStatus::Running);

        let delivery = agent
            .notify_system_resource("terminal term-live was closed".to_owned())
            .unwrap();

        assert_eq!(delivery, SystemResourceNoticeDelivery::ActiveTurn);
        assert_eq!(
            agent
                .system_resource_inbox
                .lock()
                .unwrap()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["terminal term-live was closed"]
        );
    }

    fn assert_single_cancelled_finish_without_running_tools(
        agent: &NomiAgentManager,
        rx: &mut broadcast::Receiver<AgentStreamEvent>,
    ) {
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let finish_reasons = events
            .iter()
            .filter_map(|event| match event {
                AgentStreamEvent::Finish(data) => Some(data.stop_reason),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(finish_reasons, vec![Some(TurnStopReason::Cancelled)]);
        assert!(
            !events.iter().any(|event| matches!(
                event,
                AgentStreamEvent::ToolCall(data) if data.status == ToolCallStatus::Running
            )),
            "cancelled preparation must not leave a Running tool card"
        );
        assert_eq!(agent.status(), Some(ConversationStatus::Finished));
    }

    #[tokio::test]
    async fn send_message_files_reach_provider_as_image_content() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        }]]));
        let agent = make_agent_with_provider(provider.clone());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attached.png");
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            16,
            12,
            image::Rgb([40, 80, 120]),
        ))
        .save_with_format(&path, image::ImageFormat::Png)
        .unwrap();

        agent
            .send_message(SendMessageData {
                content: "What is shown?".into(),
                msg_id: "msg-image".into(),
                source_message_id: None,
                files: vec![path.to_string_lossy().into_owned()],
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        let user = requests[0]
            .messages
            .iter()
            .find(|message| message.role == Role::User)
            .expect("provider request should contain the user message");
        assert!(matches!(
            &user.content[..],
            [ContentBlock::Text { text }, ContentBlock::Image { media_type, data }]
                if text == "What is shown?"
                    && media_type == "image/png"
                    && !data.is_empty()
        ));
    }

    #[tokio::test]
    async fn send_message_skips_image_io_for_a_known_text_only_model() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        }]]));
        let runtime = AgentRuntimeState::new("conv-text-only", "/project", 128);
        let backend_output_sink = Arc::new(BackendOutputSink::new(runtime.event_sender()));
        let output: Arc<dyn OutputSink> = backend_output_sink.clone();
        let mut config = make_test_engine_config();
        config.compat.supports_image = Some(false);
        let mut engine = AgentEngine::new_with_provider(
            provider.clone(),
            config.clone(),
            ToolRegistry::new(),
            output,
            PathBuf::from("/project"),
        );
        let approval_manager = Arc::new(ToolApprovalManager::new());
        engine.set_approval_manager(approval_manager.clone());
        let agent = NomiAgentManager {
            runtime,
            backend_output_sink,
            engine: Mutex::new(engine),
            process_supervisor: None,
            turn_teardown_fence: Arc::new(TurnTeardownFence::new()),
            slash_commands: Vec::new(),
            mcp_managers: Vec::new(),
            loopback_capability_leases: Default::default(),
            #[cfg(feature = "browser-use")]
            browser_lane_binding: None,
            ssh_lease: None,
            approval_manager,
            confirmations: Arc::new(std::sync::RwLock::new(Vec::new())),
            turn_cancel: std::sync::Mutex::new(tokio_util::sync::CancellationToken::new()),
            active_turn: Arc::new(std::sync::Mutex::new(None)),
            lifecycle_gate: Arc::new(std::sync::Mutex::new(())),
            turn_gate: Mutex::new(()),
            closing: AtomicBool::new(false),
            steering_inbox: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            system_resource_inbox: Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            distill_dir: None,
            image_read_root: None,
            distill_cfg: Arc::new(config),
            knowledge_prelude: std::sync::Mutex::new(None),
            knowledge_auto_rag: None,
            image_generation_availability: std::sync::RwLock::new(
                ImageGenerationAvailability::NoConfiguredModel,
            ),
            image_generation_discovery: None,
            image_generation_response_in_chinese: false,
        };
        let attachment_dir = tempfile::tempdir().unwrap();
        let missing_image = attachment_dir
            .path()
            .join("missing-text-only-attachment.png")
            .to_string_lossy()
            .into_owned();

        agent
            .send_message(SendMessageData {
                content: "Answer using text only.".into(),
                msg_id: "msg-text-only".into(),
                source_message_id: None,
                files: vec![missing_image],
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .expect("known text-only models should ignore image attachments");

        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        let user = requests[0]
            .messages
            .iter()
            .find(|message| message.role == Role::User)
            .expect("provider request should contain the user message");
        assert!(matches!(
            &user.content[..],
            [ContentBlock::Text { text }] if text == "Answer using text only."
        ));
    }

    /// The observed production shape: a long prose answer cut off at the output
    /// ceiling, with no tool ever called. Restarting it would spend a second and
    /// third full ceiling reproducing the same wall, which is exactly what the
    /// deleted host-side auto-continue did (`output_tokens = 3 × 8192`). The
    /// engine must decline, and the receipt must carry the truthful terminal.
    #[tokio::test]
    async fn a_prose_only_truncation_is_not_restarted_and_finishes_as_max_tokens() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta("partial".into()),
            LlmEvent::Done {
                stop_reason: StopReason::MaxTokens,
                usage: Default::default(),
            },
        ]]));
        let agent = make_agent_with_provider(provider.clone());
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "create the file".into(),
                msg_id: "msg-1".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(
            provider.calls(),
            1,
            "a truncation with no carry-forward evidence must not burn another ceiling"
        );

        let mut completed_turns = 0;
        let mut finish_reason = None;
        let mut streamed = String::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentStreamEvent::TurnCompleted(_) => completed_turns += 1,
                AgentStreamEvent::Finish(data) => finish_reason = data.stop_reason,
                AgentStreamEvent::Text(data) => streamed.push_str(&data.content),
                _ => {}
            }
        }

        assert_eq!(completed_turns, 1);
        assert_eq!(
            finish_reason,
            Some(TurnStopReason::MaxTokens),
            "the turn must report the ceiling it actually hit, not a clean EndTurn"
        );
        assert!(
            streamed.contains("partial"),
            "the already-visible prose stays durable evidence"
        );
    }

    #[tokio::test]
    async fn unbacked_file_completion_emits_metrics_then_one_error_and_never_finish() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta("Created miniapp.html.".into()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: nomi_types::message::TokenUsage {
                    input_tokens: 17,
                    output_tokens: 5,
                    ..Default::default()
                },
            },
        ]]));
        let agent = make_agent_with_provider_and_max_turns(provider.clone(), Some(1));
        let mut rx = agent.subscribe();
        agent
            .steering_inbox
            .lock()
            .unwrap()
            .push_back("irrelevant race-tail text".to_owned());

        let error = agent
            .send_message(SendMessageData {
                content: "Create miniapp.html.".into(),
                msg_id: "msg-unbacked-completion".into(),
                source_message_id: Some("root-unbacked-completion".into()),
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .expect_err("unsupported completion must fail the delivery receipt");

        assert_eq!(
            error.code(),
            Some(AgentErrorCode::UserLlmProviderUnbackedCompletion)
        );
        assert_eq!(provider.calls(), 1, "a typed verdict cannot be race-tail overwritten");
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentStreamEvent::TurnCompleted(_)))
                .count(),
            1,
            "usage accounting is retained exactly once"
        );
        let errors = events
            .iter()
            .filter_map(|event| match event {
                AgentStreamEvent::Error(data) => Some(data),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].code,
            Some(AgentErrorCode::UserLlmProviderUnbackedCompletion)
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentStreamEvent::Finish(_))),
            "a failed delivery must never publish Finish"
        );
        assert!(agent.steering_inbox.lock().unwrap().is_empty());
        assert!(
            agent.engine.lock().await.messages_transcript().is_empty(),
            "the rejected user/assistant/nudge exchange must not pollute history"
        );
    }

    #[tokio::test]
    async fn native_image_request_without_model_short_circuits_without_provider_or_tools() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let mut agent = make_agent_with_provider(provider.clone());
        agent.image_generation_response_in_chinese = true;
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "请生成一张水彩风格的学生图片".into(),
                msg_id: "msg-no-image-model".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(provider.calls(), 0);
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Text(data)
                if data.content.contains(IMAGE_MODEL_MANAGEMENT_LINK)
                    && data.content.contains("没有生成图片")
        )));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::ToolCall(_))));
        assert!(events.iter().any(|event| matches!(event, AgentStreamEvent::Finish(_))));
    }

    #[tokio::test]
    async fn strict_image_route_suppresses_forced_knowledge_prelude_and_projects_authority() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        }]]));
        let mut agent = make_agent_with_provider(provider.clone());
        *agent.image_generation_availability.get_mut().unwrap() =
            ImageGenerationAvailability::Ready;
        *agent.knowledge_prelude.get_mut().unwrap() = Some(
            "[Knowledge bases mounted: private] Call knowledge_search before answering.".into(),
        );
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(MissingImageArtifactTool))
        );

        let send_error = agent
            .send_message(SendMessageData {
                content: "generate an image of a student".into(),
                msg_id: "msg-image-no-kb-promise".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap_err();

        assert_eq!(
            send_error.code(),
            Some(AgentErrorCode::UserLlmProviderEmptyResponse)
        );
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].system.starts_with("## Request-scoped tool authority"));
        assert!(requests[0].system.contains("Declared tools for this request: `image_gen`"));
        let user_text = requests[0]
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .flat_map(|message| &message.content)
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!user_text.contains("Knowledge bases mounted"));
        assert_eq!(
            agent
                .knowledge_prelude
                .lock()
                .unwrap()
                .as_deref(),
            Some("[Knowledge bases mounted: private] Call knowledge_search before answering."),
            "a strict route must not consume the one-shot knowledge prelude"
        );
    }

    #[tokio::test]
    async fn ambiguous_visual_request_uses_typed_pass_then_honest_no_model_response() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta(r#"{"intent":"creation"}"#.into()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            },
        ]]));
        let agent = make_agent_with_provider(provider.clone());
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "Give me cat art".into(),
                msg_id: "msg-semantic-no-model".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(provider.calls(), 1, "only the isolated intent pass may run");
        assert!(provider.requests()[0].tools.is_empty());
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Text(data)
                if data.content.contains(IMAGE_MODEL_MANAGEMENT_LINK)
                    && data.content.contains("No image was generated")
        )));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::ToolCall(_))));
    }

    #[tokio::test]
    async fn semantic_creation_routes_only_to_native_image_tool_never_browser() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::TextDelta(r#"{"intent":"creation"}"#.into()),
                LlmEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    usage: Default::default(),
                },
            ],
            vec![
                LlmEvent::TextDelta("done".into()),
                LlmEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    usage: Default::default(),
                },
            ],
        ]));
        let mut agent = make_agent_with_provider(provider.clone());
        *agent.image_generation_availability.get_mut().unwrap() =
            ImageGenerationAvailability::Ready;
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(MissingImageArtifactTool))
        );

        let result = agent
            .send_message(SendMessageData {
                content: "surprise me visually".into(),
                msg_id: "msg-semantic-native".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await;

        assert!(result.is_err(), "text alone cannot satisfy the image receipt gate");
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].tools.is_empty(), "classifier must be toolless");
        assert_eq!(
            requests[1]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec![IMAGE_GEN_TOOL_NAME],
            "the execution request must expose only the native image tool"
        );
    }

    #[tokio::test]
    async fn explicit_visual_discussion_skips_classifier_and_artifact_obligation() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        }]]));
        let agent = make_agent_with_provider(provider.clone());

        agent
            .send_message(SendMessageData {
                content: "Let's discuss visual systems architecture".into(),
                msg_id: "msg-semantic-discussion".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].tools.is_empty());
    }

    #[tokio::test]
    async fn explicit_visual_explanation_is_a_single_normal_answer_pass() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        }]]));
        let agent = make_agent_with_provider(provider.clone());

        agent
            .send_message(SendMessageData {
                content: "Explain this visual art style".into(),
                msg_id: "msg-semantic-discussion-not-creation".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 1, "an explicit discussion must remain a normal answer turn");
        assert!(requests[0].tools.is_empty());
    }

    #[tokio::test]
    async fn failed_semantic_image_classification_cannot_reopen_browser_tools() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta("not valid routing json".into()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            },
        ]]));
        let mut agent = make_agent_with_provider(provider.clone());
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(BrowserScreenshotOnlyTool))
        );
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "surprise me visually".into(),
                msg_id: "msg-semantic-failure-closed".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 1, "only the isolated classifier may run");
        assert!(requests[0].tools.is_empty(), "classifier must be toolless");
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Text(data) if data.content.contains("No image was generated")
        )));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::ToolCall(_))));
    }

    #[tokio::test]
    async fn semantic_visual_none_or_discussion_is_host_clarified_without_false_success() {
        for classified_intent in ["none", "discussion"] {
            let provider = Arc::new(ScriptedProvider::new(vec![
                vec![
                    LlmEvent::TextDelta(
                        serde_json::json!({"intent": classified_intent}).to_string(),
                    ),
                    LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage: Default::default(),
                    },
                ],
                vec![
                    LlmEvent::TextDelta("The image was generated successfully.".into()),
                    LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage: Default::default(),
                    },
                ],
            ]));
            let mut agent = make_agent_with_provider(provider.clone());
            assert!(
                agent
                    .engine
                    .get_mut()
                    .registry_mut()
                    .register(Box::new(BrowserScreenshotOnlyTool))
            );
            let mut rx = agent.subscribe();

            agent
                .send_message(SendMessageData {
                    content: "surprise me visually".into(),
                    msg_id: format!("msg-semantic-{classified_intent}-closed"),
                    source_message_id: None,
                    files: Vec::new(),
                    inject_skills: Vec::new(),
                    origin: None,
                })
                .await
                .unwrap();

            let requests = provider.requests();
            assert_eq!(
                requests.len(),
                1,
                "classifier={classified_intent} must not reach a free-form main pass"
            );
            assert!(requests[0].tools.is_empty(), "classifier must be toolless");
            let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
            assert!(events.iter().any(|event| matches!(
                event,
                AgentStreamEvent::Text(data)
                    if data.content.contains(IMAGE_MODEL_MANAGEMENT_LINK)
                        && data.content.contains("No image was generated")
                        && !data.content.contains("generated successfully")
            )));
            assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::ToolCall(_))));
        }
    }

    #[tokio::test]
    async fn contextual_image_followup_reuses_the_native_route_without_opening_browser_tools() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let agent = make_agent_with_provider(provider.clone());

        for (index, content) in [
            "generate an image of a student",
            "换成 16:9，按刚才风格重做",
        ]
        .into_iter()
        .enumerate()
        {
            agent
                .send_message(SendMessageData {
                    content: content.into(),
                    msg_id: format!("msg-image-followup-{index}"),
                    source_message_id: None,
                    files: Vec::new(),
                    inject_skills: Vec::new(),
                    origin: None,
                })
                .await
                .unwrap();
        }

        assert_eq!(provider.calls(), 0);
        assert_eq!(
            agent
                .engine
                .lock()
                .await
                .host_context_value(IMAGE_ROUTE_CONTEXT_KEY)
                .as_deref(),
            Some(IMAGE_ROUTE_NATIVE)
        );
        assert!(contextual_image_generation_followup("再来个竖版"));
        assert!(is_context_only_image_followup("再来个竖版"));
        assert!(!is_context_only_image_followup("再生成一张竖版图片"));
    }

    #[tokio::test]
    async fn unsupported_external_followup_short_circuits_before_provider() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let agent = make_agent_with_provider(provider.clone());
        agent.engine.lock().await.set_host_context_value(
            IMAGE_ROUTE_CONTEXT_KEY,
            Some(IMAGE_ROUTE_EXTERNAL),
        );
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "another one in the same site".into(),
                msg_id: "msg-external-image-followup".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(provider.calls(), 0);
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Text(data)
                if data.content.contains("cannot yet persist and verify")
        )));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::ToolCall(_))));
        assert!(events.iter().any(|event| matches!(event, AgentStreamEvent::Finish(_))));
        assert_eq!(
            agent
                .engine
                .lock()
                .await
                .host_context_value(IMAGE_ROUTE_CONTEXT_KEY)
            .as_deref(),
            Some(IMAGE_ROUTE_EXTERNAL),
            "an unsupported follow-up must not overwrite the last completed image route"
        );
    }

    #[tokio::test]
    async fn cancellation_wins_while_no_model_response_waits_for_engine_history_lock() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let agent = Arc::new(make_agent_with_provider(provider.clone()));
        let mut rx = agent.subscribe();
        let engine_guard = agent.engine.lock().await;
        let sending_agent = Arc::clone(&agent);
        let send = tokio::spawn(async move {
            sending_agent
                .send_message(SendMessageData {
                    content: "generate a watercolor image".into(),
                    msg_id: "msg-cancel-no-model".into(),
                    source_message_id: None,
                    files: Vec::new(),
                    inject_skills: Vec::new(),
                    origin: None,
                })
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while agent.status() != Some(ConversationStatus::Running) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("test turn should enter Running");

        agent.cancel().await.unwrap();
        drop(engine_guard);
        send.await.unwrap().unwrap();

        assert_eq!(provider.calls(), 0);
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::Text(_))));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Finish(data)
                if data.stop_reason == Some(TurnStopReason::Cancelled)
        )));
    }

    #[tokio::test]
    async fn cancellation_wins_while_image_capability_refresh_is_pending() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let mut raw_agent = make_agent_with_provider(provider.clone());
        raw_agent.image_generation_discovery = Some(Arc::new(BlockingImageToolDiscovery {
            started: Arc::clone(&started),
        }));
        let agent = Arc::new(raw_agent);
        let mut rx = agent.subscribe();
        let sending_agent = Arc::clone(&agent);
        let send = tokio::spawn(async move {
            sending_agent
                .send_message(SendMessageData {
                    content: "generate an image of a lighthouse".into(),
                    msg_id: "msg-cancel-image-refresh".into(),
                    source_message_id: None,
                    files: Vec::new(),
                    inject_skills: Vec::new(),
                    origin: None,
                })
                .await
        });

        let permit = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            started.acquire(),
        )
        .await
        .expect("image capability discovery should start")
        .expect("discovery semaphore should remain open");
        permit.forget();
        assert_eq!(agent.status(), Some(ConversationStatus::Running));

        agent.cancel().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), send)
            .await
            .expect("cancel must drop a pending local capability refresh")
            .unwrap()
            .unwrap();

        assert_eq!(provider.calls(), 0);
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Finish(data)
                if data.stop_reason == Some(TurnStopReason::Cancelled)
        )));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::Text(_))));
    }

    #[tokio::test]
    async fn routed_image_turn_discards_text_success_claim_without_a_receipt() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta("The image was generated successfully.".into()),
            LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            },
        ]]));
        let mut agent = make_agent_with_provider(provider.clone());
        *agent.image_generation_availability.get_mut().unwrap() =
            ImageGenerationAvailability::Ready;
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(MissingImageArtifactTool))
        );
        let mut rx = agent.subscribe();

        let result = agent
            .send_message(SendMessageData {
                content: "generate an image of a student".into(),
                msg_id: "msg-false-success".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await;

        assert!(result.is_err());
        assert_eq!(provider.calls(), 1);
        assert_eq!(
            provider.requests()[0]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec![IMAGE_GEN_TOOL_NAME]
        );
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(event, AgentStreamEvent::Error(_))));
        assert!(
            !events.iter().any(|event| matches!(event, AgentStreamEvent::Text(_))),
            "a success claim must stay provisional and be discarded without a receipt"
        );
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::Finish(_))));
    }

    #[tokio::test]
    async fn external_image_request_without_bridge_or_native_model_short_circuits() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::ToolUse {
                    id: "browser-shot".into(),
                    name: "browserScreenshot".into(),
                    input: serde_json::json!({}),
                    extra: None,
                },
                LlmEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    usage: Default::default(),
                },
            ],
            vec![
                LlmEvent::TextDelta("Generated successfully in the browser.".into()),
                LlmEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    usage: Default::default(),
                },
            ],
        ]));
        let mut agent = make_agent_with_provider(provider.clone());
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(BrowserScreenshotOnlyTool))
        );
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "Use the browser website to generate a student image".into(),
                msg_id: "msg-browser-image".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(provider.calls(), 0);
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Text(data)
                if data.content.contains("cannot yet persist and verify")
                    && data.content.contains(IMAGE_MODEL_MANAGEMENT_LINK)
        )));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::Error(_))));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::ToolCall(_))));
        assert!(events.iter().any(|event| matches!(event, AgentStreamEvent::Finish(_))));
    }

    #[tokio::test]
    async fn external_image_request_without_durable_bridge_stays_host_deterministic() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let mut agent = make_agent_with_provider(provider.clone());
        *agent.image_generation_availability.get_mut().unwrap() =
            ImageGenerationAvailability::Ready;
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(BrowserScreenshotOnlyTool))
        );
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "Use the browser website to generate a student image".into(),
                msg_id: "msg-browser-image-ready-native".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(provider.calls(), 0);
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::Text(data)
                if data.content.contains("cannot yet persist and verify")
                    && data.content.contains("configured native image model")
        )));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::Error(_))));
        assert!(!events.iter().any(|event| matches!(event, AgentStreamEvent::ToolCall(_))));
        assert!(events.iter().any(|event| matches!(event, AgentStreamEvent::Finish(_))));
    }

    #[tokio::test]
    async fn missing_generated_artifact_emits_error_and_never_normal_finish() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::ToolUse {
                    id: "missing-image".into(),
                    name: "image_gen".into(),
                    input: serde_json::json!({}),
                    extra: None,
                },
                LlmEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    usage: Default::default(),
                },
            ],
            vec![LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            }],
        ]));
        let mut agent = make_agent_with_provider(provider);
        *agent.image_generation_availability.get_mut().unwrap() =
            ImageGenerationAvailability::Ready;
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(MissingImageArtifactTool))
        );
        let mut rx = agent.subscribe();

        let result = agent
            .send_message(SendMessageData {
                content: "generate an image".into(),
                msg_id: "msg-missing-image".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await;

        let send_error = result.unwrap_err();
        assert_eq!(
            send_error.code(),
            Some(AgentErrorCode::UserLlmProviderEmptyResponse)
        );
        assert_eq!(
            send_error.ownership(),
            Some(AgentErrorOwnership::UserLlmProvider)
        );
        assert_eq!(send_error.stream_error().retryable, Some(true));
        assert_eq!(send_error.stream_error().feedback_recommended, Some(false));
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentStreamEvent::ToolCall(data)
                if data.call_id == "nomi-missing-image"
                    && data.status == ToolCallStatus::Error
                    && data.artifacts.is_empty()
        )));
        assert!(events.iter().any(|event| matches!(event, AgentStreamEvent::Error(_))));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentStreamEvent::Finish(_))),
            "artifact-delivery failure must not be followed by a normal Finish"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentStreamEvent::TurnCompleted(_)))
        );
    }

    #[test]
    fn image_delivery_error_mapping_keeps_host_integrity_failures_internal() {
        let provider_failure = image_artifact_delivery_error_to_send_error(
            ImageGenerationIntent::Creation,
            "image_gen (image artifact) failed artifact delivery: tool returned an error; accepted turn required a verified image artifact, but no matching receipt was committed",
        );
        assert_eq!(
            provider_failure.code(),
            Some(AgentErrorCode::UserLlmProviderEmptyResponse)
        );
        assert_eq!(
            provider_failure.ownership(),
            Some(AgentErrorOwnership::UserLlmProvider)
        );
        assert_eq!(provider_failure.stream_error().retryable, Some(true));
        assert_eq!(provider_failure.stream_error().feedback_recommended, Some(false));

        let ledger_failure = image_artifact_delivery_error_to_send_error(
            ImageGenerationIntent::Creation,
            "artifact-delivery ledger lock was poisoned",
        );
        assert_eq!(
            ledger_failure.code(),
            Some(AgentErrorCode::NomifunInternalError)
        );
        assert_eq!(ledger_failure.ownership(), Some(AgentErrorOwnership::Nomifun));
    }

    #[tokio::test]
    async fn provider_error_never_publishes_uncommitted_tool_progress() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::ToolUseDelta {
                    id: "stale-preview".into(),
                    name: "Write".into(),
                    input: Some(serde_json::json!({"file_path": "/tmp/stale.html"})),
                },
                LlmEvent::ToolUse {
                    id: "stale-preview".into(),
                    name: "Write".into(),
                    input: serde_json::json!({
                        "file_path": "/tmp/stale.html",
                        "content": "not committed"
                    }),
                    extra: None,
                },
                LlmEvent::Error("malformed structured tool arguments".into()),
            ],
            vec![LlmEvent::Done {
                stop_reason: StopReason::MaxTokens,
                usage: Default::default(),
            }],
            vec![LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            }],
        ]));
        let agent = make_agent_with_provider(provider.clone());
        let mut rx = agent.subscribe();

        let first_result = agent
            .send_message(SendMessageData {
                content: "trigger malformed structured progress".into(),
                msg_id: "msg-error".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await;
        assert!(first_result.is_err());

        let first_statuses = std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|event| match event {
                AgentStreamEvent::ToolCall(data) if data.call_id == "nomi-stale-preview" => {
                    Some(data.status)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            first_statuses.is_empty(),
            "partial provider progress must never enter the frontend lifecycle"
        );

        agent
            .send_message(SendMessageData {
                content: "start a clean turn".into(),
                msg_id: "msg-next".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        let resurrected = std::iter::from_fn(|| rx.try_recv().ok()).any(|event| {
            matches!(
                event,
                AgentStreamEvent::ToolCall(data) if data.call_id == "nomi-stale-preview"
            )
        });
        assert!(
            !resurrected,
            "a later MaxTokens continuation must not recover a failed prior call"
        );
        // Two passes, not three: the second turn's MaxTokens carries no truncated
        // call, no declared plan and no effect, so the engine correctly declines
        // to restart it. Re-running an identical request against an identical
        // ceiling can only reproduce the identical result.
        assert_eq!(provider.calls(), 2);
    }

    // The manager's restore helpers re-acquire the engine mutex, which is not
    // reentrant, so a failing turn must release its guard first. The failure mode
    // of the reentrant shape is a deadlock, so bound it explicitly: a regression
    // has to fail in seconds instead of hanging the suite.
    #[tokio::test]
    async fn a_provider_error_turn_releases_the_engine_lock_before_restoring() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![LlmEvent::Error(
            "upstream refused the request".into(),
        )]]));
        let agent = make_agent_with_provider(provider.clone());

        let sent = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            agent.send_message(SendMessageData {
                content: "trigger a provider failure".into(),
                msg_id: "msg-provider-error".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            }),
        )
        .await
        .expect("a provider-error turn must terminalize, not deadlock on the engine mutex");
        assert!(sent.is_err(), "a provider error is never a successful turn");

        // The guard must be free again, otherwise the next turn would deadlock.
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(20), agent.engine.lock())
                .await
                .is_ok(),
            "the failed turn must not leak its engine guard"
        );
        assert_eq!(provider.calls(), 1, "a provider error is not silently replayed");
    }

    #[tokio::test]
    async fn cancel_drains_committed_tool_progress() {
        let provider = Arc::new(BlockingProvider::new());
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let mut agent = make_agent_with_provider(provider.clone());
        agent
            .engine
            .get_mut()
            .registry_mut()
            .register(Box::new(HangingTool {
                started: Arc::clone(&started),
            }));
        let agent = Arc::new(agent);
        let mut rx = agent.subscribe();
        let send_task = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "start a committed tool call".into(),
                        msg_id: "msg-cancel-tool".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };
        provider.called.acquire().await.unwrap().forget();

        let provider_tx = provider
            .senders
            .lock()
            .expect("sender lock poisoned")
            .last()
            .expect("blocking provider sender")
            .clone();
        provider_tx
            .send(LlmEvent::ToolUse {
                id: "cancel-committed".into(),
                name: "hanging_tool".into(),
                input: serde_json::json!({}),
                extra: None,
            })
            .await
            .unwrap();
        provider_tx
            .send(LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Default::default(),
            })
            .await
            .unwrap();
        drop(provider_tx);
        provider
            .senders
            .lock()
            .expect("sender lock poisoned")
            .pop()
            .expect("blocking provider sender should still be registered");

        tokio::time::timeout(std::time::Duration::from_secs(1), started.acquire())
            .await
            .expect("committed tool should start execution")
            .expect("start semaphore should remain open")
            .forget();

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let AgentStreamEvent::ToolCall(data) = rx.recv().await.unwrap()
                    && data.call_id == "nomi-cancel-committed"
                    && data.status == ToolCallStatus::Running
                {
                    break;
                }
            }
        })
        .await
        .expect("tool progress should reach the frontend before cancellation");

        agent.cancel().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), send_task)
            .await
            .expect("cancelled send should unwind")
            .unwrap()
            .unwrap();

        let remaining_events = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let terminal_events = remaining_events
            .iter()
            .filter_map(|event| match event {
                AgentStreamEvent::ToolCall(data) if data.call_id == "nomi-cancel-committed" => {
                    Some(data)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminal_events.len(), 1);
        assert_eq!(terminal_events[0].status, ToolCallStatus::Error);
        assert_eq!(
            terminal_events[0].output.as_deref(),
            Some("Tool execution canceled by user")
        );
        assert_eq!(
            remaining_events
                .iter()
                .filter_map(|event| match event {
                    AgentStreamEvent::Finish(data) => Some(data.stop_reason),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![Some(TurnStopReason::Cancelled)]
        );
        assert_eq!(agent.status(), Some(ConversationStatus::Finished));
    }

    #[tokio::test]
    async fn cancel_drops_a_hung_tool_and_emits_one_error_terminal() {
        let provider = Arc::new(ScriptedProvider::new(vec![vec![
            LlmEvent::ToolUse {
                id: "hang-1".into(),
                name: "hanging_tool".into(),
                input: serde_json::json!({}),
                extra: None,
            },
            LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Default::default(),
            },
        ]]));
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let mut agent = make_agent_with_provider(provider.clone());
        agent
            .engine
            .get_mut()
            .registry_mut()
            .register(Box::new(HangingTool {
                started: Arc::clone(&started),
            }));
        let agent = Arc::new(agent);
        let mut rx = agent.subscribe();

        let send_task = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "run the hanging tool".into(),
                        msg_id: "msg-hanging-tool".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };

        tokio::time::timeout(std::time::Duration::from_secs(1), started.acquire())
            .await
            .expect("hanging tool should start")
            .expect("start semaphore should remain open")
            .forget();

        agent.cancel().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), send_task)
            .await
            .expect("cancellation must not await a hung tool")
            .unwrap()
            .unwrap();

        let statuses = std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|event| match event {
                AgentStreamEvent::ToolCall(data) if data.call_id == "nomi-hang-1" => {
                    Some(data.status)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            statuses,
            vec![ToolCallStatus::Running, ToolCallStatus::Error],
            "a cancelled hung tool must terminate once and never complete"
        );
        assert_eq!(provider.calls(), 1);
    }

    #[tokio::test]
    async fn cancel_interrupts_hung_knowledge_preparation() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let retrieval: Arc<dyn nomi_agent::knowledge_tools::KnowledgeRetrievalSink> =
            Arc::new(HangingKnowledgeSink {
                started: Arc::clone(&started),
            });
        let mut agent = make_agent_with_provider(provider.clone());
        agent.knowledge_auto_rag = Some((retrieval, vec![nomifun_common::KnowledgeBaseId::new()]));
        let agent = Arc::new(agent);
        let mut rx = agent.subscribe();

        let send_task = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "search the mounted knowledge base".into(),
                        msg_id: "msg-hanging-rag".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };

        tokio::time::timeout(std::time::Duration::from_secs(1), started.acquire())
            .await
            .expect("knowledge preparation should start")
            .expect("start semaphore should remain open")
            .forget();
        agent.cancel().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), send_task)
            .await
            .expect("cancellation must interrupt a hung knowledge search")
            .unwrap()
            .unwrap();

        assert_eq!(provider.calls(), 0);
        assert_single_cancelled_finish_without_running_tools(&agent, &mut rx);
    }

    #[tokio::test]
    async fn cancel_interrupts_engine_lock_preparation() {
        let provider = Arc::new(ScriptedProvider::new(Vec::new()));
        let agent = Arc::new(make_agent_with_provider(provider.clone()));
        let mut rx = agent.subscribe();
        let engine_guard = agent.engine.lock().await;

        let send_task = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "wait for engine preparation".into(),
                        msg_id: "msg-blocked-engine-lock".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while agent.status() != Some(ConversationStatus::Running) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("turn should enter Running before waiting for the engine lock");

        agent.cancel().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), send_task)
            .await
            .expect("cancellation must interrupt engine-lock preparation")
            .unwrap()
            .unwrap();

        assert_eq!(provider.calls(), 0);
        assert_single_cancelled_finish_without_running_tools(&agent, &mut rx);
        drop(engine_guard);
    }

    #[tokio::test]
    async fn send_message_does_not_auto_continue_after_max_turns() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::ToolUse {
                    id: "loop-1".into(),
                    name: "ToolSearch".into(),
                    input: serde_json::json!({"query": "missing_loop_tool"}),
                    extra: None,
                },
                LlmEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    usage: Default::default(),
                },
            ],
            // This response is present only to expose a regression: the old
            // host policy issued a second provider pass after MaxTurns.
            vec![LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            }],
        ]));
        let agent = make_agent_with_provider_and_max_turns(provider.clone(), Some(1));
        {
            let mut engine = agent.engine.lock().await;
            let deferred_state = engine.registry_mut().deferred_state();
            engine.registry_mut().register(Box::new(
                nomi_tools::tool_search::ToolSearchTool::new(deferred_state),
            ));
        }
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "keep calling the tool".into(),
                msg_id: "msg-max-turns".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .unwrap();

        assert_eq!(
            provider.calls(),
            1,
            "MaxTurns must terminate the host turn instead of resetting the engine budget"
        );

        let mut completed_turns = 0;
        let mut finish_reason = None;
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentStreamEvent::TurnCompleted(_) => completed_turns += 1,
                AgentStreamEvent::Finish(data) => finish_reason = data.stop_reason,
                _ => {}
            }
        }
        assert_eq!(completed_turns, 1);
        assert_eq!(finish_reason, Some(TurnStopReason::MaxTurnRequests));
    }

    #[tokio::test]
    async fn max_turns_absorbs_late_steering_before_the_next_explicit_turn() {
        let provider = Arc::new(BlockingProvider::new());
        let mut agent = make_agent_with_provider_and_max_turns(provider.clone(), Some(1));
        {
            let deferred_state = agent.engine.get_mut().registry_mut().deferred_state();
            assert!(
                agent.engine.get_mut().registry_mut().register(Box::new(
                    nomi_tools::tool_search::ToolSearchTool::new(deferred_state),
                ))
            );
        }
        let agent = Arc::new(agent);
        let mut rx = agent.subscribe();

        let first_send = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "start".into(),
                        msg_id: "msg-max-turns-late-steer".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), provider.called.acquire())
            .await
            .expect("first provider pass should start")
            .expect("provider start semaphore should remain open")
            .forget();

        // The interjection is admitted while this exact generation is Running,
        // after the provider request has already begun. A MaxTurns terminal is
        // not eligible for the race-tail continuation, so terminalization must
        // absorb this queue entry under the lifecycle gate.
        agent
            .steering_inbox
            .lock()
            .unwrap()
            .push_back("late user direction".to_owned());
        let first_provider_tx = provider
            .senders
            .lock()
            .expect("sender lock poisoned")
            .pop()
            .expect("first blocking provider sender");
        first_provider_tx
            .send(LlmEvent::ToolUse {
                id: "late-steer-max-turns-tool".into(),
                name: "ToolSearch".into(),
                input: serde_json::json!({"query": "missing_late_steer_tool"}),
                extra: None,
            })
            .await
            .unwrap();
        first_provider_tx
            .send(LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Default::default(),
            })
            .await
            .unwrap();
        drop(first_provider_tx);
        tokio::time::timeout(std::time::Duration::from_secs(1), first_send)
            .await
            .expect("MaxTurns send should terminate")
            .unwrap()
            .unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert!(
            agent.steering_inbox.lock().unwrap().is_empty(),
            "a terminal generation must absorb its late steering"
        );

        let mut starts = 0;
        let mut completed_turns = 0;
        let mut finish_reason = None;
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentStreamEvent::Start(_) => starts += 1,
                AgentStreamEvent::TurnCompleted(_) => completed_turns += 1,
                AgentStreamEvent::Finish(data) => finish_reason = data.stop_reason,
                _ => {}
            }
        }
        assert_eq!(starts, 1, "MaxTurns must not start a race-tail engine pass");
        assert_eq!(completed_turns, 1);
        assert_eq!(finish_reason, Some(TurnStopReason::MaxTurnRequests));

        let second_send = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "next explicit turn".into(),
                        msg_id: "msg-after-max-turns".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), provider.called.acquire())
            .await
            .expect("second explicit turn should reach the provider")
            .expect("provider start semaphore should remain open")
            .forget();

        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(
            !requests[1].messages.iter().any(|message| {
                message.content.iter().any(|block| {
                    matches!(
                        block,
                        ContentBlock::Text { text } if text.contains("late user direction")
                    )
                })
            }),
            "the next explicit turn must never inherit terminal steering"
        );

        let second_provider_tx = provider
            .senders
            .lock()
            .expect("sender lock poisoned")
            .pop()
            .expect("second blocking provider sender");
        second_provider_tx
            .send(LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            })
            .await
            .unwrap();
        drop(second_provider_tx);
        tokio::time::timeout(std::time::Duration::from_secs(1), second_send)
            .await
            .expect("second explicit turn should terminate")
            .unwrap()
            .unwrap();
        assert!(agent.steering_inbox.lock().unwrap().is_empty());
    }

    /// The recoverable shape, end to end through the host: a state-changing tool
    /// call cut off at the ceiling must restart the round against the ORIGINAL
    /// requirement — not append an English "continue where you left off" prompt,
    /// and not replay the truncated draft.
    #[tokio::test]
    async fn a_truncated_tool_call_restarts_the_round_against_the_original_requirement() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            vec![
                LlmEvent::TextDelta("Here is the whole file inline: <html>".into()),
                LlmEvent::ToolUseTruncated {
                    id: "call-large-write".into(),
                    name: "Write".into(),
                    argument_bytes: 6142,
                },
                LlmEvent::Done {
                    stop_reason: StopReason::MaxTokens,
                    usage: Default::default(),
                },
            ],
            vec![LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            }],
        ]));
        let mut agent = make_agent_with_provider(provider.clone());
        assert!(
            agent
                .engine
                .get_mut()
                .registry_mut()
                .register(Box::new(PreviewOnlyWriteTool)),
            "Write must be advertised for the truncated call to be an authorized fact"
        );
        let mut rx = agent.subscribe();

        agent
            .send_message(SendMessageData {
                content: "create a polished single page site".into(),
                msg_id: "msg-1".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .expect("a restarted round is a normal outcome, not a hard failure");

        let requests = provider.requests();
        assert_eq!(requests.len(), 2, "the truncated Write must earn one restart");

        // The round facts travel on the SYSTEM channel, never as a user message:
        // a user message would pollute the durable transcript and teach the model
        // that host bookkeeping is user instruction.
        let system = &requests[1].system;
        assert!(system.contains("[resumable round 2/3]"), "system: {system}");
        assert!(system.contains("WHAT WAS CUT OFF"));
        assert!(system.contains("Write (6142 bytes of arguments streamed, NOT executed)"));

        // The tail message is the original requirement, verbatim and exactly
        // once — not a continuation prompt, and not the truncated draft.
        let messages = &requests[1].messages;
        let tail = messages.last().expect("the restart re-pushes the requirement");
        assert_eq!(tail.role, Role::User);
        assert!(matches!(
            &tail.content[..],
            [ContentBlock::Text { text }] if text.contains("create a polished single page site")
        ));

        let serialized = serde_json::to_string(messages).expect("messages serialize");
        assert!(
            !serialized.contains("Here is the whole file inline"),
            "the truncated draft must leave the provider request entirely: {serialized}"
        );
        assert_eq!(
            serialized.matches("create a polished single page site").count(),
            1,
            "the requirement must appear exactly once: {serialized}"
        );
        assert!(
            !serialized.contains("Automatic continuation"),
            "the deleted host continuation prompt must not come back"
        );

        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let leaked_partial_call = events.iter().any(|event| {
            matches!(
                event,
                AgentStreamEvent::ToolCall(data) if data.call_id == "nomi-call-large-write"
            )
        });
        assert!(
            !leaked_partial_call,
            "a truncated tool call must never enter the frontend lifecycle"
        );
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    AgentStreamEvent::OutputDiscarded(data) => Some(data.restart_attempt),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![2],
            "the retry must retract only the provider pass after its checkpoint"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentStreamEvent::Start(_)))
                .count(),
            2,
            "the second Start is a non-destructive provider-pass checkpoint"
        );
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn yolo_session_installs_browser_approval_gate_without_takeover_pref() {
        let mgr = ToolApprovalManager::new();
        mgr.set_mode(SessionMode::Yolo);

        assert!(
            should_install_browser_approval_gate(false, false, &mgr),
            "full-auto/yolo sessions need the Browser gate so gated egress can approve without UI"
        );
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn default_session_without_browser_approval_pref_does_not_install_browser_gate() {
        let mgr = ToolApprovalManager::new();

        assert!(
            !should_install_browser_approval_gate(false, false, &mgr),
            "default sessions without Browser approval prefs should keep the old fail-closed path"
        );
    }

    #[tokio::test]
    async fn nomi_agent_returns_correct_type() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        assert_eq!(agent.agent_type(), AgentType::Nomi);
        assert_eq!(agent.workspace(), "/project");
        assert_eq!(agent.conversation_id(), "conv-1");
    }

    #[tokio::test]
    async fn manager_threads_embedded_agent_execution_host_composition_into_bootstrap() {
        let mut config = make_test_config();
        config.install_embedded_agent_execution = false;
        let agent = NomiAgentManager::new(
            "conv-no-embedded-execution".into(),
            "/project".into(),
            config,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
        )
        .await
        .unwrap();

        let names = agent.engine.lock().await.tool_names();
        assert!(
            !names.iter().any(|name| name == "nomi_delegate"),
            "manager must preserve the backend host-composition decision"
        );
    }

    // -- distillation eligibility gates (construction-time) -------------------

    struct StubCompanionSink;

    #[async_trait::async_trait]
    impl nomi_agent::companion_tools::CompanionMemorySink for StubCompanionSink {
        async fn recall(&self, _conv: &str, _queries: &[String], _kind: Option<&str>, _archived: bool, _limit: usize) -> Result<String, String> {
            Ok(String::new())
        }
        async fn save(&self, _conv: &str, _kind: &str, _content: &str, _tags: &[String]) -> Result<String, String> {
            Ok(String::new())
        }
        async fn recent_events(&self, _limit: usize) -> Result<String, String> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn companion_session_never_distills() {
        // Companion red line: a session with a companion sink must have NO
        // distill target, so no post-turn distillation child is admitted.
        let sink: Arc<dyn nomi_agent::companion_tools::CompanionMemorySink> = Arc::new(StubCompanionSink);
        let agent = NomiAgentManager::new(
            "conv-companion".into(),
            "/project".into(),
            make_test_config(),
            None,
            None,
            Some(sink),
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
        )
        .await
        .unwrap();
        assert!(
            agent.distill_dir_for_test().is_none(),
            "companion sessions must not distill into file-based memory"
        );
    }

    #[tokio::test]
    async fn normal_session_resolves_a_distill_dir() {
        // A normal work session (no companion sink) gets a project-level
        // memory dir as its distill target. The runtime origin gate and the
        // enable flag are checked separately at send time.
        let agent = NomiAgentManager::new(
            "conv-normal".into(),
            "/project".into(),
            make_test_config(),
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
        )
        .await
        .unwrap();
        // auto_memory_dir resolves unless the platform has no config dir; on
        // the test host it should be Some.
        assert!(
            agent.distill_dir_for_test().is_some(),
            "normal sessions should resolve a distill target dir"
        );
    }

    #[tokio::test]
    async fn nomi_agent_initial_status_is_pending() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        assert_eq!(agent.status(), Some(ConversationStatus::Pending));
    }

    // -- summoned-companion sessions (spec §设计 B) ----------------------------

    struct StubSummonContextSink;

    #[async_trait::async_trait]
    impl SummonContextSink for StubSummonContextSink {
        async fn resolve_context(&self) -> Option<String> {
            Some("## 召唤的伙伴记忆（只读参考）".into())
        }
    }

    fn stub_summon_wiring() -> NomiSummonWiring {
        NomiSummonWiring {
            memory_sink: Arc::new(StubCompanionSink),
            context_sink: Arc::new(StubSummonContextSink),
        }
    }

    #[tokio::test]
    async fn summon_session_registers_readonly_tools_never_save_memory() {
        // Read-only boundary (spec §B3): a summoned work session gets recall
        // only, and must NEVER see the direct-write save_memory, the retired
        // propose_companion_memory, or the companion-only list_recent_events.
        let agent = NomiAgentManager::new_with_host_wiring(
            "conv-summon".into(),
            "/project".into(),
            make_test_config(),
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            None,
            Some(stub_summon_wiring()),
            NomiHostWiring::default(),
        )
        .await
        .unwrap();
        let names = agent.engine.lock().await.tool_names();
        assert!(names.iter().any(|n| n == "recall_memories"), "{names:?}");
        assert!(
            !names.iter().any(|n| n == "propose_companion_memory"),
            "the retired propose tool must never come back: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "save_memory"),
            "save_memory must never be registered under summon: {names:?}"
        );
        assert!(!names.iter().any(|n| n == "list_recent_events"), "{names:?}");
        // Summoned work sessions are still ordinary work sessions: the
        // companion no-distill red line does NOT apply to them.
        assert!(agent.distill_dir_for_test().is_some());
    }

    #[tokio::test]
    async fn plain_session_has_no_summon_tools() {
        let agent = NomiAgentManager::new(
            "conv-plain".into(),
            "/project".into(),
            make_test_config(),
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
        )
        .await
        .unwrap();
        let names = agent.engine.lock().await.tool_names();
        assert!(!names.iter().any(|n| n == "recall_memories"), "{names:?}");
        assert!(!names.iter().any(|n| n == "propose_companion_memory"), "{names:?}");
    }

    #[tokio::test]
    async fn nomi_agent_subscribe_returns_receiver() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        let _rx = agent.subscribe();
    }

    #[tokio::test]
    async fn nomi_agent_kill_succeeds() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        assert!(agent.kill(None).is_ok());
        // Idle kill publishes Finished only after its exact process-tree fence.
        // Waiting for the terminal is intentionally unbounded; the registry
        // retains quarantine authority while teardown is unresolved.
        agent.runtime.wait_until_finished_unbounded().await;
        assert_eq!(agent.status(), Some(ConversationStatus::Finished));
    }

    #[tokio::test]
    async fn nomi_agent_kill_with_reason_succeeds() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        assert!(agent.kill(Some(AgentKillReason::IdleTimeout)).is_ok());
    }

    #[tokio::test]
    async fn nomi_agent_kill_running_turn_cancels_turn_token() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        agent.runtime.reset_for_new_turn(ConversationStatus::Running);

        let token = agent
            .turn_cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(!token.is_cancelled());

        agent
            .kill(Some(AgentKillReason::ConversationDeleted))
            .expect("kill should request stop");

        assert!(token.is_cancelled(), "running kill must cancel the active turn token");
    }

    #[tokio::test]
    async fn nomi_agent_kill_idle_turn_does_not_leave_stale_stop_signal() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();

        agent
            .kill(Some(AgentKillReason::ConversationDeleted))
            .expect("idle kill should be harmless");

        assert!(agent.closing.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn nomi_agent_kill_is_durable_and_rejects_raced_turn_admission() {
        let agent = make_agent_with_provider(Arc::new(ScriptedProvider::new(vec![])));

        agent
            .kill(Some(AgentKillReason::ConversationDeleted))
            .expect("kill should close the task");

        assert!(agent.closing.load(Ordering::Acquire));
        assert!(
            agent
                .turn_cancel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_cancelled(),
            "kill cancellation must remain observable even without a registered waiter"
        );
        let error = agent
            .send_message(SendMessageData {
                content: "must not start".into(),
                msg_id: "raced-after-kill".into(),
                source_message_id: None,
                files: Vec::new(),
                inject_skills: Vec::new(),
                origin: None,
            })
            .await
            .expect_err("closed manager must reject a raced clone");
        assert!(
            error
                .stream_error()
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("shutting down"))
        );
    }

    #[tokio::test]
    async fn kill_cancels_active_turn_and_rejects_second_queued_send() {
        let provider = Arc::new(BlockingProvider::new());
        let agent = Arc::new(make_agent_with_provider(provider.clone()));
        let first = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "first".into(),
                        msg_id: "first".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };
        provider.called.acquire().await.unwrap().forget();

        let second = {
            let agent = Arc::clone(&agent);
            tokio::spawn(async move {
                agent
                    .send_message(SendMessageData {
                        content: "second".into(),
                        msg_id: "second".into(),
                        source_message_id: None,
                        files: Vec::new(),
                        inject_skills: Vec::new(),
                        origin: None,
                    })
                    .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        agent
            .kill(Some(AgentKillReason::ConversationDeleted))
            .expect("kill should close every admitted send");

        let first_result = tokio::time::timeout(std::time::Duration::from_millis(200), first)
            .await
            .expect("active turn must observe kill")
            .unwrap();
        assert!(first_result.is_ok());
        assert!(second.await.unwrap().is_err(), "queued send must see closing task");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn nomi_agent_confirmations_initially_empty() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        assert!(agent.get_confirmations().is_empty());
    }

    #[tokio::test]
    async fn nomi_agent_get_slash_commands_does_not_wait_for_engine_lock() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();

        let _engine_guard = agent.engine.lock().await;
        let commands = tokio::time::timeout(std::time::Duration::from_millis(50), agent.get_slash_commands())
            .await
            .expect("slash command metadata should not wait for an active turn execution")
            .unwrap();

        assert!(!commands.is_empty());
    }

    #[tokio::test]
    async fn nomi_agent_check_approval_returns_false_by_default() {
        let agent = NomiAgentManager::new("conv-1".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        assert!(!agent.check_approval("any_action", None));
    }

    #[tokio::test]
    async fn idle_cancel_emits_finish_and_transitions() {
        // Cancelling an agent with no in-flight run must still emit a terminal
        // event and transition to Finished — otherwise a subscribed relay hangs
        // forever in a 'running' spinner because no Finish/Error ever arrives.
        // (Phase 0 F0.2)
        let agent = NomiAgentManager::new("conv-stop".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        let mut rx = agent.subscribe();

        agent.cancel().await.unwrap();

        assert_eq!(agent.status(), Some(ConversationStatus::Finished));
        match rx.try_recv() {
            Ok(AgentStreamEvent::Finish(_)) => {}
            other => panic!("expected Finish on idle cancel, got {:?}", other),
        }
        // A later reusable turn replaces this cancelled token during admission.
        assert!(
            agent
                .turn_cancel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_cancelled()
        );
    }

    #[tokio::test]
    async fn termination_guard_emits_finish_on_armed_drop() {
        // The guard backstops panics / unexpected early-returns in send_message:
        // if the turn unwinds without emitting a terminal event, dropping the
        // armed guard must still broadcast one so the relay does not hang. (F0.2)
        let rt = AgentRuntimeState::new("c-guard", "/w", 16);
        let mut rx = rt.subscribe();
        let backend_output_sink = Arc::new(BackendOutputSink::new(rt.event_sender()));
        let turn = rt.reset_for_new_turn(ConversationStatus::Running);
        let active_turn = Arc::new(std::sync::Mutex::new(Some(turn)));
        let lifecycle_gate = Arc::new(std::sync::Mutex::new(()));
        let steering_inbox = Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(["unconsumed steer".to_owned()]),
        ));
        backend_output_sink.emit_tool_call("guarded-call", "Write", "{}");
        assert!(matches!(rx.try_recv(), Ok(AgentStreamEvent::ToolCall(_))));
        {
            let _g = TurnTerminationGuard {
                runtime: rt.clone(),
                turn,
                active_turn: Arc::clone(&active_turn),
                lifecycle_gate,
                steering_inbox: Arc::clone(&steering_inbox),
                backend_output_sink,
                process_supervisor: None,
                mcp_managers: Vec::new(),
                turn_teardown_fence: Arc::new(TurnTeardownFence::new()),
                accepted_turn_recovery_required: Arc::new(AtomicBool::new(false)),
                #[cfg(feature = "browser-use")]
                browser_lane_binding: None,
                armed: true,
            };
        }
        assert!(
            !rt.is_transport_healthy(),
            "an abnormal unwind must make the cached manager ineligible for reuse"
        );
        match rx.try_recv() {
            Ok(AgentStreamEvent::ToolCall(data)) => {
                assert_eq!(data.call_id, "nomi-guarded-call");
                assert_eq!(data.status, ToolCallStatus::Error);
                assert_eq!(data.description.as_deref(), Some("Tool call cancelled"));
            }
            other => panic!("expected tool cleanup on armed drop, got {:?}", other),
        }
        // The terminal is published by the guard's spawned teardown task only
        // after every cleanup fence has been attempted, so it is asynchronous
        // relative to the drop itself.
        match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(AgentStreamEvent::Finish(_))) => {}
            other => panic!("expected Finish after tool cleanup, got {:?}", other),
        }
        assert_eq!(rt.status(), Some(ConversationStatus::Finished));
        assert!(active_turn.lock().unwrap().is_none());
        assert!(steering_inbox.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn termination_guard_reports_session_inconsistency_after_root_registration() {
        let rt = AgentRuntimeState::new("c-guard-session-root", "/w", 16);
        let mut rx = rt.subscribe();
        let turn = rt.reset_for_new_turn(ConversationStatus::Running);
        let active_turn = Arc::new(std::sync::Mutex::new(Some(turn)));
        let recovery_required = Arc::new(AtomicBool::new(true));
        {
            let _guard = TurnTerminationGuard {
                runtime: rt.clone(),
                turn,
                active_turn: Arc::clone(&active_turn),
                lifecycle_gate: Arc::new(std::sync::Mutex::new(())),
                steering_inbox: Arc::new(std::sync::Mutex::new(
                    std::collections::VecDeque::new(),
                )),
                backend_output_sink: Arc::new(BackendOutputSink::new(rt.event_sender())),
                process_supervisor: None,
                mcp_managers: Vec::new(),
                turn_teardown_fence: Arc::new(TurnTeardownFence::new()),
                accepted_turn_recovery_required: recovery_required,
                #[cfg(feature = "browser-use")]
                browser_lane_binding: None,
                armed: true,
            };
        }

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("armed drop must publish its typed terminal")
            .expect("terminal channel closed unexpectedly");
        match event {
            AgentStreamEvent::Error(error) => assert_eq!(
                error.code,
                Some(nomifun_api_types::AgentErrorCode::NomifunAgentSessionInconsistent)
            ),
            other => panic!("expected session-consistency Error, got {other:?}"),
        }
        assert_eq!(rt.status(), Some(ConversationStatus::Finished));
        assert!(active_turn.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn termination_guard_exact_terminalization_absorbs_steering_and_emits_once() {
        // On the normal path the guard owns both the terminal event and the
        // exact generation's steering cleanup. Its later Drop must stay silent.
        let rt = AgentRuntimeState::new("c-guard2", "/w", 16);
        let mut rx = rt.subscribe();
        let backend_output_sink = Arc::new(BackendOutputSink::new(rt.event_sender()));
        let turn = rt.reset_for_new_turn(ConversationStatus::Running);
        let active_turn = Arc::new(std::sync::Mutex::new(Some(turn)));
        let lifecycle_gate = Arc::new(std::sync::Mutex::new(()));
        let steering_inbox = Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from(["tail steer".to_owned()]),
        ));
        {
            let mut g = TurnTerminationGuard {
                runtime: rt.clone(),
                turn,
                active_turn: Arc::clone(&active_turn),
                lifecycle_gate,
                steering_inbox: Arc::clone(&steering_inbox),
                backend_output_sink,
                process_supervisor: None,
                mcp_managers: Vec::new(),
                turn_teardown_fence: Arc::new(TurnTeardownFence::new()),
                accepted_turn_recovery_required: Arc::new(AtomicBool::new(false)),
                #[cfg(feature = "browser-use")]
                browser_lane_binding: None,
                armed: true,
            };
            assert!(g.terminalize(|runtime, turn| {
                runtime.emit_finish_for_turn(turn, None, Some(TurnStopReason::EndTurn))
            }).await.unwrap());
        }
        assert!(matches!(rx.try_recv(), Ok(AgentStreamEvent::Finish(_))));
        assert!(matches!(rx.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
        assert!(active_turn.lock().unwrap().is_none());
        assert!(steering_inbox.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_can_emit_error_and_finish() {
        let agent = NomiAgentManager::new("conv-err".into(), "/project".into(), make_test_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
            .await
            .unwrap();
        let mut rx = agent.subscribe();

        agent.runtime.emit_error("test error");
        // emit_error sets status to Finished, so emit_finish is a no-op here.
        // We emit directly for the Finish broadcast path test:
        agent
            .runtime
            .emit(AgentStreamEvent::Finish(crate::protocol::events::FinishEventData {
                session_id: None,
                stop_reason: None,
            }));

        match rx.try_recv().unwrap() {
            AgentStreamEvent::Error(data) => assert_eq!(data.message, "test error"),
            other => panic!("Expected Error, got {:?}", other),
        }
        match rx.try_recv().unwrap() {
            AgentStreamEvent::Finish(_) => {}
            other => panic!("Expected Finish, got {:?}", other),
        }
    }

    #[test]
    fn nomi_provider_connection_error_is_user_llm_provider_error() {
        let send_error = nomi_engine_error_to_send_error(
            "Nomi agent error: Provider error: Connection error: Signable request error: failed to create canonical request"
                .to_owned(),
        );

        assert_eq!(
            send_error.code(),
            Some(nomifun_api_types::AgentErrorCode::UserLlmProviderConfigError)
        );
        assert_eq!(
            send_error.ownership(),
            Some(nomifun_api_types::AgentErrorOwnership::UserLlmProvider)
        );
        assert_eq!(send_error.stream_error().retryable, Some(false));
    }

    #[test]
    fn nomi_api_connection_error_is_user_llm_provider_network_error() {
        let send_error = nomi_engine_error_to_send_error(
            "Nomi agent error: API error: Connection error: error decoding response body".to_owned(),
        );

        assert_eq!(
            send_error.code(),
            Some(nomifun_api_types::AgentErrorCode::UserLlmProviderNetworkError)
        );
        assert_eq!(
            send_error.ownership(),
            Some(nomifun_api_types::AgentErrorOwnership::UserLlmProvider)
        );
        assert_eq!(send_error.stream_error().retryable, Some(true));
    }
}

#[cfg(test)]
mod knowledge_search_gate_tests {
    use super::should_register_knowledge_search;
    use nomifun_common::KnowledgeBaseId;

    #[test]
    fn registers_only_with_sink_and_bound_bases() {
        let kb_id = KnowledgeBaseId::new();
        assert!(should_register_knowledge_search(true, std::slice::from_ref(&kb_id)));
        assert!(!should_register_knowledge_search(true, &[]));
        assert!(!should_register_knowledge_search(false, &[kb_id]));
    }
}

#[cfg(test)]
mod knowledge_write_gate_tests {
    use super::should_register_knowledge_write;
    use nomifun_common::KnowledgeBaseId;

    #[test]
    fn registers_only_with_sink_and_bound_bases() {
        let bases = vec![(KnowledgeBaseId::new(), "Finance".to_owned())];
        assert!(should_register_knowledge_write(true, &bases));
        // No bound bases → nothing to write to, even with a sink.
        assert!(!should_register_knowledge_write(true, &[]));
        // No sink (write-back disabled or standalone) → never registered.
        assert!(!should_register_knowledge_write(false, &bases));
    }
}

#[cfg(test)]
mod knowledge_prelude_tests {
    use super::apply_knowledge_prelude;
    use super::prepend_knowledge_context;
    #[test]
    fn prepend_knowledge_context_formats_hits_and_passthrough_when_empty() {
        use nomi_agent::knowledge_tools::KnowledgeHit;
        assert_eq!(prepend_knowledge_context(&[], "hi".to_string()), "hi");

        let hits = vec![KnowledgeHit {
            kb_id: nomifun_common::KnowledgeBaseId::new(),
            handle: "h".into(),
            kb_name: "Docs".into(),
            rel_path: "a/b.md".into(),
            heading: "Title".into(),
            snippet: "the snippet".into(),
        }];
        let out = prepend_knowledge_context(&hits, "do the task".to_string());
        assert!(out.contains("Docs/a/b.md"));
        assert!(out.contains("Title"));
        assert!(out.contains("the snippet"));
        assert!(out.contains("handle: h"), "proactive hit must expose its opaque handle: {out}");
        assert!(
            out.contains("knowledge_read") && out.contains("unchanged"),
            "proactive guidance must tell the model to copy the handle unchanged: {out}"
        );
        assert!(out.ends_with("do the task"), "original message preserved at end");
    }
    #[test]
    fn prepends_when_present_and_passthrough_when_absent() {
        let out = apply_knowledge_prelude(Some("[KB: A]".into()), "do the thing");
        assert!(out.starts_with("[KB: A]"));
        assert!(out.ends_with("do the thing"));
        assert_eq!(apply_knowledge_prelude(None, "do the thing"), "do the thing");
        assert_eq!(apply_knowledge_prelude(Some(String::new()), "x"), "x");
    }
}

#[cfg(test)]
mod turn_completed_mapping_tests {
    use super::map_engine_stop_reason;
    use crate::protocol::events::TurnStopReason;
    use nomi_types::message::StopReason;

    #[test]
    fn maps_engine_stop_reason_to_normalized_turn_reason() {
        // A natural finish is a clean EndTurn.
        assert_eq!(map_engine_stop_reason(StopReason::EndTurn), TurnStopReason::EndTurn);
        // ToolUse as the terminal reason means the engine handed back between
        // tool batches without a refusal/limit — treat as a normal completion.
        assert_eq!(map_engine_stop_reason(StopReason::ToolUse), TurnStopReason::EndTurn);
        // Truncation by token budget — the turn did NOT accomplish its goal.
        assert_eq!(map_engine_stop_reason(StopReason::MaxTokens), TurnStopReason::MaxTokens);
        // Per-turn request cap — likewise a truncated turn.
        assert_eq!(map_engine_stop_reason(StopReason::MaxTurns), TurnStopReason::MaxTurnRequests);
        assert_eq!(map_engine_stop_reason(StopReason::Refusal), TurnStopReason::Refusal);
    }
}
