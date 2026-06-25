//! Public DTOs for IDMM (Intelligent Decision-Making Mode) — a per-session,
//! opt-in supervision capability that keeps agent/terminal sessions alive
//! through provider faults and decision stalls. Pure serde — no axum.
//!
//! Phase 2 重组为两个可独立开关、默认关的「值守」(spec §5.1/5.2):故障值守
//! (`fault_watch`)与决策值守(`decision_watch`),各持一套 [`WatchBase`] 旋钮 +
//! 旁路模型;决策值守额外带结构化决策策略([`DecisionStrategy`])与纯问答开关。

use serde::{Deserialize, Serialize};

/// Which kind of session IDMM supervises. The integer conversation/terminal
/// ids CAN collide numerically, so a target is keyed by `(kind, id)` — see the
/// IDMM supervisor's domain-qualified handle/shared maps (spec §2.2 C3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdmmTargetKind {
    Conversation,
    Terminal,
}

impl IdmmTargetKind {
    /// Parse the path-segment form (`conversation` | `terminal`).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "conversation" => Some(Self::Conversation),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }

    /// The persisted / path-segment string form (`conversation` | `terminal`).
    /// Inverse of [`Self::parse`]; the single source for the `target_kind`
    /// column + path segment, reused by the IDMM service + supervisor.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Terminal => "terminal",
        }
    }
}

/// 单个值守的处理档位(取代 Phase-1 的 `IdmmTier`)。
///
/// - `RuleOnly` = 无模型规则检查模式:只跑规则,不升级旁路模型。
/// - `RulePlusModel` = 旁路模型混合:规则扫描日志 + 模型做决策(可升级)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WatchTier {
    /// 无模型规则检查模式。
    #[default]
    RuleOnly,
    /// 规则 + 旁路模型混合。
    RulePlusModel,
}

/// 扫描内容范围:送给规则/旁路模型的会话上下文裁剪口径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScanScope {
    /// 仅末轮(默认,等价 Phase-1 行为)。
    #[default]
    LastTurn,
    /// 最近 N 条(N 由 `max_context_chars` 同侧的预算约束;条数语义在引擎层取用)。
    LastMessages,
    /// 全会话。
    FullSession,
}

/// 唤醒动作策略(故障值守)。`P2` 仅 `Retry` 生效;`Failover` 系列在 P3 接故障
/// 转移队列后启用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WakeStrategy {
    /// 仅重试 / 唤醒原会话(P2 默认且唯一生效)。
    #[default]
    Retry,
    /// 故障转移到队列中下一个可用模型(P3)。
    Failover,
    /// 先故障转移,失败再重试(P3)。
    FailoverThenRetry,
}

/// 决策倾向:影响规则档无推荐项时是否敢挑、纯问答果断程度、Idle 倾向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tendency {
    /// 保守:倾向 Halt。
    Conservative,
    /// 均衡(默认)。
    #[default]
    Balanced,
    /// 激进:更敢自动推进。
    Aggressive,
}

/// 被阻塞(无安全自动解)时的行为倾向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockedBehavior {
    /// 倾向继续(默认)。
    #[default]
    PreferContinue,
    /// 倾向暂停。
    PreferPause,
    /// 必须问人。
    MustAsk,
}

/// 分类规则的处理模式。`Auto` = 自动处理;`AskFirst`/`Off` 本期均退回人类
/// (Halt),不引入异步问人通道(详见 plan D5)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CategoryMode {
    /// 按规则/模型自动处理(默认)。
    #[default]
    Auto,
    /// 先问人(本期等同 Off:Halt)。
    AskFirst,
    /// 关闭该分类的自动处理(Halt)。
    Off,
}

fn default_scan_interval_secs() -> u32 {
    60
}
fn default_max_retries() -> u32 {
    5
}
fn default_true() -> bool {
    true
}
fn default_max_context_chars() -> u32 {
    8000
}
fn default_open_answer_chars() -> u32 {
    600
}
fn default_max_interventions_per_hour() -> u32 {
    30
}
fn default_min_interval_secs() -> u32 {
    20
}

/// Rate limits to keep IDMM from thrashing a session. 每个值守各持一份(预算/
/// 最小间隔按值守各自计,plan D4)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetConfig {
    #[serde(default = "default_max_interventions_per_hour")]
    pub max_interventions_per_hour: u32,
    #[serde(default = "default_min_interval_secs")]
    pub min_interval_secs: u32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_interventions_per_hour: default_max_interventions_per_hour(),
            min_interval_secs: default_min_interval_secs(),
        }
    }
}

/// 旁路模型供应商选择。空 → 全局默认(`idmm_backup_*`)→ 会话自身模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BypassModelRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// 两个值守共享的基础旋钮(spec §5.1)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchBase {
    /// 默认 false —— 值守 opt-in。
    #[serde(default)]
    pub enabled: bool,
    /// 处理档位:`RuleOnly`(无模型)| `RulePlusModel`(旁路模型混合)。
    #[serde(default)]
    pub tier: WatchTier,
    /// 监测间隔(=Phase-1 `idle_threshold_secs`,默认 60 秒)。
    #[serde(default = "default_scan_interval_secs")]
    pub scan_interval_secs: u32,
    /// 最大重试次数(默认 5)。
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// 扫描内容范围。
    #[serde(default)]
    pub scan_scope: ScanScope,
    /// 送给旁路模型的上下文字符上限(沿用 Phase-1 默认 8000)。
    #[serde(default = "default_max_context_chars")]
    pub max_context_chars: u32,
    /// 旁路模型供应商选择(空 → 全局默认 → 会话自身模型)。
    #[serde(default)]
    pub bypass_model: BypassModelRef,
    /// 预算/最小间隔(复用 Phase-1 `BudgetConfig`)。
    #[serde(default)]
    pub budget: BudgetConfig,
}

impl Default for WatchBase {
    fn default() -> Self {
        Self {
            enabled: false,
            tier: WatchTier::RuleOnly,
            scan_interval_secs: default_scan_interval_secs(),
            max_retries: default_max_retries(),
            scan_scope: ScanScope::LastTurn,
            max_context_chars: default_max_context_chars(),
            bypass_model: BypassModelRef::default(),
            budget: BudgetConfig::default(),
        }
    }
}

/// 故障值守:检测供应商故障 / 网络异常 → 自动唤醒、重试 / 故障转移。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FaultWatchConfig {
    #[serde(default, flatten)]
    pub base: WatchBase,
    /// 唤醒动作(P2 仅 `Retry` 生效)。
    #[serde(default)]
    pub wake_action: WakeStrategy,
    /// 是否调用模型故障转移队列(P3 用,P2 占位 false)。
    #[serde(default)]
    pub use_failover_queue: bool,
}

/// 选项决策规则。默认值等价 Phase-1 `RuleConfig`(plan D5):
/// `prefer_recommended` ↔ 旧 `auto_accept_recommended`;`allow_unmarked_pick` ↔
/// 旧 `auto_pick_unmarked`;`never_destructive` ↔ 旧 `!allow_destructive`(取反,
/// 默认 true = 不碰破坏性)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionRule {
    #[serde(default)]
    pub mode: CategoryMode,
    #[serde(default = "default_true")]
    pub prefer_recommended: bool,
    #[serde(default = "default_true")]
    pub allow_unmarked_pick: bool,
    #[serde(default = "default_true")]
    pub never_destructive: bool,
}

impl Default for OptionRule {
    fn default() -> Self {
        Self {
            mode: CategoryMode::Auto,
            prefer_recommended: true,
            allow_unmarked_pick: true,
            never_destructive: true,
        }
    }
}

/// 纯问答规则。`max_answer_chars` 默认 600(plan D2)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenQuestionRule {
    #[serde(default)]
    pub mode: CategoryMode,
    #[serde(default = "default_open_answer_chars")]
    pub max_answer_chars: u32,
}

impl Default for OpenQuestionRule {
    fn default() -> Self {
        Self {
            mode: CategoryMode::Auto,
            max_answer_chars: default_open_answer_chars(),
        }
    }
}

/// 权限确认规则。`only_safe_value` + `escalate_risky` 对应 Phase-1 安全闸
/// (只读工具自动 confirm,风险升级)——**安全闸不可破**(plan D5,默认 true)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    #[serde(default)]
    pub mode: CategoryMode,
    #[serde(default = "default_true")]
    pub only_safe_value: bool,
    #[serde(default = "default_true")]
    pub escalate_risky: bool,
}

impl Default for PermissionRule {
    fn default() -> Self {
        Self {
            mode: CategoryMode::Auto,
            only_safe_value: true,
            escalate_risky: true,
        }
    }
}

/// 三类分类规则(spec §5.2)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CategoryRules {
    #[serde(default)]
    pub option_decision: OptionRule,
    #[serde(default)]
    pub open_question: OpenQuestionRule,
    #[serde(default)]
    pub permission: PermissionRule,
}

/// 结构化决策策略(spec §5.2):倾向 + 阻塞行为 + 三类分类规则 + 自由文本兜底。
/// 结构化字段驱动规则档、约束模型档;`freeform_policy` 仅在模型档拼进 sidecar
/// 提示词(prompt.rs),不参与规则档。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DecisionStrategy {
    #[serde(default)]
    pub tendency: Tendency,
    #[serde(default)]
    pub on_blocked: BlockedBehavior,
    #[serde(default)]
    pub categories: CategoryRules,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeform_policy: Option<String>,
}

/// 决策值守:检测会话内问题(选项决策 + 纯问答)→ 自动作答。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionWatchConfig {
    #[serde(default, flatten)]
    pub base: WatchBase,
    #[serde(default)]
    pub strategy: DecisionStrategy,
    /// 纯问答开关。**旁路模型档默认开启**(选模型档即意味着让模型替你决策,
    /// 含开放式提问);可手动关闭。在规则档惰性无影响(规则档永不作答纯问答)。
    #[serde(default = "default_true")]
    pub answer_open_questions: bool,
}

impl Default for DecisionWatchConfig {
    fn default() -> Self {
        Self {
            base: WatchBase::default(),
            strategy: DecisionStrategy::default(),
            answer_open_questions: true,
        }
    }
}

/// The full per-session IDMM config (persisted as JSON in `conversation.extra.idmm`
/// or `terminal_sessions.idmm`).
///
/// Phase 2 形态:两个可独立开关、默认关的值守(故障值守 / 决策值守)。
///
/// **向后兼容(有意决策,plan D3 / spec §6.3 YAGNI 简化)**:Phase-1 的旧形态
/// `{enabled, tier, steering_prompt, rule, sidecar, budget}` **不做迁移映射**。
/// 旧功能此前「无法使用」,几乎无有效已存配置,且用户明确接受返工。新结构全字段
/// `#[serde(default)]`,旧 blob 的未知字段被 serde 忽略,故旧配置反序列化为默认值
/// (两个值守皆关,`enabled=false`)——**不报错、不 panic**。升级后旧配置回到默认
/// 关,需用户重新开启。见测试 `legacy_idmm_config_blob_deserializes_to_default`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdmmConfig {
    /// 故障值守。
    #[serde(default)]
    pub fault_watch: FaultWatchConfig,
    /// 决策值守。
    #[serde(default)]
    pub decision_watch: DecisionWatchConfig,
}

impl IdmmConfig {
    /// 「启用」当且仅当任一值守开启(plan D4)。`IdmmManager::ensure` / service 的
    /// enable 判断改读此。
    pub fn any_enabled(&self) -> bool {
        self.fault_watch.base.enabled || self.decision_watch.base.enabled
    }
}

/// Tri-state run state for the UI dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdmmRunState {
    /// Disabled.
    Off,
    /// Enabled, supervising, no active intervention.
    Armed,
    /// Mid-intervention right now.
    Intervening,
}

/// Live state surfaced to the client + the `idmm.statusChanged` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdmmState {
    pub kind: IdmmTargetKind,
    pub target_id: String,
    /// 任一值守启用即 true(plan D4:`enabled` 表示「任一值守开」)。
    pub enabled: bool,
    /// 故障值守是否启用(per-watch 运行概要,廉价补充)。
    #[serde(default)]
    pub fault_enabled: bool,
    /// 决策值守是否启用(per-watch 运行概要)。
    #[serde(default)]
    pub decision_enabled: bool,
    pub run_state: IdmmRunState,
    pub interventions_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_intervention_at: Option<i64>,
    /// Whether a backup provider is resolvable (per-session or global default).
    pub sidecar_provider_resolved: bool,
    /// The persisted per-session `IdmmConfig`, included so the frontend can
    /// rehydrate its form (per-watch tier, bypass model, strategy rules) on
    /// remount instead of reconstructing from scratch and silently dropping the
    /// user's saved config. Absent for targets that have never been configured
    /// (the frontend then falls back to the global defaults via `IdmmSettings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<IdmmConfig>,
}

impl IdmmState {
    /// Compute the tri-state run state from enabled + intervening flags.
    pub fn run_state(enabled: bool, intervening: bool) -> IdmmRunState {
        if !enabled {
            IdmmRunState::Off
        } else if intervening {
            IdmmRunState::Intervening
        } else {
            IdmmRunState::Armed
        }
    }
}

/// One row of the intervention audit log + the `idmm.intervention` payload.
///
/// 向后兼容:新增字段全部带 `#[serde(default)]`,旧 WS 消费方(只看
/// stall_class/tier_used/action/outcome)发来的精简对象仍能反序列化,新字段取
/// 默认值。`Option` 字段额外 `skip_serializing_if` 在缺省时不入 wire。
///
/// **Phase-2 不变量**:本类型字段与 `outcome` 文档**逐字沿用 Phase-1**,勿改。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterventionRecord {
    /// `idmmrec_{uuidv7}` — 落库主键。旧消费方不带,故 `default`。
    #[serde(default)]
    pub id: String,
    /// "conversation" | "terminal"。旧消费方不带,故 `default`。
    #[serde(default)]
    pub target_kind: String,
    pub target_id: String,
    /// "fault" | "decision"。旧消费方不带,故 `default`。
    #[serde(default)]
    pub watch: String,
    pub at: i64,
    /// "provider_error" | "idle" | "decision" | "scheduled".
    pub stall_class: String,
    /// "rule" | "sidecar" | "rule_fallback".
    pub tier_used: String,
    /// "option" | "open_question" | "permission" | "fault"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// "retry" | "send_text" | "answer_choice" | "wait" | "stop".
    pub action: String,
    /// 选了什么/答了什么(截断 ≤2000 字符)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Canonical disposition token the UI renders/colors. One of
    /// `applied | resolved | failed | halted | skipped` — Phase-1 emits
    /// `applied` (an action was injected) or `halted` (stood down, needs a
    /// human). The free-form *why* lives in `reason`, never here.
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 模型置信度(规则档 None)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// provider/model(规则档 None)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass_model: Option<String>,
}

/// Request body for `POST /api/idmm`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetIdmmRequest {
    pub kind: IdmmTargetKind,
    /// Session id handle. Accepts both a JSON integer (what the frontend sends —
    /// ids are numeric) and a JSON string; see
    /// [`crate::serde_util::deserialize_target_id`]. Without this, enabling
    /// 会话→智能决策 (which POSTs a numeric `target_id`) is rejected by serde
    /// and surfaces as a 400.
    #[serde(deserialize_with = "crate::serde_util::deserialize_target_id")]
    pub target_id: String,
    #[serde(flatten)]
    pub config: IdmmConfig,
}

/// Global IDMM defaults (`GET/PUT /api/idmm/settings`), stored in `client_preferences`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdmmSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_model: Option<String>,
    #[serde(default)]
    pub default_steering_prompt: String,
}

fn default_max_switches() -> u32 {
    4
}

/// Phase 3 模型故障转移队列(spec §5.5)。独立于 IDMM 生效,IDMM 故障值守也能触发它。
///
/// 全局存于 `client_preferences` 键 `agent.model_failover`(整体 JSON);会话级
/// 可在 `conversations.extra.model_failover` 覆盖(存在则优先于全局)。所有字段
/// 带 serde 默认,故空对象 → 关闭、空队列、`max_switches=4`、`stamp_unhealthy=true`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelFailoverConfig {
    /// 默认 false:不配置即不转移。
    #[serde(default)]
    pub enabled: bool,
    /// 有序候选 `(provider_id, model, use_model?)`;按序挑下一个可用模型。
    #[serde(default)]
    pub queue: Vec<nomifun_common::ProviderWithModel>,
    /// 单轮最大切换次数(默认 4;实际还受队列长度封顶)。
    #[serde(default = "default_max_switches")]
    pub max_switches: u32,
    /// 默认 true:故障时把失败模型的 `model_health` 标 `Unhealthy`。
    #[serde(default = "default_true")]
    pub stamp_unhealthy: bool,
}

impl Default for ModelFailoverConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            queue: Vec::new(),
            max_switches: default_max_switches(),
            stamp_unhealthy: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认空对象 → 两个值守皆关,且各旋钮回到 Phase-1 等价默认(D5)。
    #[test]
    fn idmm_config_defaults_deserialize_from_empty_object() {
        let cfg: IdmmConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.any_enabled());
        // 故障值守
        assert!(!cfg.fault_watch.base.enabled);
        assert_eq!(cfg.fault_watch.base.tier, WatchTier::RuleOnly);
        assert_eq!(cfg.fault_watch.base.scan_interval_secs, 60);
        assert_eq!(cfg.fault_watch.base.max_retries, 5);
        assert_eq!(cfg.fault_watch.base.scan_scope, ScanScope::LastTurn);
        assert_eq!(cfg.fault_watch.base.max_context_chars, 8000);
        assert!(cfg.fault_watch.base.bypass_model.provider_id.is_none());
        assert_eq!(cfg.fault_watch.base.budget.max_interventions_per_hour, 30);
        assert_eq!(cfg.fault_watch.base.budget.min_interval_secs, 20);
        assert_eq!(cfg.fault_watch.wake_action, WakeStrategy::Retry);
        assert!(!cfg.fault_watch.use_failover_queue);
        // 决策值守
        assert!(!cfg.decision_watch.base.enabled);
        assert_eq!(cfg.decision_watch.base.tier, WatchTier::RuleOnly);
        // 纯问答默认开(旁路模型档生效;规则档惰性无影响)。
        assert!(cfg.decision_watch.answer_open_questions);
    }

    /// D5:决策策略默认值必须等价 Phase-1 行为(开箱即用且安全)。
    #[test]
    fn decision_strategy_defaults_match_phase1_behavior() {
        let s = DecisionStrategy::default();
        assert_eq!(s.tendency, Tendency::Balanced);
        assert_eq!(s.on_blocked, BlockedBehavior::PreferContinue);
        assert!(s.freeform_policy.is_none());
        // 选项决策:prefer_recommended ↔ auto_accept_recommended、
        // allow_unmarked_pick ↔ auto_pick_unmarked、never_destructive ↔ !allow_destructive
        let opt = &s.categories.option_decision;
        assert_eq!(opt.mode, CategoryMode::Auto);
        assert!(opt.prefer_recommended);
        assert!(opt.allow_unmarked_pick);
        assert!(opt.never_destructive);
        // 权限:只读放行 + 风险升级(安全闸不可破)
        let perm = &s.categories.permission;
        assert_eq!(perm.mode, CategoryMode::Auto);
        assert!(perm.only_safe_value);
        assert!(perm.escalate_risky);
        // 纯问答默认 600 字符上限
        assert_eq!(s.categories.open_question.mode, CategoryMode::Auto);
        assert_eq!(s.categories.open_question.max_answer_chars, 600);
    }

    /// D3 向后兼容:旧形态 blob 反序列化为默认(两值守关),不报错。
    #[test]
    fn legacy_idmm_config_blob_deserializes_to_default() {
        // 完整的 Phase-1 旧形态,含 rule/sidecar 子对象。
        let legacy = serde_json::json!({
            "enabled": true,
            "tier": "rule_plus_sidecar",
            "steering_prompt": "prefer the recommended option; never delete data",
            "rule": {
                "idle_threshold_secs": 90,
                "auto_retry": true,
                "max_retries": 9,
                "auto_accept_recommended": true,
                "auto_pick_unmarked": false,
                "allow_destructive": true
            },
            "sidecar": {
                "provider_id": "openrouter",
                "model": "gpt-x",
                "read_history": true,
                "session_mode": "continuous",
                "max_context_chars": 12000,
                "confidence_floor": 0.4,
                "scheduled_check": {"enabled": true, "every_secs": 300}
            },
            "budget": {"max_interventions_per_hour": 99, "min_interval_secs": 1}
        });
        let cfg: IdmmConfig =
            serde_json::from_value(legacy).expect("legacy blob must deserialize, not error");
        // 旧未知字段被忽略 → 两值守皆关。
        assert!(!cfg.any_enabled());
        assert!(!cfg.fault_watch.base.enabled);
        assert!(!cfg.decision_watch.base.enabled);
    }

    /// 新形态往返:配置 → JSON → 配置 保真。
    #[test]
    fn new_shape_roundtrip() {
        let cfg = IdmmConfig {
            fault_watch: FaultWatchConfig {
                base: WatchBase {
                    enabled: true,
                    tier: WatchTier::RulePlusModel,
                    scan_interval_secs: 45,
                    max_retries: 3,
                    scan_scope: ScanScope::FullSession,
                    max_context_chars: 5000,
                    bypass_model: BypassModelRef {
                        provider_id: Some("openrouter".into()),
                        model: Some("gpt-x".into()),
                    },
                    budget: BudgetConfig {
                        max_interventions_per_hour: 10,
                        min_interval_secs: 5,
                    },
                },
                wake_action: WakeStrategy::FailoverThenRetry,
                use_failover_queue: true,
            },
            decision_watch: DecisionWatchConfig {
                base: WatchBase {
                    enabled: true,
                    tier: WatchTier::RulePlusModel,
                    ..WatchBase::default()
                },
                strategy: DecisionStrategy {
                    tendency: Tendency::Aggressive,
                    on_blocked: BlockedBehavior::MustAsk,
                    categories: CategoryRules {
                        option_decision: OptionRule {
                            mode: CategoryMode::Auto,
                            prefer_recommended: false,
                            allow_unmarked_pick: false,
                            never_destructive: true,
                        },
                        open_question: OpenQuestionRule {
                            mode: CategoryMode::Off,
                            max_answer_chars: 1200,
                        },
                        permission: PermissionRule::default(),
                    },
                    freeform_policy: Some("stay on task, never touch prod".into()),
                },
                answer_open_questions: true,
            },
        };
        let json = serde_json::to_value(&cfg).unwrap();
        // flatten 把 base 字段提到 fault_watch / decision_watch 顶层。
        assert_eq!(json["fault_watch"]["enabled"], serde_json::json!(true));
        assert_eq!(json["fault_watch"]["tier"], serde_json::json!("rule_plus_model"));
        assert_eq!(json["fault_watch"]["wake_action"], serde_json::json!("failover_then_retry"));
        assert_eq!(json["decision_watch"]["answer_open_questions"], serde_json::json!(true));
        assert_eq!(json["decision_watch"]["strategy"]["tendency"], serde_json::json!("aggressive"));
        let back: IdmmConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, cfg);
    }

    /// flatten 让 `SetIdmmRequest` 与值守配置在同一对象里共存(真实 payload 形态)。
    #[test]
    fn set_idmm_request_flattens_config() {
        let json = serde_json::json!({
            "kind": "terminal",
            "target_id": "t1",
            "fault_watch": {"enabled": true, "tier": "rule_plus_model"},
            "decision_watch": {"enabled": true, "answer_open_questions": true}
        });
        let req: SetIdmmRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.kind, IdmmTargetKind::Terminal);
        assert_eq!(req.target_id, "t1");
        assert!(req.config.any_enabled());
        assert!(req.config.fault_watch.base.enabled);
        assert_eq!(req.config.fault_watch.base.tier, WatchTier::RulePlusModel);
        assert!(req.config.decision_watch.answer_open_questions);
    }

    #[test]
    fn idmm_run_state_tristate() {
        assert_eq!(IdmmState::run_state(false, false), IdmmRunState::Off);
        assert_eq!(IdmmState::run_state(false, true), IdmmRunState::Off);
        assert_eq!(IdmmState::run_state(true, false), IdmmRunState::Armed);
        assert_eq!(IdmmState::run_state(true, true), IdmmRunState::Intervening);
    }

    #[test]
    fn watch_tier_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(WatchTier::RulePlusModel).unwrap(),
            serde_json::json!("rule_plus_model")
        );
        assert_eq!(serde_json::to_value(WatchTier::RuleOnly).unwrap(), serde_json::json!("rule_only"));
    }

    #[test]
    fn target_kind_and_run_state_serialize_snake_case() {
        assert_eq!(
            serde_json::to_value(IdmmTargetKind::Conversation).unwrap(),
            serde_json::json!("conversation")
        );
        assert_eq!(
            serde_json::to_value(IdmmRunState::Intervening).unwrap(),
            serde_json::json!("intervening")
        );
    }

    /// Regression: enabling 会话→智能决策 POSTs `target_id` as a JSON NUMBER
    /// (the frontend models session ids numerically). The backend keeps it as a
    /// String handle; deserialization must accept the integer instead of
    /// rejecting it with "invalid type: integer N, expected a string" (the 400
    /// testers hit). Mirrors the AutoWork fix in `requirement.rs`.
    #[test]
    fn set_idmm_request_accepts_numeric_target_id() {
        let body = r#"{"kind":"conversation","target_id":2,"fault_watch":{"enabled":true}}"#;
        let req: SetIdmmRequest =
            serde_json::from_str(body).expect("numeric target_id must deserialize");
        assert_eq!(req.target_id, "2");
        assert_eq!(req.kind, IdmmTargetKind::Conversation);
        assert!(req.config.fault_watch.base.enabled);
    }

    /// A numeric `target_id` coexists with the flattened `IdmmConfig`.
    #[test]
    fn set_idmm_request_accepts_numeric_target_id_with_flattened_config() {
        let json = serde_json::json!({
            "kind": "conversation",
            "target_id": 12345,
            "decision_watch": {"enabled": true, "strategy": {"tendency": "conservative"}}
        });
        let req: SetIdmmRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.target_id, "12345");
        assert_eq!(req.kind, IdmmTargetKind::Conversation);
        assert!(req.config.decision_watch.base.enabled);
        assert_eq!(
            req.config.decision_watch.strategy.tendency,
            Tendency::Conservative
        );
    }

    /// A string `target_id` (forward-compatible / other clients) still works.
    #[test]
    fn set_idmm_request_accepts_string_target_id() {
        let body = r#"{"kind":"terminal","target_id":"term_7"}"#;
        let req: SetIdmmRequest =
            serde_json::from_str(body).expect("string target_id must deserialize");
        assert_eq!(req.target_id, "term_7");
        assert_eq!(req.kind, IdmmTargetKind::Terminal);
        assert!(!req.config.any_enabled());
    }

    #[test]
    fn intervention_record_enriched_fields_roundtrip() {
        let r = InterventionRecord {
            id: "idmmrec_x".into(),
            target_kind: "conversation".into(),
            target_id: "c1".into(),
            watch: "decision".into(),
            at: 1,
            stall_class: "decision".into(),
            tier_used: "sidecar".into(),
            category: Some("open_question".into()),
            action: "send_text".into(),
            detail: Some("用方案B".into()),
            outcome: "applied".into(),
            reason: Some("因为…".into()),
            confidence: Some(0.82),
            bypass_model: Some("prov:gpt-x".into()),
        };
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["category"], "open_question");
        // confidence 是 f32,序列化为 JSON 后带 f32 精度尾差(0.82 → 0.8199999…),
        // 故按 f32 提升后的 f64 值比对,而非裸字面量。
        assert_eq!(j["confidence"].as_f64().unwrap(), 0.82_f32 as f64);
    }

    #[test]
    fn intervention_record_back_compat_minimal_object() {
        // 旧 WS 消费方只看 stall_class/tier_used/action/outcome;新增字段 default。
        let j = serde_json::json!({
            "id":"x","target_kind":"terminal","target_id":"t1","watch":"fault","at":2,
            "stall_class":"provider_error","tier_used":"rule","action":"retry","outcome":"applied"
        });
        let r: InterventionRecord = serde_json::from_value(j).unwrap();
        assert!(r.category.is_none());
        assert!(r.confidence.is_none());
    }

    #[test]
    fn idmm_state_omits_optional_none_fields() {
        let st = IdmmState {
            kind: IdmmTargetKind::Conversation,
            target_id: "c1".into(),
            enabled: false,
            fault_enabled: false,
            decision_enabled: false,
            run_state: IdmmRunState::Off,
            interventions_count: 0,
            last_signal: None,
            last_intervention_at: None,
            sidecar_provider_resolved: false,
            config: None,
        };
        let json = serde_json::to_value(&st).unwrap();
        assert!(json.get("last_signal").is_none());
        assert!(json.get("last_intervention_at").is_none());
        assert!(json.get("config").is_none());
        assert_eq!(json["run_state"], serde_json::json!("off"));
    }

    /// D1:空对象 → 关闭、空队列、`max_switches=4`、`stamp_unhealthy=true`,
    /// 与 [`ModelFailoverConfig::default`] 等价。
    #[test]
    fn model_failover_config_defaults_from_empty_object() {
        let cfg: ModelFailoverConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.enabled);
        assert!(cfg.queue.is_empty());
        assert_eq!(cfg.max_switches, 4);
        assert!(cfg.stamp_unhealthy);
        assert_eq!(cfg, ModelFailoverConfig::default());
    }

    /// 队列与显式旋钮 round-trip;`use_model` 可选项保留。
    #[test]
    fn model_failover_config_roundtrip() {
        let json = serde_json::json!({
            "enabled": true,
            "queue": [
                {"provider_id": "openrouter", "model": "gpt-x", "use_model": null},
                {"provider_id": "anthropic", "model": "claude", "use_model": "claude-alias"}
            ],
            "max_switches": 2,
            "stamp_unhealthy": false
        });
        let cfg: ModelFailoverConfig = serde_json::from_value(json).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.queue.len(), 2);
        assert_eq!(cfg.queue[0].provider_id, "openrouter");
        assert_eq!(cfg.queue[1].use_model.as_deref(), Some("claude-alias"));
        assert_eq!(cfg.max_switches, 2);
        assert!(!cfg.stamp_unhealthy);
    }
}
