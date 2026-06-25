//! 动作层类型契约（P2 地基，Stage A）：`act` 的输入/输出形状。
//!
//! [`ActSpec`] 是 facade 解析 LLM tool input 的统一动作枚举——`#[serde(tag="action")]`
//! 让每个变体在 JSON 里以 `{"action":"click", ...}` 形态出现（snake_case），既能从工具
//! 入参反序列化，也能回灌日志/回放。[`ActResult`]/[`Effect`] 是动作执行的产物：人读的
//! `message` + 「页面是否真变了」的 [`Effect`]（before/after 锚点供前后对比）。
//!
//! **安全红线**：[`TypeInput::Secret`] 的明文**绝不**进 LLM / 日志 / Debug 输出——本模块
//! 手写 `Debug` 把 secret 脱敏为 `<redacted>`（同文件测试守卫）。Serialize 仍透出原值
//! （写回密码字段需要），但任何 `{:?}` 路径都不泄露。
//!
//! **范围（Stage A）**：只定义类型 + serde 契约；真执行逻辑在 P2 Stage B/C
//! （[`crate::backend::cdp::CdpBackend::act`] 当前是 `Unsupported` stub）。
//!
//! **范围（B6，本文件后补）**：[`run_act_with_retry`] —— 动作层短重试编排（退避
//! [`BACKOFF`] = `[0,20,50,100,100,500]`ms + IRREVERSIBLE 禁重试），全程跑在
//! [`crate::progress::Progress::race`]（abort 优先 timeout）上下文里。它是 C1 各动作
//! （click/type/…）的统一外壳；B6 只做编排，不实现具体动作。detach/crash → `progress.abort`
//! 的事件源接线在 [`crate::backend::cdp`] 的 act 入口（act 期间临时订阅）。

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use crate::engine::BrowserError;
use crate::errmap::map_progress_err;
use crate::progress::Progress;

/// **动作层短重试的退避序列**（DESIGN §11，设计裁决⑮）：每次重判之间 sleep 的毫秒数。
/// 第 0 次尝试**不**等待（`0`），之后 `20→50→100→100→500`ms 递增。共 6 个槽位即最多
/// 重试 6 次（首次尝试 + 5 次退避后重试 = 共 6 次 `op` 调用上限）。镜像 vendored PW 的
/// `[0,20,50,100,100,500]`（与 `[0,20,100,100,500]` 同源；我们取前者的 6 槽形态）。
pub const BACKOFF: [u64; 6] = [0, 20, 50, 100, 100, 500];

/// 一次 `op` 尝试的结果裁决（[`run_act_with_retry`] 据此决定重试 / 立返 / 成功）。
///
/// `op` 返 `Result<T, RetryDecision>`：成功是 `Ok(T)`；失败按本枚举分流——
/// - [`RetryDecision::Retryable`]：**瞬态可重试**缺态。镜像 actionability 五检查的
///   [`crate::actionability::CheckResult::Missing`]（visible/stable/enabled/暂时 readonly）
///   与「代际内漂移」类 stale（[`BrowserError::NotConnected`]）——等一拍重判可能就绪。走退避循环。
/// - [`RetryDecision::Fatal`]：**NonRecoverable**，立返、**不**重试。镜像 IDMM「Decision 失败禁
///   Retry」与五检查的不可编辑特例（[`BrowserError::Blocked`]，元素类型根本不支持编辑）、代际层
///   stale（[`BrowserError::NodeStale`]，需上层重拍快照而非动作层短重试）。携带要上抛的 `BrowserError`。
///
/// **IRREVERSIBLE 动作**另由 [`run_act_with_retry`] 的 `irreversible` 参数强制「只试一次」——
/// 即便 `op` 返 `Retryable` 也不进退避循环（绝不自动重试不可逆动作；DESIGN §22）。
#[derive(Debug)]
pub enum RetryDecision {
    /// 瞬态缺态 / 代际内漂移：走退避后重判。`reason` 仅供诊断（退避耗尽时作为最终错误的载荷）。
    Retryable(BrowserError),
    /// NonRecoverable：立返、不重试。携带要上抛给引擎调用方的错误。
    Fatal(BrowserError),
}

/// **动作模式三档**（DESIGN §11 三级兜底）。当前只交付引擎内部档位 + 决策逻辑；
/// 因 [`ActSpec`] 暂无 `mode` 字段，facade 尚未接线，所有动作隐式走 [`ActMode::Actionable`]。
///
/// - [`ActMode::Actionable`]（默认）：跑全部 actionability 检查（visible/stable/enabled/[editable]）
///   + hit-target 三步舞，再真投递。这是已交付的主路径，行为不变。
/// - [`ActMode::Force`]：**绕过** actionability 检查与 hit-target（直接 `dispatchEvent`），仍真投递。
///   用于「检查太严、人工确认可点」的逃生。**不**豁免不可逆动作的 facade 层 hard-deny（force 只跳
///   actionability，不碰带外确认红线，见计划锁定不变量）。
/// - [`ActMode::Trial`]：**只判定不执行**——跑全部检查判元素是否可达，但**不**投递任何事件（dry-run，
///   用于「这个元素现在能点吗」的探测）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActMode {
    /// 默认档：全检查 + hit-target + 真投递。
    #[default]
    Actionable,
    /// 绕检查直投递（force）。
    Force,
    /// 只判定不投递（trial / dry-run）。
    Trial,
}

/// **[纯逻辑] 该模式是否跳过 actionability 检查 + hit-target**（true = 绕过，直 dispatchEvent）。
/// 仅 [`ActMode::Force`] 跳过；Actionable/Trial 都跑检查。
pub fn mode_skips_checks(mode: ActMode) -> bool {
    matches!(mode, ActMode::Force)
}

/// **[纯逻辑] 该模式是否真投递事件**（false = trial dry-run，只判定不执行）。
/// Actionable/Force 都投递；仅 [`ActMode::Trial`] 不投递。
pub fn mode_dispatches(mode: ActMode) -> bool {
    !matches!(mode, ActMode::Trial)
}

/// **动作层重试编排**（DESIGN §11，设计裁决⑮）：把一个**可重判的** `op`（每次尝试内部跑
/// per-action 五检查重判 + 执行）放进退避循环，全程跑在 [`Progress::race`]（abort 优先于 timeout）
/// 上下文里。这是 C1 各动作（click/type/…）的统一外壳；B6 只做编排，不实现具体动作。
///
/// 形态（`op: FnMut(usize) -> Future<Output = Result<T, RetryDecision>>`）：
/// - 入参是 **attempt 序号**（从 0 起），让 `op` 可按尝试次数调整策略（如 scroll alignment 轮转）。
/// - `Ok(T)` —— 成功，立返。
/// - `Err(RetryDecision::Retryable(_))` —— 瞬态：若还有退避槽位则 sleep 后重判；耗尽则上抛该错误。
/// - `Err(RetryDecision::Fatal(e))` —— NonRecoverable：**立返** `Err(e)`，不再重试。
///
/// **每次 attempt 与 `progress.race` 竞速**：`op` 进行中 page.close/frame.detach → progress.abort →
/// race 立即以 `Aborted` 返回（远早于 deadline），经 [`map_progress_err`] 成 `TargetClosed`/
/// `Detached`/…。deadline 先到 → `Timeout{phase:Action}`。退避 sleep 用 [`tokio::time::sleep`]，
/// 测试可用虚拟时钟（`start_paused`）确定性推进。
///
/// **IRREVERSIBLE**（`irreversible == true`）：**只试一次**（DESIGN §22「不可逆动作绝不自动重试」，
/// 镜像 IDMM「Decision 失败禁 Retry」）。`op` 返 `Retryable` 也**不**进退避——直接把该 `Retryable`
/// 的载荷错误上抛（attempt 计数恒为 1）。`Fatal` 同样立返。
///
/// **绝不 panic**：所有错误经类型系统上抛。
pub async fn run_act_with_retry<F, Fut, T>(
    progress: &Progress,
    irreversible: bool,
    mut op: F,
) -> Result<T, BrowserError>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T, RetryDecision>>,
{
    // IRREVERSIBLE：退避序列坍缩成单槽（只第 0 次尝试，不等待、不重试）。
    let backoff: &[u64] = if irreversible { &BACKOFF[..1] } else { &BACKOFF };

    let mut last_retryable: Option<BrowserError> = None;
    for (attempt, &delay_ms) in backoff.iter().enumerate() {
        // 退避：第 0 次尝试 delay==0（不 sleep）；之后按序列 sleep。sleep 本身也与 progress 竞速——
        // 退避期间 page.close/frame.detach 应立即打断（而非白等一个已经没意义的退避）。
        if delay_ms > 0 {
            // race 的成功分支返回 sleep 的 `()`；abort/timeout 经 map_progress_err 立返。
            progress
                .race(tokio::time::sleep(Duration::from_millis(delay_ms)))
                .await
                .map_err(map_progress_err)?;
        }

        // 本次尝试：op 与 (deadline, abort) 三方竞速。abort/timeout 优先，经 map_progress_err。
        match progress.race(op(attempt)).await.map_err(map_progress_err)? {
            Ok(value) => return Ok(value),
            Err(RetryDecision::Fatal(e)) => return Err(e),
            Err(RetryDecision::Retryable(e)) => {
                // IRREVERSIBLE：单槽循环，下一轮不会发生——这里直接上抛（不自动重试不可逆动作）。
                // 非 IRREVERSIBLE：记下最后一次瞬态错误，继续退避；退避耗尽则它成为最终错误。
                last_retryable = Some(e);
                // 不可逆：立即结束（循环本就只有 1 槽，但显式 break 以表意「绝不重试」）。
                if irreversible {
                    break;
                }
            }
        }
    }

    // 退避耗尽仍 Retryable（或 IRREVERSIBLE 单次即 Retryable）→ 上抛最后一次瞬态错误。
    // 该错误是 op 给出的具体瞬态分类（NotConnected / Missing 映射的 Other / …），保留语义供 LLM 路由。
    // last_retryable 恒为 Some（循环至少跑一轮且 Retryable 分支必写它）；兜底 Timeout{Action} 仅防御性。
    Err(last_retryable.unwrap_or(BrowserError::Timeout {
        phase: crate::engine::NavPhase::Action,
    }))
}

/// 一次 `act` 要执行的动作。`#[serde(tag="action", rename_all="snake_case")]`：JSON 里以
/// `{"action":"click","ref":"f0e3"}` 形态出现，作为 facade 解析 LLM tool input 的入口。
///
/// `ref` 是 [`crate::engine::Observation`] 投影给 LLM 的稳定句柄（`f<seq>e<n>`，frame-local）。
///
/// **安全红线（`SetValue { secret: true }`）**：`SetValue` 的 `value` 是裸 `String`（要传给注入侧
/// `fill`/`insertText` 真键入），无法像 [`TypeInput`] 那样把明文藏进脱敏变体里。故本枚举**手写
/// `Debug`**（见下方 impl）：当 `SetValue.secret == true` 时把 `value` 显示为 `<redacted>`，镜像
/// [`TypeInput::Secret`] 的 Debug——任何 `{:?}`/`dbg!`/`tracing` 路径都不泄漏 set_value 的 secret 明文。
/// `secret` 标志还驱动 [`CdpBackend::act_set_value`] 抑制 before/after verify 锚点（不把 read-back 的
/// 明文回填进 [`Effect`]，否则 F2 把 anchor 透进 ToolResult 就 live 泄漏）。Serialize 仍透出原值
/// （写回密码字段需要真值上线），脱敏只针对 Debug——与 `TypeInput` 同契约。
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ActSpec {
    /// 点击一个元素。
    Click { r#ref: String },
    /// 悬停一个元素。
    Hover { r#ref: String },
    /// 在一个元素里键入文本（明文或脱敏 secret，见 [`TypeInput`]）。
    Type { r#ref: String, text: TypeInput },
    /// 直接给一个表单控件设值（不模拟逐字键入）。
    ///
    /// `secret == true` 标记 `value` 是敏感凭据（来自 facade 的 `secret:NAME` 解析）：Debug 脱敏
    /// （见枚举级 `Debug` impl）+ 引擎抑制 verify 锚点（[`CdpBackend::act_set_value`]）。
    /// `value` 本身仍是明文（要给 `insertText`/`fill`），`secret` 只让序列化/Debug/anchor 知道脱敏。
    SetValue {
        r#ref: String,
        value: String,
        #[serde(default)]
        secret: bool,
    },
    /// 在一个 `<select>` 上选项（按可见文本/value，可多选）。
    SelectOption { r#ref: String, options: Vec<String> },
    /// 发送键（组合键如 `"Control+A"` / 单键如 `"Enter"`）。
    PressKey { keys: String },
    /// 滚动（视口或某元素，方向 + 可选量）。
    Scroll {
        target: ScrollTarget,
        direction: ScrollDir,
        amount: Option<f64>,
    },
    /// 滚动直到某文本可见。
    ScrollToText { text: String },
    /// 取整页可读文本。
    GetPageText,
    /// 在当前页内查找文本（命中位置/上下文）。
    SearchPage { query: String },
    /// 用 CSS 选择器找元素。
    FindElements { selector: String },
    /// 取某下拉控件的可选项列表。
    GetDropdownOptions { r#ref: String },
    /// 显示/查询光标当前位置（调试/对齐用）。
    Cursor,
    /// 固定等待若干毫秒。
    Wait { ms: u64 },
    /// 等到某条件满足（见 [`WaitCondition`]）。
    WaitFor { condition: WaitCondition },
    /// 给一个 `<input type=file>` 设置上传文件路径。
    UploadFile { r#ref: String, paths: Vec<PathBuf> },
    /// 触发下载某 URL。
    Download { url: String },
    /// 把当前页另存为 PDF。
    SaveAsPdf,
    /// 按给定 JSON schema 抽取结构化数据。
    Extract { schema: serde_json::Value },
    /// 切换到某 iframe（后续动作在该帧上下文执行）。
    SwitchFrame { r#ref: String },
    /// 列出当前所有标签页。
    Tabs,
    /// 切换到某标签页。
    SwitchTab { tab_id: String },
    /// 关闭某标签页。
    CloseTab { tab_id: String },
    /// 在新标签页打开某链接。
    OpenLinkNewTab { url: String },
    /// 后退。
    Back,
    /// 前进。
    Forward,
    /// 刷新当前页。
    Reload,
    /// 导航到某 URL（可在新标签页）。
    Navigate { url: String, new_tab: bool },
    /// 在页面上下文执行一段脚本（受限/审计；高风险）。
    Evaluate { script: String },
    /// 取 console 日志（读取 per-tab 调试缓冲，只读）。
    GetConsoleLogs,
    /// 取页面错误（未捕获异常 + error-level 日志，只读）。
    GetPageErrors,
    /// 取网络请求日志（只读）。`include_bodies` 默认 false（bodies 重且易含 secret）。
    GetNetworkLog {
        #[serde(default)]
        include_bodies: bool,
    },
}

impl std::fmt::Debug for ActSpec {
    /// 手写 `Debug`：唯一与 derive 的差异是 [`ActSpec::SetValue`] 在 `secret == true` 时把 `value`
    /// 显示为 `<redacted>`（镜像 [`TypeInput::Secret`] 的 Debug），其余变体逐字段照常打印。任何
    /// `{:?}`/`dbg!`/`tracing` 路径（含 `dbg!` 整个 spec）都不会泄漏 set_value 的 secret 明文。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActSpec::Click { r#ref } => f.debug_struct("Click").field("ref", r#ref).finish(),
            ActSpec::Hover { r#ref } => f.debug_struct("Hover").field("ref", r#ref).finish(),
            ActSpec::Type { r#ref, text } => f
                .debug_struct("Type")
                .field("ref", r#ref)
                .field("text", text) // TypeInput 自身的 Debug 已脱敏 Secret。
                .finish(),
            // 安全红线：secret set_value 的 value 脱敏（裸 String 无法靠类型藏明文，靠这里）。
            ActSpec::SetValue {
                r#ref,
                value,
                secret,
            } => {
                let mut s = f.debug_struct("SetValue");
                s.field("ref", r#ref);
                if *secret {
                    s.field("value", &"<redacted>");
                } else {
                    s.field("value", value);
                }
                s.field("secret", secret).finish()
            }
            ActSpec::SelectOption { r#ref, options } => f
                .debug_struct("SelectOption")
                .field("ref", r#ref)
                .field("options", options)
                .finish(),
            ActSpec::PressKey { keys } => f.debug_struct("PressKey").field("keys", keys).finish(),
            ActSpec::Scroll {
                target,
                direction,
                amount,
            } => f
                .debug_struct("Scroll")
                .field("target", target)
                .field("direction", direction)
                .field("amount", amount)
                .finish(),
            ActSpec::ScrollToText { text } => {
                f.debug_struct("ScrollToText").field("text", text).finish()
            }
            ActSpec::GetPageText => f.write_str("GetPageText"),
            ActSpec::SearchPage { query } => {
                f.debug_struct("SearchPage").field("query", query).finish()
            }
            ActSpec::FindElements { selector } => f
                .debug_struct("FindElements")
                .field("selector", selector)
                .finish(),
            ActSpec::GetDropdownOptions { r#ref } => f
                .debug_struct("GetDropdownOptions")
                .field("ref", r#ref)
                .finish(),
            ActSpec::Cursor => f.write_str("Cursor"),
            ActSpec::Wait { ms } => f.debug_struct("Wait").field("ms", ms).finish(),
            ActSpec::WaitFor { condition } => f
                .debug_struct("WaitFor")
                .field("condition", condition)
                .finish(),
            ActSpec::UploadFile { r#ref, paths } => f
                .debug_struct("UploadFile")
                .field("ref", r#ref)
                .field("paths", paths)
                .finish(),
            ActSpec::Download { url } => f.debug_struct("Download").field("url", url).finish(),
            ActSpec::SaveAsPdf => f.write_str("SaveAsPdf"),
            ActSpec::Extract { schema } => {
                f.debug_struct("Extract").field("schema", schema).finish()
            }
            ActSpec::SwitchFrame { r#ref } => {
                f.debug_struct("SwitchFrame").field("ref", r#ref).finish()
            }
            ActSpec::Tabs => f.write_str("Tabs"),
            ActSpec::SwitchTab { tab_id } => {
                f.debug_struct("SwitchTab").field("tab_id", tab_id).finish()
            }
            ActSpec::CloseTab { tab_id } => {
                f.debug_struct("CloseTab").field("tab_id", tab_id).finish()
            }
            ActSpec::OpenLinkNewTab { url } => f
                .debug_struct("OpenLinkNewTab")
                .field("url", url)
                .finish(),
            ActSpec::Back => f.write_str("Back"),
            ActSpec::Forward => f.write_str("Forward"),
            ActSpec::Reload => f.write_str("Reload"),
            ActSpec::Navigate { url, new_tab } => f
                .debug_struct("Navigate")
                .field("url", url)
                .field("new_tab", new_tab)
                .finish(),
            ActSpec::Evaluate { script } => {
                f.debug_struct("Evaluate").field("script", script).finish()
            }
            ActSpec::GetConsoleLogs => f.debug_struct("GetConsoleLogs").finish(),
            ActSpec::GetPageErrors => f.debug_struct("GetPageErrors").finish(),
            ActSpec::GetNetworkLog { include_bodies } => {
                f.debug_struct("GetNetworkLog")
                    .field("include_bodies", include_bodies)
                    .finish()
            }
        }
    }
}

/// 滚动目标：视口整体，或某个具体元素的可滚动容器。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ScrollTarget {
    /// 滚动视口本身。
    Viewport,
    /// 滚动某元素（或其最近可滚动祖先）。
    Element { r#ref: String },
}

/// 滚动方向。
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDir {
    Up,
    Down,
    Left,
    Right,
}

/// `WaitFor` 的等待条件。`#[serde(tag="kind")]`：`{"kind":"url_contains","text":"..."}`。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitCondition {
    /// 等到当前 URL 含某子串。
    UrlContains { text: String },
    /// 等到某文本在页面上可见。
    TextVisible { text: String },
    /// 等到某 ref 元素变为可操作（actionable：可见/未禁用/未被遮挡）。
    RefActionable { r#ref: String },
}

/// 键入内容。`Secret` 变体的值绝不进 LLM/日志——手写 Debug 脱敏（见模块级安全红线）。
///
/// Serialize 仍透出原值（写回密码字段需要真值上线）；脱敏只针对 `Debug`（`{:?}` 路径），
/// 这样误把整个 spec `dbg!`/`tracing` 出去也不泄露明文。
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum TypeInput {
    /// 明文文本（可进日志）。
    Literal(String),
    /// 敏感文本（密码/令牌等）：Debug 脱敏，绝不进 LLM/日志明文。
    Secret(String),
}

impl std::fmt::Debug for TypeInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeInput::Literal(s) => f.debug_tuple("Literal").field(s).finish(),
            TypeInput::Secret(_) => f.write_str("Secret(<redacted>)"),
        }
    }
}

/// 一次动作对页面产生的「效果」：是否真改变了页面，以及前后锚点（供前后对比 / 回放诊断）。
/// 锚点是不透明 JSON（由执行层填，如 URL/标题/某节点指纹）；类型层只承载形状。
#[derive(Clone, Debug)]
pub struct Effect {
    /// 这次动作是否让页面发生了可观测变化（导航 / DOM 变更 / 焦点移动等）。
    pub changed: bool,
    /// 动作前的锚点快照（不透明，执行层定义）。
    pub before_anchor: Option<serde_json::Value>,
    /// 动作后的锚点快照（不透明，执行层定义）。
    pub after_anchor: Option<serde_json::Value>,
}

/// 一次 `act` 的产物：人读的 `message`（回给 LLM 的动作小结）+ [`Effect`]（机读的页面变化）+
/// `success`（动作语义上是否成功——区别于「调用没出错但没达成目的」）。
#[derive(Clone, Debug)]
pub struct ActResult {
    /// 人读的动作小结（回给 LLM）。
    pub message: String,
    /// 机读的页面变化效果。
    pub effect: Effect,
    /// 动作语义上是否成功。
    pub success: bool,
}

// ═══════════════════════════════════════════════════════════════════════════
// C1：click / type / set_value 三分支 + 公共骨架（seq / frame 定位 / abort 作用域 /
// retry 包裹 / group 释放 / 基础 verify）。串 B2-B6 全部原语，零新增注入 JS（fill /
// checkElementStates / hit-target 都是 vendored 已有，经 injected.rs 的薄封装调用）。
// ═══════════════════════════════════════════════════════════════════════════

use crate::actionability::ObjectHandle;
use crate::aria_ref::RefRecord;
use crate::backend::cdp::{map_inject_err, CdpBackend};
use crate::injected::InjectError;

/// **[纯逻辑] vendored `fill(node,value)` 的三态返回 → C1 的兜底决策**（DESIGN §11 不变量⑰）。
///
/// `fill`（injectedScript.ts:824）返 `'done'` | `'needsinput'` | `'error:notconnected'`。本枚举是 Rust
/// 侧对其的三态翻译，type/set_value 据此决定下一步（tier1 fill 之后走哪条兜底）：
/// - [`FillOutcome::Done`] —— set-value 类控件已直接设值 + 派发事件，**无需再 insertText**（成功）。
/// - [`FillOutcome::NeedsInput`] —— text/textarea/contenteditable：`fill` 已 focus + 全选，值要靠
///   **`Input.insertText` 真键入**（type 的 tier2；IME/复杂控件兜底）。
/// - [`FillOutcome::NotConnected`] —— 元素 detach（`'error:notconnected'`），→ 可重试/重拍。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FillOutcome {
    /// `'done'`：已完成（set-value 控件直接设值），无需键入。
    Done,
    /// `'needsinput'`：已 focus+全选，值靠调用方 insertText 键入。
    NeedsInput,
    /// `'error:notconnected'`：元素 detach。
    NotConnected,
}

/// **[纯逻辑] 解析 `fill` 的 by-value 返回 `value`** 成 [`FillOutcome`]（不进浏览器，便于单测）。
/// 喂入 `result.value`（`fill_element` 回包的 `.value`，是三态字符串）：
/// - `"done"` → [`FillOutcome::Done`]；
/// - `"needsinput"` → [`FillOutcome::NeedsInput`]；
/// - `"error:notconnected"` → [`FillOutcome::NotConnected`]；
/// - 任何其它形状（不该发生：fill 契约只产上述三态）→ 保守当 `NotConnected`（宁可让上层重新
///   observe / 重拍，也不静默放行一个形状陌生的结果当成功）。
pub fn parse_fill_outcome(value: Option<&serde_json::Value>) -> FillOutcome {
    match value.and_then(|v| v.as_str()) {
        Some("done") => FillOutcome::Done,
        Some("needsinput") => FillOutcome::NeedsInput,
        Some("error:notconnected") => FillOutcome::NotConnected,
        _ => FillOutcome::NotConnected,
    }
}

/// **[纯逻辑] 取 [`TypeInput`] 的实际待键入文本**（C1：secret vault 解析在 E1，C1 走原值 insertText
/// 路径——值经 `Input.insertText` 注入**不过 LLM**，与 Literal 同路径，区别只在 Debug 脱敏）。
/// 抽纯函数便于单测「Secret 与 Literal 都返其内含字符串」。
pub fn type_input_text(input: &TypeInput) -> &str {
    match input {
        TypeInput::Literal(s) => s,
        // C1：secret 经 insertText 注入（不过 LLM，值不进日志）；E1 接 vault `secret:NAME` 解析。
        TypeInput::Secret(s) => s,
    }
}

/// **[纯逻辑] F2：把 click 的 URL + 元素态合成一个 verify 锚点 JSON**（不进浏览器，便于单测）。
/// 两端都缺（URL+元素态都读不到）→ `None`（Effect 该端为 None）；否则返一个对象，按需带 `url` 键
/// 与元素态键（直接并入 [`Self::act_read_click_anchor`] 返回对象的 checked/value/aria*/text）。
/// 这让 [`Effect`] 的 before/after 锚点既反映导航（url）又反映就地态变（checkbox 等），且 `changed`
/// 由两端不等判定。**non-secret only**——click 不处理 secret，调用方对 secret 动作另抑制锚点。
pub fn compose_click_anchor(
    url: Option<&str>,
    element: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    if let Some(u) = url {
        map.insert("url".to_string(), serde_json::Value::String(u.to_string()));
    }
    if let Some(serde_json::Value::Object(el)) = element {
        for (k, v) in el {
            map.insert(k.clone(), v.clone());
        }
    }
    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// C2 纯逻辑 helper（[纯逻辑]，不进浏览器，便于单测）：press_key 不可逆判定 + select_options 返回解析
// + scroll alignment 序列。
// ═══════════════════════════════════════════════════════════════════════════

/// **[纯逻辑] press_key 的不可逆（IRREVERSIBLE）检测**（C2，DESIGN §22 设计裁决⑧）。
///
/// 判据：**`keys` 解析为 `Enter`（无额外修饰键）且当前焦点落在 `<form>` 内** → `true`（IRREVERSIBLE）。
/// 在表单字段里按 Enter 通常触发隐式提交（submit），是不可逆副作用（发请求/扣款/发消息），须升级
/// 为不可逆——镜像分类器把「form submit / Enter 落 form」判 `Irreversible`（裁决⑧）。其它键
/// （含带修饰键的 `Ctrl+Enter` 等组合）/ 焦点不在 form 内 → `false`（普通 Exec 级）。
///
/// **C2 范围只做检测**：本函数 + 单测落地；**不接 enforcement**（强制门 / 审批闸在 E2/F1 接线，届时
/// facade 据本判定升级 [`crate::actions`] 的 IRREVERSIBLE 路径）。`focus_in_form` 是运行时信息（press_key
/// 执行前注入 `document.activeElement.closest('form')!=null` 取），由调用方传入。
///
/// `keys` 经 [`crate::input::parse_key_combo`] 规范化判定：仅当 `key=="Enter"` 且 `modifiers==0` 才算
/// 「裸 Enter」。解析失败（畸形/未知键）→ `false`（无法判定不可逆，保守不升级——非阻塞，真执行会另报错）。
pub fn press_key_is_irreversible(keys: &str, focus_in_form: bool) -> bool {
    if !focus_in_form {
        return false;
    }
    match crate::input::parse_key_combo(keys) {
        // 裸 Enter（无修饰键）落在 form 内 → 隐式提交风险 → IRREVERSIBLE。
        Ok(chord) => chord.key == "Enter" && chord.modifiers == 0,
        // 解析不了：保守不升级（真执行会另报 key combo 错）。
        Err(_) => false,
    }
}

/// **scroll element-target 的 4 种 alignment**（C2，DESIGN §11 设计裁决⑮「4 alignment 逃 sticky」）。
/// 把目标滚进视口时，sticky header/footer 可能仍遮挡——依次尝试这些 `block` alignment，取首个让目标
/// 「在视口内」的；全不行也不报错（良性态，已尽力滚到最近）。顺序：center（最常见居中可见）优先，
/// 然后 start/end/nearest。
pub const SCROLL_ALIGNMENTS: [&str; 4] = ["center", "start", "end", "nearest"];

/// select_options 注入返回（[`crate::injected::InjectionManager::select_options`] 的 by-value `value`）
/// 解析结果（[纯逻辑]，喂构造 Value 单测）。vendored `selectOptions` 返 `string[]`（已选 value 数组）
/// 或错误字符串（DESIGN §9，injectedScript.ts:777）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectOutcome {
    /// 成功：已选中项的 value 列表（多选含全部命中项）。
    Selected(Vec<String>),
    /// `'error:notconnected'`：元素 detach（→ 可重试/重拍）。
    NotConnected,
    /// `'error:optionsnotfound'`：给的某些选项找不到（→ 良性失败 success=false）。
    OptionsNotFound,
    /// `'error:optionnotenabled'`：命中的 option 被 disabled（→ 良性失败 success=false）。
    OptionNotEnabled,
    /// 形状陌生（不该发生：注入契约只产上述）→ 保守失败。
    Unknown,
}

/// **[纯逻辑] 解析 `selectOptions` 的 by-value 返回 `value`** 成 [`SelectOutcome`]（不进浏览器）。
/// 喂入 `result.value`：数组 → `Selected(values)`；三态错误字符串各自归类；其它 → `Unknown`。
pub fn parse_select_outcome(value: Option<&serde_json::Value>) -> SelectOutcome {
    match value {
        Some(serde_json::Value::Array(arr)) => SelectOutcome::Selected(
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
        ),
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "error:notconnected" => SelectOutcome::NotConnected,
            "error:optionsnotfound" => SelectOutcome::OptionsNotFound,
            "error:optionnotenabled" => SelectOutcome::OptionNotEnabled,
            _ => SelectOutcome::Unknown,
        },
        _ => SelectOutcome::Unknown,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// C3 纯逻辑 helper（[纯逻辑]，不进浏览器，便于单测）：Wait ms 钳制 + search grep 匹配 +
// wait_for deadline 默认 + dropdown 解析。全只读零写。
// ═══════════════════════════════════════════════════════════════════════════

/// **Wait 的最大有界时长**（C3，DESIGN §22「action 各自 deadline，避免 agent 整轮挂死」）：
/// `Wait{ms}` 的 sleep 上限。防 LLM 给出离谱大值（`wait 999999999` 实为整轮挂死）。**10s**：
/// 与 action 级默认 deadline（DESIGN:285，10-15s）同量级且**留出余量**——固定等待是显式动作，10s
/// 已足够覆盖正常异步加载等待；更长应改用 `wait_for(condition)`（轮询，按需）而非死等。**刻意小于**
/// 调用方常给的 action deadline（如 facade 默认/30s），避免「钳到上限的 wait 恰好撞上同值 deadline →
/// 被判 Timeout」的退化（钳制后的 sleep 须能在 deadline 内跑完）。
pub const WAIT_MS_CAP: u64 = 10_000;

/// **[纯逻辑] 钳制 Wait 的毫秒数到 [`WAIT_MS_CAP`]**（不进浏览器）。返回 `(clamped_ms, was_capped)`：
/// `was_capped` 供文案如实告知模型「请求的等待被钳制到上限」。`0` 合法（立即返回）。
pub fn clamp_wait_ms(ms: u64) -> (u64, bool) {
    if ms > WAIT_MS_CAP {
        (WAIT_MS_CAP, true)
    } else {
        (ms, false)
    }
}

/// search_page 的一条命中：含命中行/片段 + 其在文本里的字符偏移（供模型定位）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// 命中所在的「行」（按 `\n` 切；已 trim 两端空白）。
    pub line: String,
    /// 命中在整段文本里的字符偏移（首个匹配位置，按字节，仅供诊断排序）。
    pub offset: usize,
}

/// **[纯逻辑] 在页面文本里 grep `query`（大小写不敏感子串）**（C3，不进浏览器，便于单测）。
///
/// 逐行扫描，命中（大小写不敏感子串）的行收进结果（trim + 去重相邻空行），最多 `cap` 条（0=不限）。
/// 空 query → 空结果（不报错，良性）。返回命中列表（按出现顺序）。**只在已脱敏文本上跑**（调用方
/// 先 redact，故命中片段不含明文 secret）。
pub fn grep_page_text(text: &str, query: &str, cap: usize) -> Vec<SearchHit> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let mut hits = Vec::new();
    let mut byte_offset = 0usize;
    for line in text.split('\n') {
        let line_start = byte_offset;
        // +1 复原 split 掉的 '\n'（末行无 '\n' 也无害——后续不再用 byte_offset）。
        byte_offset += line.len() + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.to_lowercase().contains(&needle) {
            hits.push(SearchHit {
                line: trimmed.to_string(),
                offset: line_start,
            });
            if cap > 0 && hits.len() >= cap {
                break;
            }
        }
    }
    hits
}

/// **WaitFor 的默认 deadline**（C3，DESIGN §22「wait_for 各自 deadline」）：单次 `WaitFor` 轮询的
/// 总预算。超过 → [`BrowserError::Timeout`]`{phase:Action}`。比 action 默认略宽（条件可能要等异步加载/
/// SPA 软导航），但远小于 nav 的 30s（避免整轮挂死）。facade 的 `timeout_ms` 可覆盖（F1）。
pub const WAIT_FOR_DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// **WaitFor 轮询的退避间隔**（C3）：每次条件检查之间 sleep 这么久（小间隔，兼顾响应性与不忙等）。
pub const WAIT_FOR_POLL_INTERVAL: Duration = Duration::from_millis(150);

/// get_dropdown_options 注入返回（[`crate::injected::InjectionManager::dropdown_options`] 的 by-value
/// `value`）解析结果（[纯逻辑]，喂构造 Value 单测）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropdownOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
    pub disabled: bool,
}

/// get_dropdown_options 的三态解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropdownOutcome {
    /// 成功：option 列表（按文档顺序）。
    Options(Vec<DropdownOption>),
    /// `'error:notselect'`：元素不是 `<select>`（→ 良性失败 success=false，引导模型换 ref）。
    NotSelect,
    /// `'error:notconnected'`：元素 detach（→ 可重试/重拍）。
    NotConnected,
    /// 形状陌生（不该发生）→ 保守失败。
    Unknown,
}

/// **[纯逻辑] 解析 `dropdown_options` 的 by-value 返回**成 [`DropdownOutcome`]（不进浏览器）。
pub fn parse_dropdown_outcome(value: Option<&serde_json::Value>) -> DropdownOutcome {
    match value {
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "error:notselect" => DropdownOutcome::NotSelect,
            "error:notconnected" => DropdownOutcome::NotConnected,
            _ => DropdownOutcome::Unknown,
        },
        Some(serde_json::Value::Object(map)) => {
            match map.get("options").and_then(|v| v.as_array()) {
                Some(arr) => {
                    let options = arr
                        .iter()
                        .map(|o| DropdownOption {
                            value: o
                                .get("value")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            label: o
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            selected: o
                                .get("selected")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            disabled: o
                                .get("disabled")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        })
                        .collect();
                    DropdownOutcome::Options(options)
                }
                None => DropdownOutcome::Unknown,
            }
        }
        _ => DropdownOutcome::Unknown,
    }
}

/// find_elements 注入返回的一条命中（[`crate::injected::InjectionManager::find_elements`] 的
/// `matches[]` 元素）。[纯逻辑] 解析用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundElement {
    pub r#ref: String,
    pub role: String,
    pub name: String,
}

/// find_elements 的解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindOutcome {
    /// 成功：命中元素列表（已登记 ref）+ 命中总数（可能 > matches.len() 若 cap 截断）。
    Found {
        matches: Vec<FoundElement>,
        total: usize,
    },
    /// `'error:notobserved'`：该帧还没 observe 过（`_lastAriaSnapshotForQuery` 未物化）→ 引导先 observe。
    NotObserved,
    /// 形状陌生（不该发生）→ 保守失败。
    Unknown,
}

/// **[纯逻辑] 解析 `find_elements` 的 by-value 返回**成 [`FindOutcome`]（不进浏览器）。
pub fn parse_find_outcome(value: Option<&serde_json::Value>) -> FindOutcome {
    match value {
        Some(serde_json::Value::String(s)) if s == "error:notobserved" => FindOutcome::NotObserved,
        Some(serde_json::Value::Object(map)) => {
            let matches = map
                .get("matches")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            let r = m.get("ref").and_then(|v| v.as_str())?;
                            Some(FoundElement {
                                r#ref: r.to_string(),
                                role: m
                                    .get("role")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                name: m
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            })
                        })
                        .collect::<Vec<_>>()
                });
            match matches {
                Some(matches) => {
                    let total = map
                        .get("total")
                        .and_then(|v| v.as_u64())
                        .map(|t| t as usize)
                        .unwrap_or(matches.len());
                    FindOutcome::Found { matches, total }
                }
                None => FindOutcome::Unknown,
            }
        }
        _ => FindOutcome::Unknown,
    }
}

/// **[纯逻辑] 把注入侧 [`InjectError`] 分类成动作层 [`RetryDecision`]**（C1 op 内统一收口）。
/// - `ContextNotReady` —— utility world 还没物化（导航中）：瞬态，[`RetryDecision::Retryable`]。
/// - `JsException`（命中 [`crate::actionability::is_non_editable_error`]）—— 元素类型根本不支持编辑/
///   填充（NonRecoverable）：[`RetryDecision::Fatal`]（`Blocked`，禁重试，B3 语义）。
/// - 其它 `JsException` / `Protocol` / `Transport` —— 经 [`map_inject_err`] 上抛为 `Fatal`（非瞬态缺态，
///   不靠动作层短重试自愈；真瞬态缺态走 `check_states` 的 `Missing` 分支，不到这里）。
fn classify_inject_err(e: InjectError) -> RetryDecision {
    match e {
        InjectError::ContextNotReady { .. } => RetryDecision::Retryable(map_inject_err(
            InjectError::ContextNotReady {
                frame_id: String::new(),
            },
        )),
        InjectError::JsException(msg) => {
            if crate::actionability::is_non_editable_error(&msg) {
                RetryDecision::Fatal(BrowserError::Blocked { reason: msg })
            } else {
                RetryDecision::Fatal(map_inject_err(InjectError::JsException(msg)))
            }
        }
        other => RetryDecision::Fatal(map_inject_err(other)),
    }
}

/// **[纯逻辑] `check_states` 结果 → 是否放行 / 重试**（C1 op 内统一收口；B6 语义）。
/// - `Pass` → `Ok(())`（放行执行动作）。
/// - `Missing(state)` → [`RetryDecision::Retryable`]（瞬态缺态 visible/stable/enabled/暂只读，等一拍重判）。
/// - `NotConnected` → [`RetryDecision::Retryable`]（元素 detach 漂移，重试触发 resolve 重定位/重拍）。
fn gate_check_result(r: crate::actionability::CheckResult) -> Result<(), RetryDecision> {
    use crate::actionability::CheckResult;
    match r {
        CheckResult::Pass => Ok(()),
        CheckResult::Missing(state) => Err(RetryDecision::Retryable(BrowserError::Other(format!(
            "element not actionable yet: missing state '{state}'"
        )))),
        CheckResult::NotConnected => Err(RetryDecision::Retryable(BrowserError::NotConnected)),
    }
}

/// **[纯逻辑] 几何取点错误的 LLM 文案**（NotVisible / NotInViewport 不同措辞，供模型路由）。
fn geom_err_reason(e: crate::input::GeomError) -> &'static str {
    use crate::input::GeomError;
    match e {
        GeomError::NotVisible => "element has no visible layout box (not visible)",
        GeomError::NotInViewport => "element center is outside the viewport (scroll needed)",
    }
}

/// **[纯逻辑] 几何取点错误 → 重试决策**（sticky 逃逸已在 op 内尝试后的兜底）：NotVisible /
/// NotInViewport 都按瞬态 [`RetryDecision::Retryable`]（退避后布局/懒加载可能就绪；耗尽则上抛该
/// 瞬态错误，文案引导模型 scroll / 重 observe）。**sticky 逃逸**（[`CdpBackend::scroll_escape_sticky`]）
/// 由调用点在取点失败时**先于**本兜底尝试，见 act_click/act_hover。
fn gate_geom_err(e: crate::input::GeomError) -> RetryDecision {
    RetryDecision::Retryable(BrowserError::Other(geom_err_reason(e).to_string()))
}

/// 把 [`BrowserError`]（来自原语方法）按动作层语义分类成 [`RetryDecision`]（C1 op 内统一收口）。
/// 这是「原语返 BrowserError」→「retry 编排吃 RetryDecision」的桥：
/// - `NotConnected` —— 漂移：可重试（resolve 重定位 / 重拍）。
/// - `NodeStale` —— 代际层 stale：**Fatal**（动作层短重试无用，须上层重 observe；镜像反查链层①③语义）。
/// - `Blocked` —— 遮挡 / 不可编辑：按 spec **可重试一次让上层重判遮挡**（DESIGN：遮挡通常 Retryable）。
/// - `Timeout`/`Detached`/`TargetClosed`/… —— 生命周期/超时：**Fatal**（abort/deadline 已是终态，
///   run_act_with_retry 的 race 本就会优先以这些返回；到这里多为原语内 CDP 超时，按终态上抛）。
/// - 其它 —— Fatal（保守上抛，不盲目重试未知错误）。
fn classify_browser_err(e: BrowserError) -> RetryDecision {
    match e {
        BrowserError::NotConnected => RetryDecision::Retryable(BrowserError::NotConnected),
        BrowserError::Blocked { reason } => RetryDecision::Retryable(BrowserError::Blocked { reason }),
        other => RetryDecision::Fatal(other),
    }
}

/// **[纯逻辑] editable 检查（type/set_value 的 `check_states([...editable])`）专用分类器**
/// （DESIGN §11 不变量④ + IDMM「Decision 失败禁 Retry」）。
///
/// **为何不复用 [`classify_browser_err`]**：occlusion-Blocked（hit-target 路径，遮挡可能瞬态散去 →
/// **可重试**）与 non-editable-Blocked（editable 检查路径，元素类型根本不支持编辑 → **NonRecoverable
/// 禁重试**）共用 [`BrowserError::Blocked`] 变体，[`classify_browser_err`] 把**所有** `Blocked` 一律归
/// `Retryable`（注释「遮挡通常 Retryable」）。但 `check_states([...editable])` 返的 `Blocked`（来自
/// [`crate::actionability::is_non_editable_error`] 命中，actionability.rs:396）是**不可编辑这一终态**——
/// 走 [`classify_browser_err`] 会被错走完 6 槽退避（770ms）才上抛，违反不变量④（NonRecoverable 立返）。
///
/// 故 editable 检查路径在 [`act_type`]/[`act_set_value`] 用本分类器（**不**用 `classify_browser_err`）：
/// - [`BrowserError::Blocked`] —— 不可编辑特例 → [`RetryDecision::Fatal`]（禁重试，立返）。
/// - 其它（`NotConnected` 漂移 / `NodeStale` / 终态…）—— 委托 [`classify_browser_err`]，语义不变
///   （readOnly **不**到这里：B3 已把它分流成 `Ok(Missing("editable"))`，由 [`gate_check_result`] 判
///   `Retryable`，故 readOnly 可重试路径不受影响）。
fn classify_editable_check_err(e: BrowserError) -> RetryDecision {
    match e {
        // 不可编辑特例（元素类型根本不支持编辑）：NonRecoverable → Fatal（禁重试，立返）。
        // 区别于 click 的遮挡 Blocked（那走 classify_browser_err → Retryable）。
        BrowserError::Blocked { reason } => RetryDecision::Fatal(BrowserError::Blocked { reason }),
        // 其它错误语义与通用分类器一致（NotConnected 可重试、终态 Fatal）。
        other => classify_browser_err(other),
    }
}

impl CdpBackend {
    /// **C1 act 主循环**：把 LLM 动作（Click/Type/SetValue）串 B2-B6 原语执行。其它 ActSpec 变体
    /// 仍返 [`BrowserError::Unsupported`]（C2/C3/D/E/F 后续任务）。
    ///
    /// 公共骨架（三动作共用，DESIGN §7/§11/§22）：
    /// 1. **seq**：[`Self::next_act_seq`] 取唯一 seq → objectGroup `act-<seq>`。
    /// 2. **frame 定位**：[`Self::resolve_ref_record`]（层①，纯 Rust）拿 [`RefRecord`]；ref 不在当前
    ///    代际表 → [`BrowserError::NodeStale`]（文案引导 re-observe，**不进浏览器**）。
    /// 3. **abort 作用域**：[`Self::arm_act_abort`] 据 record.frame_id 派生子 [`Progress`] + 装 detach/
    ///    crash 监听 guard；整个动作跑在子 Progress 上（page.close/frame.detach → 立即 abort）。
    /// 4. **retry 包裹**：[`run_act_with_retry`]（irreversible=false，C1 分类器在 E2/F1）。op 内**每次
    ///    重解析 ref**（外层自愈：resolve 不到 → 触发代际/重拍；check Missing → 退避重判）。
    /// 5. **group 释放**：finally 式——无论成功/失败，末尾 [`Self::release_act_group_by_ref`] 一次，
    ///    释放本动作 objectGroup 的全部句柄（无泄漏）。
    pub async fn act_impl(
        &self,
        spec: &ActSpec,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        match spec {
            ActSpec::Click { r#ref } => self.act_click(r#ref, parent).await,
            ActSpec::Type { r#ref, text } => self.act_type(r#ref, text, parent).await,
            ActSpec::SetValue {
                r#ref,
                value,
                secret,
            } => self.act_set_value(r#ref, value, *secret, parent).await,
            // C2：hover / select_option / press_key / scroll / scroll_to_text。
            ActSpec::Hover { r#ref } => self.act_hover(r#ref, parent).await,
            ActSpec::SelectOption { r#ref, options } => {
                self.act_select_option(r#ref, options, parent).await
            }
            ActSpec::PressKey { keys } => self.act_press_key(keys, parent).await,
            ActSpec::Scroll {
                target,
                direction,
                amount,
            } => self.act_scroll(target, *direction, *amount, parent).await,
            ActSpec::ScrollToText { text } => self.act_scroll_to_text(text, parent).await,
            // C3：只读类（Info 级，零写）。get_page_text/search_page 经 redact+wrap；find_elements 登记当代际 ref。
            ActSpec::GetPageText => self.act_get_page_text(parent).await,
            ActSpec::SearchPage { query } => self.act_search_page(query, parent).await,
            ActSpec::FindElements { selector } => self.act_find_elements(selector, parent).await,
            ActSpec::GetDropdownOptions { r#ref } => self.act_get_dropdown_options(r#ref, parent).await,
            ActSpec::Cursor => self.act_cursor(parent).await,
            ActSpec::Wait { ms } => self.act_wait(*ms, parent).await,
            ActSpec::WaitFor { condition } => self.act_wait_for(condition, parent).await,
            // D3：tab 发现 + switch/close/open/tabs（active_target 逻辑指针；新 tab 不抢焦点）。
            ActSpec::Tabs => self.act_tabs().await,
            ActSpec::SwitchTab { tab_id } => self.act_switch_tab(tab_id).await,
            ActSpec::CloseTab { tab_id } => self.close_tab_impl(tab_id, parent).await,
            ActSpec::OpenLinkNewTab { url } => self.act_open_link_new_tab(url).await,
            // D4：history 导航（back/forward 边界钳制）+ reload（POST 页→IRREVERSIBLE 检测）+
            // switch_frame（active_frame 逻辑指针）。settle 复用 D2 run_settle；良性边界态不报错。
            ActSpec::Back => self.act_history_nav(crate::nav::HistoryNav::Back).await,
            ActSpec::Forward => self.act_history_nav(crate::nav::HistoryNav::Forward).await,
            ActSpec::Reload => self.act_reload().await,
            ActSpec::SwitchFrame { r#ref } => self.act_switch_frame(r#ref).await,
            // E3：evaluate 门控（默认 OFF / opt-in 全权 / 与持久登录互斥 / yolo 不豁免不看 session_mode）。
            ActSpec::Evaluate { script } => self.act_evaluate(script).await,
            // F-actions：补全动作空间 upload_file / download / save_as_pdf / extract（P2 DoD「完整动作空间+extract」）。
            // - upload_file：DOM.setFileInputFiles（绕系统文件对话框的唯一正道，对标 Playwright）。
            // - download：注入 `<a download>` + click 触发，复用 E4 沙箱（denylist 红线 / MOTW / 落隔离目录）。
            // - save_as_pdf：Page.printToPDF → 写隔离 downloads 目录（headful 已实测可用，见 act_save_as_pdf）。
            // - extract：deterministic plumbing（aria snapshot + 可见文本，redact+wrap）；LLM-driven 抽取留 P3。
            ActSpec::UploadFile { r#ref, paths } => self.act_upload_file(r#ref, paths, parent).await,
            ActSpec::Download { url } => self.act_download(url, parent).await,
            ActSpec::SaveAsPdf => self.act_save_as_pdf(parent).await,
            ActSpec::Extract { schema } => self.act_extract(schema, parent).await,
            // 调试捕获读取动作（只读，读 per-tab 缓冲并脱敏序列化）。
            ActSpec::GetConsoleLogs => self.act_get_console_logs().await,
            ActSpec::GetPageErrors => self.act_get_page_errors().await,
            ActSpec::GetNetworkLog { include_bodies } => self.act_get_network_log(*include_bodies).await,
            // 其它动作保持 Unsupported（D3 之外的 tab 路由细节已接；剩余无）。
            #[allow(unreachable_patterns)]
            _ => Err(BrowserError::Unsupported {
                capability: "act".into(),
                hint: "this action is not implemented yet".into(),
            }),
        }
    }

    /// 公共骨架包装：定位 frame（层①）→ arm abort 子作用域 → arm group 释放 RAII guard →
    /// run_act_with_retry。group 释放是**类型保证的 finally**（[`crate::actionability::ActGroupReleaseGuard`]
    /// Drop，覆盖正常返回 / `?` 早返 / await 点 panic），非手动约定。
    /// `op` 收 (seq, frame_id) 每次 attempt 重跑（内部自行重解析 ref）；返 `Result<ActResult, RetryDecision>`。
    async fn act_with_skeleton<F, Fut>(
        &self,
        llm_ref: &str,
        parent: &Progress,
        op: F,
    ) -> Result<ActResult, BrowserError>
    where
        F: Fn(u64, RefRecord) -> Fut,
        Fut: Future<Output = Result<ActResult, RetryDecision>>,
    {
        // 1) seq → objectGroup act-<seq>。
        let seq = self.next_act_seq();
        // 2) frame 定位（层①，纯 Rust）：拿 RefRecord（含 frame_id）。ref 不在当前代际 → NodeStale。
        let rec = self.resolve_ref_record(llm_ref).await?;
        // 3) abort 作用域：据 record.frame_id 派生子 Progress + detach/crash 监听 guard。
        //    D1：arm_act_abort 现 async（内部取 active tab 的 page session 锚定 detach 订阅）。
        let (child, _abort_guard) = self.arm_act_abort(parent, &rec.frame_id).await?;
        // 4) group 释放 RAII guard：arm 一个 drop-guard（类型保证 finally）。无论下面 run_act_with_retry
        //    正常返回 / `?` 早返 / await 点 panic，guard 离开作用域即释放本动作 objectGroup（act-<seq>）的
        //    全部句柄（无泄漏、只释放一次）。**必须**在 run_act_with_retry 之前 arm（覆盖整个动作）。
        let _release_guard = self.arm_act_group_release(&rec, seq).await;
        // 5) retry 包裹（irreversible=false，C1）：op 每次 attempt 重解析 ref（外层自愈）。
        let rec_for_op = rec.clone();
        let result = run_act_with_retry(&child, false, move |_attempt| op(seq, rec_for_op.clone())).await;
        // 6) finally：_release_guard 在此（函数返回）Drop → 释放本动作 objectGroup（成功/失败/早返/panic
        //    均然）；_abort_guard 同时 Drop 收摊 detach/crash 监听。无显式手动释放（RAII 唯一释放点）。
        result
    }

    /// **Click 分支**（C1，DESIGN §9/§11，三步舞 hit-target）：
    /// resolve_ref_to_object（层②③）→ check_states(visible/stable/enabled) → 几何取点
    /// （getContentQuads + viewport + pick_click_point）→ hit_setup（by_value=false）→ dispatch_click
    /// → hit_stop（'done'→OK / 遮挡→Blocked）→ verify（捕按钮 DOM 锚 + URL）。
    ///
    /// **三级兜底**（DESIGN §11）：C1 实现默认 `actionable` 主路径（全检查 + hit-target）。force（绕
    /// check_states + hit-target 直接 dispatch）/ trial（只检查不实点）入口随 ActSpec mode 扩展时接
    /// （当前 `ActSpec::Click` 无 mode 字段，故 C1 走 actionable；见模块 TODO）。
    async fn act_click(&self, llm_ref: &str, parent: &Progress) -> Result<ActResult, BrowserError> {
        let llm_ref_owned = llm_ref.to_string();
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            async move {
                // 层②③：ref → 活元素句柄（每次 attempt 重解析——外层自愈）。
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;

                // actionability 四检查（visible/stable/enabled；click 不检 editable）。
                let cr = this
                    .check_states(&handle, &["visible", "stable", "enabled"])
                    .await
                    .map_err(classify_browser_err)?;
                gate_check_result(cr)?;

                // verify 前锚（F2 富锚点）：URL（导航类副作用证据）+ 目标元素 DOM 态
                // （checked/value/aria-pressed/text——checkbox/radio/toggle 即便不导航也能判 changed）。
                let before_url = this.act_current_url().await;
                let before_el = this.act_read_click_anchor(&handle).await;

                // 几何取点（CSS 像素，禁 DPR）。取点失败(NotVisible/NotInViewport)→先 4-alignment 逃
                // sticky 顶/底栏遮挡再重取点(C2);逃不出再退瞬态重试(退避内布局可能就绪)。
                let point = this
                    .pick_point_escaping_sticky(&rec, &handle)
                    .await?;

                // 三步舞：setup（by_value=false 保活）→ dispatch_click → stop。
                let interceptor = this
                    .hit_setup(&handle, "mouse", point, false)
                    .await
                    .map_err(classify_browser_err)?;
                this.click_at(point).await.map_err(classify_browser_err)?;
                this.hit_stop(&interceptor)
                    .await
                    .map_err(classify_browser_err)?;

                // verify 后锚（F2 富锚点）：URL + 目标元素态。changed = URL 变 OR 元素态变
                // （checkbox 勾选/toggle 文案/aria-pressed 翻转都被 element 锚点捕获，导航被 URL 捕获）。
                // 元素可能因点击 detach（导航/重渲染）→ after_el 读不到 → None，此时靠 URL 判 changed。
                let after_url = this.act_current_url().await;
                let after_el = this.act_read_click_anchor(&handle).await;
                let changed = before_url != after_url || before_el != after_el;
                let before_anchor = compose_click_anchor(before_url.as_deref(), before_el.as_ref());
                let after_anchor = compose_click_anchor(after_url.as_deref(), after_el.as_ref());
                Ok(ActResult {
                    message: format!("clicked {llm_ref}; re-observe to see the updated page"),
                    effect: Effect {
                        changed,
                        before_anchor,
                        after_anchor,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **Type 分支**（C1，DESIGN §9/§11，fill 三级兜底）：
    /// resolve_ref_to_object → check_states(visible/stable/enabled/editable)（editable error→Fatal 禁
    /// 重试；readOnly→Missing 可重试，B3 已区分）→ fill 三级兜底：
    /// - tier1：注入 `fill`（'done'→直接成功 / 'needsinput'→tier2 / 'error:notconnected'→Retryable）；
    /// - tier2（'needsinput'）：[`Self::type_text`]（`Input.insertText`，IME/复杂控件兜底）；
    /// - tier3（仍失败）：逐键 [`Self::key_combo`] per char（最后逃生）。
    ///
    /// `TypeInput::Secret`：C1 走原值 insertText（不过 LLM，值不进日志；vault `secret:NAME` 解析 E1 接）。
    /// verify：读回 input.value → Effect{changed, before/after}。
    async fn act_type(
        &self,
        llm_ref: &str,
        text: &TypeInput,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let llm_ref_owned = llm_ref.to_string();
        let to_type = type_input_text(text).to_string();
        let is_secret = matches!(text, TypeInput::Secret(_));
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            let to_type = to_type.clone();
            async move {
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;

                // editable 在内：不可编辑特例 → check_states 返 Err(Blocked) → Fatal 禁重试（B3 语义）。
                // 用 classify_editable_check_err（**非** classify_browser_err）：editable-check 的 Blocked =
                // 不可编辑终态 = Fatal；readOnly 已被 B3 分流为 Ok(Missing) 不到这里（可重试路径不受影响）。
                let cr = this
                    .check_states(&handle, &["visible", "stable", "enabled", "editable"])
                    .await
                    .map_err(classify_editable_check_err)?;
                gate_check_result(cr)?;

                let before_value = this.act_read_element_value(&handle).await;

                // tier1：fill（focus + 全选 / set-value 类直接设值）。
                let outcome = this.act_fill(&rec, &handle, &to_type).await?;
                match outcome {
                    FillOutcome::Done => {
                        // set-value 类已设值 + 派发事件，无需键入。
                    }
                    FillOutcome::NotConnected => {
                        return Err(RetryDecision::Retryable(BrowserError::NotConnected));
                    }
                    FillOutcome::NeedsInput => {
                        // tier2：insertText（fill 已 focus+全选，这里键入覆盖选区）。
                        if let Err(e) = this.type_text(&to_type).await {
                            // tier3：insertText 失败 → 逐字符 keyDown/keyUp 逃生（最后兜底）。
                            this.act_type_per_char(&to_type)
                                .await
                                .map_err(classify_browser_err)?;
                            // tier3 也失败会经 ? 上抛；走到这里说明 tier3 成功，e 仅诊断。
                            let _ = e;
                        }
                    }
                }

                // verify：读回 value。secret 不把值写进 message（only 长度），明文可写长度小结。
                let after_value = this.act_read_element_value(&handle).await;
                let changed = before_value != after_value;
                let message = if is_secret {
                    format!("typed {} secret chars into {llm_ref}; re-observe to confirm", to_type.chars().count())
                } else {
                    format!("typed into {llm_ref}; re-observe to confirm")
                };
                // secret：锚点不含值（before/after 都置 None，只记 changed 布尔，绝不泄漏 secret value）。
                let (before_anchor, after_anchor) = if is_secret {
                    (None, None)
                } else {
                    (
                        before_value.map(serde_json::Value::String),
                        after_value.map(serde_json::Value::String),
                    )
                };
                Ok(ActResult {
                    message,
                    effect: Effect {
                        changed,
                        before_anchor,
                        after_anchor,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **SetValue 分支**（C1，DESIGN §9）：与 Type 区别——**直接设值**（注入 `fill`，set-value 类控件
    /// 直接 set value + 派发 input/change；text 类则 fill 'needsinput' 后一次性 insertText，不模拟逐键）。
    /// 适合大文本 / 受控组件快路径。check_states(editable) → fill → （needsinput 则 insertText）→ verify。
    ///
    /// **安全红线（`secret == true`，来自 facade `secret:NAME` 解析）**：镜像 [`Self::act_type`] 的
    /// `is_secret` 处理——read-back 的 input.value 是注入进去的**明文凭据**，绝不能进 verify 锚点
    /// （否则 F2 把 anchor 透进 ToolResult 就 live 泄漏）。故 secret 时 before/after 锚点都置 `None`，
    /// `message` 只记字符数不记值（与 Debug 脱敏 + Serialize-时-anchor-空 共同构成 set_value secret 不泄漏）。
    async fn act_set_value(
        &self,
        llm_ref: &str,
        value: &str,
        is_secret: bool,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let llm_ref_owned = llm_ref.to_string();
        let value_owned = value.to_string();
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            let value = value_owned.clone();
            async move {
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;
                // editable 在内：不可编辑特例 → Fatal 禁重试（同 act_type，用 editable 专用分类器）。
                let cr = this
                    .check_states(&handle, &["visible", "stable", "enabled", "editable"])
                    .await
                    .map_err(classify_editable_check_err)?;
                gate_check_result(cr)?;

                // secret：read-back 是明文凭据，绝不采锚点（before/after 都置 None）——只读 changed 用的
                // 前值时也跳过（不持有任何 read-back 明文）。非 secret 才读前值作锚点。
                let before_value = if is_secret {
                    None
                } else {
                    this.act_read_element_value(&handle).await
                };

                // 直接设值快路径：fill。set-value 类 → 'done'；text 类 → 'needsinput' 后一次 insertText
                // （不逐键模拟——set_value 语义就是「整体设值」）。
                match this.act_fill(&rec, &handle, &value).await? {
                    FillOutcome::Done => {}
                    FillOutcome::NotConnected => {
                        return Err(RetryDecision::Retryable(BrowserError::NotConnected));
                    }
                    FillOutcome::NeedsInput => {
                        this.type_text(&value).await.map_err(classify_browser_err)?;
                    }
                }

                if is_secret {
                    // 安全红线：不读 read-back 值（不持有明文）；changed 视作 true（已 fill 写入），
                    // message 只记字符数不记值；锚点全 None（不泄漏 set_value 的 secret 明文）。
                    Ok(ActResult {
                        message: format!(
                            "set {} secret chars into {llm_ref}; re-observe to confirm",
                            value.chars().count()
                        ),
                        effect: Effect {
                            changed: true,
                            before_anchor: None,
                            after_anchor: None,
                        },
                        success: true,
                    })
                } else {
                    let after_value = this.act_read_element_value(&handle).await;
                    let changed = before_value != after_value;
                    Ok(ActResult {
                        message: format!("set value of {llm_ref}; re-observe to confirm"),
                        effect: Effect {
                            changed,
                            before_anchor: before_value.map(serde_json::Value::String),
                            after_anchor: after_value.map(serde_json::Value::String),
                        },
                        success: true,
                    })
                }
            }
        })
        .await
    }

    // ═══════════════════════════════════════════════════════════════════════
    // C2：hover / select_option / press_key / scroll / scroll_to_text。
    // hover/select_option 复用 act_with_skeleton（seq/abort/group 释放）；press_key/scroll(viewport)/
    // scroll_to_text 是**页面级**动作（无 element ref），走简化骨架（abort+retry，无 ref 解析）。
    // 全程禁 DPR；良性态（滚到底/未命中）不报错，如实 success。
    // ═══════════════════════════════════════════════════════════════════════

    /// **Hover 分支**（C2，DESIGN §9）：resolve_ref_to_object → check_states(visible/stable)（hover
    /// 不需 enabled/editable）→ 几何取点（getContentQuads + viewport + pick_click_point，**禁 DPR**）→
    /// hit_setup(action="hover")（遮挡校验：hover 也要点到目标本身，否则 hover 的是遮挡层）→
    /// dispatch_mouse_move（B5）→ hit_stop。verify：hover 无稳定 DOM 锚（:hover 是瞬时样式），Effect
    /// 记 changed=true（已投递 mouseMoved）+ before/after 取 URL（hover 一般不改 URL，但保持锚点形态）。
    async fn act_hover(&self, llm_ref: &str, parent: &Progress) -> Result<ActResult, BrowserError> {
        let llm_ref_owned = llm_ref.to_string();
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            async move {
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;

                // hover 的 actionability：visible + stable（不需 enabled/editable——hover 不操作控件态）。
                let cr = this
                    .check_states(&handle, &["visible", "stable"])
                    .await
                    .map_err(classify_browser_err)?;
                gate_check_result(cr)?;

                // 几何取点（CSS 像素，禁 DPR）。取点失败→先 4-alignment 逃 sticky 再重取点(C2)。
                let point = this
                    .pick_point_escaping_sticky(&rec, &handle)
                    .await?;

                // 三步舞（action="hover"，裁决③）：setup 预检遮挡 → mouseMoved → stop 读命中。
                // 遮挡（hover 到了别的元素）→ Blocked（可重试让上层重判遮挡）。
                let interceptor = this
                    .hit_setup(&handle, "hover", point, false)
                    .await
                    .map_err(classify_browser_err)?;
                this.mouse_move_to(point).await.map_err(classify_browser_err)?;
                this.hit_stop(&interceptor)
                    .await
                    .map_err(classify_browser_err)?;

                Ok(ActResult {
                    message: format!("hovered {llm_ref}; re-observe to see any revealed content"),
                    effect: Effect {
                        // hover 已投递 mouseMoved（可能触发 :hover/onmouseover 揭示菜单等）→ 视作 changed。
                        changed: true,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **SelectOption 分支**（C2，DESIGN §9）：resolve_ref_to_object（须 `<select>`）→
    /// check_states(visible/enabled) → 注入 vendored `selectOptions(node, options)`
    /// （[`crate::injected::InjectionManager::select_options`]，按 value/label 匹配）→ verify 读回
    /// `select.value`/`selectedOptions`。
    ///
    /// 良性失败（optionsnotfound/optionnotenabled）→ success=false **不报错**（如实告诉模型某选项找不到/
    /// 被禁用，引导改选）。元素非 `<select>` → 注入抛 → check_states 之前的 select_options JsException →
    /// Fatal（禁重试）。detach → 可重试。
    async fn act_select_option(
        &self,
        llm_ref: &str,
        options: &[String],
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let llm_ref_owned = llm_ref.to_string();
        let options_owned = options.to_vec();
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            let options = options_owned.clone();
            async move {
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;

                // select 的 actionability：visible + enabled（不需 stable——下拉一般不动画；不需 editable）。
                let cr = this
                    .check_states(&handle, &["visible", "enabled"])
                    .await
                    .map_err(classify_browser_err)?;
                gate_check_result(cr)?;

                let before_value = this.act_read_element_value(&handle).await;

                // 注入 vendored selectOptions：按 value/label 匹配并派发 input/change。非 <select> → 抛 →
                // classify_inject_err → Fatal（禁重试）；detach → 'error:notconnected' → 可重试。
                let result = this
                    .manager_for_record(&rec)
                    .await
                    .ok_or(RetryDecision::Retryable(BrowserError::NotConnected))?
                    .select_options(&rec.frame_id, &handle.object_id, &options)
                    .await
                    .map_err(classify_inject_err)?;
                let outcome = parse_select_outcome(result.get("value"));

                let after_value = this.act_read_element_value(&handle).await;
                let changed = before_value != after_value;
                match outcome {
                    SelectOutcome::Selected(values) => Ok(ActResult {
                        message: format!(
                            "selected {} option(s) on {llm_ref}: {}; re-observe to confirm",
                            values.len(),
                            values.join(", ")
                        ),
                        effect: Effect {
                            changed,
                            before_anchor: before_value.map(serde_json::Value::String),
                            after_anchor: after_value.map(serde_json::Value::String),
                        },
                        success: true,
                    }),
                    // detach：可重试（重定位/重拍）。
                    SelectOutcome::NotConnected => {
                        Err(RetryDecision::Retryable(BrowserError::NotConnected))
                    }
                    // 良性失败：选项找不到 / 被禁用 → success=false 不报错（引导模型改选）。
                    SelectOutcome::OptionsNotFound => Ok(ActResult {
                        message: format!(
                            "no matching option for {options:?} on {llm_ref}; re-observe the dropdown's options"
                        ),
                        effect: Effect {
                            changed: false,
                            before_anchor: None,
                            after_anchor: None,
                        },
                        success: false,
                    }),
                    SelectOutcome::OptionNotEnabled => Ok(ActResult {
                        message: format!("the matched option on {llm_ref} is disabled; pick another"),
                        effect: Effect {
                            changed: false,
                            before_anchor: None,
                            after_anchor: None,
                        },
                        success: false,
                    }),
                    // 形状陌生：保守可重试（让上层重判/重拍，不静默当成功）。
                    SelectOutcome::Unknown => {
                        Err(RetryDecision::Retryable(BrowserError::Other(
                            "selectOptions returned an unexpected shape".into(),
                        )))
                    }
                }
            }
        })
        .await
    }

    /// **PressKey 分支**（C2，DESIGN §9，**页面级无 ref**）：直接对当前 page session 投递组合键
    /// （[`Self::key_combo`]，B5 US 布局解析）。无 element 反查，走 [`Self::act_page_with_skeleton`]
    /// 简化骨架（abort 作用域 + retry，**无** ref 解析 / group 释放）。
    ///
    /// **Enter-in-form IRREVERSIBLE 检测（裁决⑧）**：执行前注入查 `document.activeElement.closest('form')`，
    /// 若 [`press_key_is_irreversible`] 判 true（裸 Enter 落 form），**记一条 tracing::warn 标记**——
    /// **C2 只做检测，不接 enforcement**（强制门 / 审批闸在 E2/F1 接线）。
    ///
    /// **TODO(E2/F1)**：把 `press_key_is_irreversible` 的判定接到 facade/engine 的 IRREVERSIBLE 强制门
    /// （fail-closed 拒 / 带外确认），并把 `run_act_with_retry` 的 `irreversible` 参数据此置 true（禁重试）。
    /// C2 此处 retry `irreversible=false`（仅页面级键投递；真正的不可逆门在 E2/F1）。
    async fn act_press_key(&self, keys: &str, parent: &Progress) -> Result<ActResult, BrowserError> {
        // 检测（C2 只检测不强制）：焦点是否在 form 内（运行时取）→ Enter-in-form 判不可逆。
        let focus_in_form = self.act_focus_in_form().await;
        let irreversible = press_key_is_irreversible(keys, focus_in_form);
        if irreversible {
            tracing::warn!(
                target: "nomi_browser_engine::actions",
                keys = %keys,
                "press_key detected IRREVERSIBLE (Enter in form, implicit submit); \
                 TODO(E2/F1): wire fail-closed enforcement (C2 detection-only, not blocking)"
            );
        }

        let keys_owned = keys.to_string();
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            let keys = keys_owned.clone();
            async move {
                // 页面级键投递（B5）：解析失败（畸形/未知键）→ Other（Fatal，调用方传错非瞬态）。
                this.key_combo(&keys).await.map_err(classify_browser_err)?;
                Ok(ActResult {
                    message: format!("pressed key(s) \"{keys}\"; re-observe to see the result"),
                    effect: Effect {
                        changed: true,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **Scroll 分支**（C2，DESIGN §9/§11）：视口滚动（target=viewport）或元素滚进视口（target=element）。
    /// - **viewport**：[`Self::scroll_viewport`]（`Input.dispatchMouseEvent{mouseWheel, deltaX/deltaY}`，
    ///   [`crate::input::scroll_deltas`] 按 direction/amount 换算，**禁 DPR**）；verify 读回 scrollY/X 证变。
    /// - **element**：resolve_ref → [`Self::scroll_element_into_view`]（注入 scrollIntoView，**4 alignment
    ///   逃 sticky**：依次 [`SCROLL_ALIGNMENTS`]，取首个让目标 inViewport 的）。
    ///
    /// **良性态不报错**（验收）：已滚到底 / 无更多内容 → scrollY 不再变 → success=true（不 emit_error，
    /// 只是 `changed=false`，如实告诉模型「已到边界」）；element 4 alignment 都没完全逃出 sticky → 仍
    /// success=true（已尽力滚到最近），changed 据位置是否变化判。
    async fn act_scroll(
        &self,
        target: &ScrollTarget,
        direction: ScrollDir,
        amount: Option<f64>,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        match target {
            ScrollTarget::Viewport => {
                self.act_scroll_viewport(direction, amount, parent).await
            }
            ScrollTarget::Element { r#ref } => {
                self.act_scroll_element(r#ref, direction, amount, parent).await
            }
        }
    }

    /// 视口滚动（page-level，无 ref）：mouseWheel delta → 读回 scrollX/Y 前后对比。良性到底不报错。
    async fn act_scroll_viewport(
        &self,
        direction: ScrollDir,
        amount: Option<f64>,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            async move {
                let (dx, dy) = crate::input::scroll_deltas(direction, amount);
                let before = this.scroll_position().await;
                this.scroll_viewport(dx, dy).await.map_err(classify_browser_err)?;
                let after = this.scroll_position().await;
                let changed = (before.0 - after.0).abs() > 0.5 || (before.1 - after.1).abs() > 0.5;
                let message = if changed {
                    format!(
                        "scrolled viewport {direction:?}; scrollY {} -> {}; re-observe for new content",
                        before.1, after.1
                    )
                } else {
                    // 良性：已到边界（无更多内容）→ success 仍 true，如实告知。
                    format!("viewport already at the {direction:?} edge (no further content)")
                };
                Ok(ActResult {
                    message,
                    effect: Effect {
                        changed,
                        before_anchor: Some(serde_json::json!({"scrollX": before.0, "scrollY": before.1})),
                        after_anchor: Some(serde_json::json!({"scrollX": after.0, "scrollY": after.1})),
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// element 滚进视口（ref-based，复用 act_with_skeleton）：4 alignment 逃 sticky；良性态不报错。
    async fn act_scroll_element(
        &self,
        llm_ref: &str,
        direction: ScrollDir,
        _amount: Option<f64>,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let llm_ref_owned = llm_ref.to_string();
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            async move {
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;

                // 4 alignment 逃 sticky（裁决⑮）：依次试 center/start/end/nearest，取首个让目标 inViewport。
                let manager = this
                    .manager_for_record(&rec)
                    .await
                    .ok_or(RetryDecision::Retryable(BrowserError::NotConnected))?;
                let mut last_pos: Option<serde_json::Value> = None;
                let mut in_viewport = false;
                for block in SCROLL_ALIGNMENTS {
                    let pos = manager
                        .scroll_into_view(&handle.object_id, block)
                        .await
                        .map_err(classify_inject_err)?;
                    in_viewport = pos
                        .get("value")
                        .and_then(|v| v.get("inViewport"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    last_pos = pos.get("value").cloned();
                    if in_viewport {
                        break;
                    }
                }

                let message = if in_viewport {
                    format!("scrolled {llm_ref} into view ({direction:?}); re-observe")
                } else {
                    // 良性：4 alignment 都没完全逃出 sticky → 已尽力滚到最近，success 仍 true。
                    format!("scrolled {llm_ref} as close as possible (may be partly under a sticky header); re-observe")
                };
                Ok(ActResult {
                    message,
                    effect: Effect {
                        changed: true,
                        before_anchor: None,
                        after_anchor: last_pos,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **取点失败时的 sticky 逃逸**（C2，DESIGN §11 设计裁决⑮，落实 actions.rs gate_geom_err 的
    /// 原 TODO(C2)）：当 [`crate::input::pick_click_point`] 报 `NotVisible`/`NotInViewport`（元素被
    /// sticky 顶/底栏遮挡或滚出视口），按 [`SCROLL_ALIGNMENTS`]（`center`/`start`/`end`/`nearest`）
    /// 依次调既有注入 [`crate::injected::InjectionManager::scroll_into_view`]，取**首个**让目标
    /// `inViewport == true` 的 alignment 后立即返 `Ok(true)`；4 个都没完全逃出 → `Ok(false)`（调用方
    /// 退一步走 `gate_geom_err` 瞬态重试，退避内可能就绪）。
    ///
    /// **复用既有原语零新 CDP**：`scroll_into_view` 已在 `InjectionManager` 内经传输层（含超时），返
    /// `{inViewport}`（CSS 像素 `getBoundingClientRect`，**禁 DPR**）。`manager_for_record` 选元素所属帧
    /// 注入管线；管线找不到（OOPIF 子 session detach）→ `Err(RetryDecision::Retryable(NotConnected))`
    /// （让外层重解析 ref）。注入异常经 [`classify_inject_err`] 分类（ContextNotReady→Retryable）。**绝不 panic**。
    async fn scroll_escape_sticky(
        &self,
        rec: &RefRecord,
        handle: &ObjectHandle,
    ) -> Result<bool, RetryDecision> {
        let manager = self
            .manager_for_record(rec)
            .await
            .ok_or(RetryDecision::Retryable(BrowserError::NotConnected))?;
        for block in SCROLL_ALIGNMENTS {
            let pos = manager
                .scroll_into_view(&handle.object_id, block)
                .await
                .map_err(classify_inject_err)?;
            let in_viewport = pos
                .get("value")
                .and_then(|v| v.get("inViewport"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if in_viewport {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// **取点(含 sticky 逃逸)收口**（C2）：取元素 content quads → [`crate::input::pick_click_point`]；
    /// 成功直接返点。失败（`NotVisible`/`NotInViewport`，元素被 sticky 遮挡或滚出视口）→ 调
    /// [`Self::scroll_escape_sticky`] 按 4 alignment 逃逸：逃出（`inViewport`）则**重取一次点**（逃逸后
    /// 几何已变）；逃不出 / 重取仍失败 → 退回 [`gate_geom_err`] 瞬态 [`RetryDecision::Retryable`]（外层
    /// 退避后整个 attempt 重来，懒加载/动画可能就绪）。**CSS 像素全程禁 DPR**。**绝不 panic**。
    async fn pick_point_escaping_sticky(
        &self,
        rec: &RefRecord,
        handle: &ObjectHandle,
    ) -> Result<crate::input::Point, RetryDecision> {
        // 第一次取点。
        let quads = self
            .element_content_quads(handle)
            .await
            .map_err(classify_browser_err)?;
        let (vw, vh) = self.viewport_size().await.map_err(classify_browser_err)?;
        match crate::input::pick_click_point(&quads, vw, vh) {
            Ok(p) => return Ok(p),
            Err(_first_err) => {
                // 取点失败：尝试 4-alignment 逃 sticky。逃不出 → 退瞬态重试（用 NotInViewport 文案，
                // 因 sticky 遮挡语义最贴近「需要滚动」）。
                if !self.scroll_escape_sticky(rec, handle).await? {
                    return Err(gate_geom_err(crate::input::GeomError::NotInViewport));
                }
            }
        }
        // 逃出 sticky 后几何已变：重取一次点。仍失败 → 退瞬态重试（保留真实 first/second 错误语义）。
        let quads = self
            .element_content_quads(handle)
            .await
            .map_err(classify_browser_err)?;
        let (vw, vh) = self.viewport_size().await.map_err(classify_browser_err)?;
        crate::input::pick_click_point(&quads, vw, vh).map_err(gate_geom_err)
    }

    /// **ScrollToText 分支**（C2，DESIGN §9，**页面级无 ref**）：注入文本查找（遍历可见文本节点找首个
    /// 含 `text` 的元素）→ 找到则 `scrollIntoView` 并返回是否进视口。**未找到 → success=false 如实**
    /// （非报错——文本可能还没加载/拼写不符，引导模型 re-observe 或换词）。
    async fn act_scroll_to_text(
        &self,
        text: &str,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let text_owned = text.to_string();
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            let text = text_owned.clone();
            async move {
                let found = this.act_scroll_to_text_impl(&text).await.map_err(classify_browser_err)?;
                if found {
                    Ok(ActResult {
                        message: format!("scrolled to text \"{text}\"; re-observe to see it in view"),
                        effect: Effect {
                            changed: true,
                            before_anchor: None,
                            after_anchor: None,
                        },
                        success: true,
                    })
                } else {
                    // 未命中：success=false 如实（非报错，良性态）。
                    Ok(ActResult {
                        message: format!("text \"{text}\" not found on the page; re-observe or try different wording"),
                        effect: Effect {
                            changed: false,
                            before_anchor: None,
                            after_anchor: None,
                        },
                        success: false,
                    })
                }
            }
        })
        .await
    }

    /// **页面级动作简化骨架**（C2，press_key/scroll-viewport/scroll_to_text 用）：与
    /// [`Self::act_with_skeleton`] 同样 arm abort 作用域（detach/crash → 立即 abort）+ retry 编排，但
    /// **无 ref 解析 / 无 objectGroup**（页面级动作不锚元素句柄）。abort 绑主帧（页面级动作影响主文档）。
    /// retry `irreversible=false`（页面级键/滚动可重试瞬态缺态；真正的 IRREVERSIBLE 强制门在 E2/F1）。
    async fn act_page_with_skeleton<F, Fut>(
        &self,
        parent: &Progress,
        op: F,
    ) -> Result<ActResult, BrowserError>
    where
        F: Fn(usize) -> Fut,
        Fut: Future<Output = Result<ActResult, RetryDecision>>,
    {
        // abort 作用域：绑主帧（页面级动作影响主文档；page.close/主帧 detach → 立即 abort）。
        // D1：main_frame_id / arm_act_abort 现 async（经 active tab 解引用）。
        let main_frame_id = self.main_frame_id().await?;
        let (child, _abort_guard) = self.arm_act_abort(parent, &main_frame_id).await?;
        run_act_with_retry(&child, false, op).await
    }

    /// **[运行时] 当前焦点是否在 `<form>` 内**（press_key 的 Enter-in-form 检测取此运行时信息）。
    /// 在**当前作用帧**（[`Self::active_frame_eval`]）查 `document.activeElement.closest('form') != null`。
    /// best-effort：取不到 / 异常 → `false`（保守不升级不可逆——非阻塞）。`activeElement` 是页面态，
    /// 但 utility world（isolated）共享同一 document，仍读得到 activeElement。
    async fn act_focus_in_form(&self) -> bool {
        let expression = "(() => { const a = document.activeElement; return !!(a && a.closest && a.closest('form')); })()";
        self.active_frame_eval(expression)
            .await
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// **[运行时] 注入文本查找 + scrollIntoView**（scroll_to_text 的真执行）：在**当前作用帧**
    /// （[`Self::active_frame_eval`]）跑只读+滚动脚本，遍历可见文本节点（含 open shadow root）找首个
    /// 含 `text` 的元素，找到则 `scrollIntoView({block:'center'})` 并返回 `true`；未找到返 `false`。
    /// **禁 DPR**（scrollIntoView 是 CSS 像素语义）。脚本异常 → 上抛（Fatal）。**只读 DOM**。
    async fn act_scroll_to_text_impl(&self, text: &str) -> Result<bool, BrowserError> {
        // 文本经 JSON.stringify 安全内联（防注入/引号）。遍历文本节点（TreeWalker），大小写不敏感 contains。
        let needle = serde_json::Value::String(text.to_string()).to_string();
        let expression = format!(
            "(() => {{ \
               const needle = {needle}.toLowerCase(); \
               if (!needle) return false; \
               const seen = new Set(); \
               const search = (root) => {{ \
                 let walker; \
                 try {{ walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); }} catch (e) {{ return null; }} \
                 let node; \
                 while ((node = walker.nextNode())) {{ \
                   const t = (node.textContent || '').toLowerCase(); \
                   if (t.includes(needle)) {{ \
                     const el = node.parentElement; \
                     if (el) return el; \
                   }} \
                 }} \
                 return null; \
               }}; \
               const walkShadow = (root) => {{ \
                 const hit = search(root); \
                 if (hit) return hit; \
                 let all; \
                 try {{ all = root.querySelectorAll('*'); }} catch (e) {{ return null; }} \
                 for (const el of all) {{ \
                   if (el.shadowRoot && !seen.has(el.shadowRoot)) {{ \
                     seen.add(el.shadowRoot); \
                     const h = walkShadow(el.shadowRoot); \
                     if (h) return h; \
                   }} \
                 }} \
                 return null; \
               }}; \
               const target = walkShadow(document); \
               if (!target) return false; \
               try {{ target.scrollIntoView({{ block: 'center', inline: 'nearest' }}); }} catch (e) {{}} \
               return true; \
             }})()"
        );
        let value = self.active_frame_eval(&expression).await?;
        Ok(value.as_bool().unwrap_or(false))
    }

    /// tier1 fill：在元素所属帧的 utility world 跑 vendored `fill(node,value)`（[`InjectionManager::fill_element`]）。
    /// 注入异常（不可编辑特例 / context 未就绪）经 [`classify_inject_err`] 成 [`RetryDecision`]（不可编辑→Fatal）。
    async fn act_fill(
        &self,
        rec: &RefRecord,
        handle: &ObjectHandle,
        value: &str,
    ) -> Result<FillOutcome, RetryDecision> {
        let manager = self
            .manager_for_record(rec)
            .await
            .ok_or(RetryDecision::Retryable(BrowserError::NotConnected))?;
        let result = manager
            .fill_element(&rec.frame_id, &handle.object_id, value)
            .await
            .map_err(classify_inject_err)?;
        Ok(parse_fill_outcome(result.get("value")))
    }

    /// tier3 逐字符键入（insertText 失败的最后逃生）：每个 char 经 [`Self::key_combo`] 发 keyDown/keyUp。
    /// 单字符（字母/数字）走 parse_key_combo 的单键路径；非 ASCII / 组合字符不在 US 布局映射 → key_combo
    /// 返 UnknownKey（上抛 Other），由调用方按非瞬态处理（tier3 是逃生舱，覆盖面有限是已知约束）。
    async fn act_type_per_char(&self, text: &str) -> Result<(), BrowserError> {
        for ch in text.chars() {
            // 空格走 "Space"，其余字符直接当 key token（单 ASCII 字母/数字 parse_key_combo 认得）。
            let token = if ch == ' ' { "Space".to_string() } else { ch.to_string() };
            self.key_combo(&token).await?;
        }
        Ok(())
    }

    /// 读元素当前 `value`（input/textarea/**select**）或 `textContent`（其它）作 verify 锚点。
    /// **select 必读 `.value`/`selectedOptions`**（HTMLSelectElement.value = 当前选中 option 的 value；
    /// 多选拼 selectedOptions 的 value）——否则 `textContent` 返回全部 option 文案拼接，select_option
    /// 前后锚点都是同一份 option 列表 → 误报 `changed=false`。best-effort：读不到返 None（不致命；
    /// verify 锚点缺失只是 Effect.before/after 为 None，不影响动作成功判定）。
    async fn act_read_element_value(&self, handle: &ObjectHandle) -> Option<String> {
        let read_fn = "function() { \
             if (!this) return null; \
             if (this.tagName === 'SELECT') { \
                 return this.multiple \
                     ? Array.from(this.selectedOptions).map(function(o){ return o.value; }).join(',') \
                     : this.value; \
             } \
             if (this.tagName === 'INPUT' || this.tagName === 'TEXTAREA') return this.value; \
             return this.textContent || ''; \
         }";
        // D1：injection_manager 现 async（active tab 解引用）；best-effort 读不到 / active tab 缺失返 None。
        let manager = self.injection_manager().await.ok()?;
        let result = manager.call_on_element(&handle.object_id, read_fn, true).await.ok()?;
        result.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
    }

    /// **F2 富锚点：读 click 目标元素的可观测 DOM 态快照**作 verify before/after 锚点。返回一个不透明
    /// JSON 对象，含**点击常见会改变**的状态：可见文本（`text`，trim 后截断防爆 token）、表单 `value`、
    /// `checked`（checkbox/radio——点击切换的关键证据，URL 不变也能判 changed）、`aria-pressed`/
    /// `aria-expanded`（toggle/disclosure 按钮的可观测态）。这让 checkbox/radio/toggle 类点击即便不
    /// 导航、不改 URL，也能经 before≠after 如实判 `changed=true`（任务⑫ checkbox/radio click：checked
    /// 前后）。best-effort：读不到（active tab 缺失 / 异常）→ None（锚点缺失只让 Effect 该端为 None，
    /// 不影响动作成功判定）。**只读**（只读属性，不改 DOM）。
    async fn act_read_click_anchor(&self, handle: &ObjectHandle) -> Option<serde_json::Value> {
        let read_fn = "function() { \
             if (!this) return null; \
             var out = {}; \
             try { \
                 var tag = this.tagName; \
                 if (tag === 'INPUT' && (this.type === 'checkbox' || this.type === 'radio')) { \
                     out.checked = !!this.checked; \
                 } else if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') { \
                     out.value = this.value; \
                 } \
                 var pressed = this.getAttribute && this.getAttribute('aria-pressed'); \
                 if (pressed !== null && pressed !== undefined) out.ariaPressed = pressed; \
                 var expanded = this.getAttribute && this.getAttribute('aria-expanded'); \
                 if (expanded !== null && expanded !== undefined) out.ariaExpanded = expanded; \
                 var text = (this.textContent || '').trim(); \
                 if (text.length > 200) text = text.slice(0, 200); \
                 out.text = text; \
             } catch (e) {} \
             return out; \
         }";
        let manager = self.injection_manager().await.ok()?;
        let result = manager
            .call_on_element(&handle.object_id, read_fn, true)
            .await
            .ok()?;
        result.get("value").cloned()
    }

    /// 当前 page url（verify 锚点；点击可能触发导航/同文档跳转）。best-effort：取不到 / active tab
    /// 缺失返 None。**D1：经 active tab 解引用**。
    pub(crate) async fn act_current_url(&self) -> Option<String> {
        let session = self.page_session_id().await.ok()?;
        let mut params =
            chromiumoxide::cdp::js_protocol::runtime::EvaluateParams::new("location.href".to_string());
        params.return_by_value = Some(true);
        let result = self
            .conn()
            .send::<chromiumoxide::cdp::js_protocol::runtime::EvaluateParams>(
                &session,
                &params,
            )
            .await
            .ok()?;
        result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    // ═══════════════════════════════════════════════════════════════════════
    // C3：只读类动作（Info 级，**全只读零写**：不点不改 DOM）。
    // get_page_text/search_page 的页面文本是**不可信内容** → 必须 redact_yaml（抹凭据）+
    // wrap_untrusted（<data> 包裹防注入），镜像 P1 observe 的安全契约。
    // find_elements 复用 P1 ref 登记（注入侧写 _lastAriaSnapshotForQuery.elements + _ariaRef，
    // 宿主侧登记进当代际 RefTable）——登记的 ref 能被 resolve_ref_to_object 反解。
    // cursor 对齐 observe 的 [cursor=pointer] 标记（cursor==='pointer'）。
    // wait/wait_for 挂 Progress::race（abort 优先 timeout）。
    // ═══════════════════════════════════════════════════════════════════════

    /// **GetPageText 分支**（C3，DESIGN §9，Info 级只读）：注入取整页可读文本
    /// （[`Self::act_extract_page_text`]，`document.body.innerText`）→ **必须 redact（抹凭据）+
    /// wrap_untrusted（`<data>` 包裹防注入）**（喂 LLM 的不可信内容，镜像 P1 observe 安全契约）→ 文本进
    /// `message`。页面级动作（无 ref），走简化骨架（abort+retry，无 ref 解析）。
    async fn act_get_page_text(&self, parent: &Progress) -> Result<ActResult, BrowserError> {
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            async move {
                let raw = this
                    .act_extract_page_text()
                    .await
                    .map_err(classify_browser_err)?;
                // 不可信内容安全契约：先脱敏（抹凭据/高熵 token），再 <data> 包裹（防提示注入越狱）。
                let url = this.act_current_url().await;
                let redacted = crate::redact::redact_yaml(&raw);
                let wrapped = crate::redact::wrap_untrusted(&redacted, url.as_deref());
                Ok(ActResult {
                    // 只读动作不改页面：changed=false，文本在 message。
                    message: format!("page text (untrusted, redacted):\n{wrapped}"),
                    effect: Effect {
                        changed: false,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **SearchPage 分支**（C3，DESIGN §9，Info 级只读）：取整页文本（同 get_page_text）→ **先脱敏**
    /// → grep `query`（大小写不敏感子串，[`grep_page_text`]）→ 命中片段进 message（同样 redact+wrap）。
    /// 未命中 → success=false 如实（非报错，良性态：拼写不符/未加载，引导换词或滚动）。零成本 token 利器。
    async fn act_search_page(
        &self,
        query: &str,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let query_owned = query.to_string();
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            let query = query_owned.clone();
            async move {
                let raw = this
                    .act_extract_page_text()
                    .await
                    .map_err(classify_browser_err)?;
                // **先脱敏再 grep**：在脱敏后的文本上匹配——命中片段绝不含明文 secret，且 query 命中
                // `[REDACTED_SECRET]` 这类占位也无害（不泄漏）。
                let redacted = crate::redact::redact_yaml(&raw);
                // cap 命中条数防超大页爆 token（50 条上下文足够模型定位）。
                let hits = grep_page_text(&redacted, &query, 50);
                let url = this.act_current_url().await;
                if hits.is_empty() {
                    // 良性：未命中（或空 query）→ success=false 如实。
                    return Ok(ActResult {
                        message: format!(
                            "no match for {query:?} on the page; try different wording, scroll, or re-observe"
                        ),
                        effect: Effect {
                            changed: false,
                            before_anchor: None,
                            after_anchor: None,
                        },
                        success: false,
                    });
                }
                // 命中片段同样 <data> 包裹（不可信内容防注入）。
                let body = hits
                    .iter()
                    .map(|h| h.line.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let wrapped = crate::redact::wrap_untrusted(&body, url.as_deref());
                Ok(ActResult {
                    message: format!(
                        "{} match(es) for {query:?} (untrusted, redacted):\n{wrapped}",
                        hits.len()
                    ),
                    effect: Effect {
                        changed: false,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **FindElements 分支**（C3，DESIGN §9，Info 级只读）：CSS 选择器查元素 + **登记可反解的 ref**。
    ///
    /// 注入 [`crate::injected::InjectionManager::find_elements`]：`querySelectorAll(selector)` 命中元素 →
    /// 对每个登记一个新 ref 到注入侧 `_lastAriaSnapshotForQuery.elements`（vendored `aria-ref=` engine 读的
    /// 同一张表）+ `_ariaRef` expando（层③ role 校验读它）。**宿主侧**再把每个 ref 登记进**当前代际**
    /// [`crate::aria_ref::RefTable`]（与 observe 登记 ref 同表同代际）——故登记的 ref 后续能被
    /// [`Self::resolve_ref_to_object`] 反解回 objectId（端到端可 act）。
    ///
    /// **复用 P1 ref 登记不另造编号**：ref 形如 `f<seq>e<n>`（与 observe 同形，refPrefix 取主帧 observe 的
    /// 前缀）；注入侧用专用高位计数器分配 `e<n>` 并与现有 key 去重，与 snapshot lastRef 互不串号。
    ///
    /// 页面级动作（selector 页面作用域，C3 只查主帧），走简化骨架。该帧须新近 observe 过（否则注入返
    /// `notobserved`，引导先 observe）。selector 非法 → 注入抛 → Fatal。命中片段进 message（role/name/ref
    /// 供模型后续 act 引用）。**只读零写**（仅写注入侧缓存 + ref 表，不改页面可见 DOM）。
    async fn act_find_elements(
        &self,
        selector: &str,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let selector_owned = selector.to_string();
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            let selector = selector_owned.clone();
            async move {
                // D4：作用帧经 active_page_frame（switch_frame 后是 iframe，否则主帧）。同进程 iframe
                // 与父帧同 page session，故 injection_manager 仍是 active tab 的；只 frame_id 不同。
                let (_frame_session, frame_id) =
                    this.active_page_frame().await.map_err(classify_browser_err)?;
                // refPrefix：复用该帧 observe 的前缀（与该帧 snapshot ref 同形），无则退 "f0"。
                let ref_prefix = this.frame_ref_prefix(&frame_id).await;
                // cap 命中条数防超大命中爆 token（登记 50 个 ref 足够模型选用）。
                let result = this
                    .injection_manager()
                    .await
                    .map_err(classify_browser_err)?
                    .find_elements(&frame_id, &selector, &ref_prefix, 50)
                    .await
                    .map_err(classify_inject_err)?;
                match parse_find_outcome(result.get("value")) {
                    FindOutcome::Found { matches, total } => {
                        // 把命中的 ref 登记进**当前代际** RefTable（与 observe 同表同代际，故可被 resolve）。
                        this.register_found_refs(&frame_id, &matches).await;
                        let listing = matches
                            .iter()
                            .map(|m| {
                                if m.name.is_empty() {
                                    format!("- {} [ref={}]", m.role, m.r#ref)
                                } else {
                                    format!("- {} {:?} [ref={}]", m.role, m.name, m.r#ref)
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let head = if total > matches.len() {
                            format!(
                                "found {total} element(s) matching {selector:?} (showing first {}); act on a [ref]:",
                                matches.len()
                            )
                        } else {
                            format!("found {} element(s) matching {selector:?}; act on a [ref]:", matches.len())
                        };
                        Ok(ActResult {
                            message: if matches.is_empty() {
                                format!("no element matched {selector:?}; try a different selector or re-observe")
                            } else {
                                format!("{head}\n{listing}")
                            },
                            effect: Effect {
                                changed: false,
                                before_anchor: None,
                                after_anchor: None,
                            },
                            // 命中 0 个也 success=true（合法查询，只是没匹配）——区别于 notobserved 错误。
                            success: true,
                        })
                    }
                    // 该帧没 observe 过：引导先 observe（瞬态——observe 后即可用，故可重试一次）。
                    FindOutcome::NotObserved => Err(RetryDecision::Retryable(BrowserError::NodeStale {
                        generation: this.current_generation().await,
                    })),
                    FindOutcome::Unknown => Err(RetryDecision::Retryable(BrowserError::Other(
                        "find_elements returned an unexpected shape".into(),
                    ))),
                }
            }
        })
        .await
    }

    /// **GetDropdownOptions 分支**（C3，DESIGN §9，Info 级只读）：resolve_ref → 须 `<select>` →
    /// 注入 [`crate::injected::InjectionManager::dropdown_options`] 枚举 `<option>`（value/label/selected/
    /// disabled）→ 列表进 message。**只读零写**（不选不改不派发事件——区别于 C2 的 select_option）。
    /// 元素非 `<select>` → success=false 如实（良性，引导换 ref）；detach → 可重试。复用 ref 骨架（含
    /// objectGroup 释放）；**不做 actionability 检查**（只读枚举，靠注入侧 `isConnected`+`tagName==='SELECT'` 兜底）。
    async fn act_get_dropdown_options(
        &self,
        llm_ref: &str,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let llm_ref_owned = llm_ref.to_string();
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            async move {
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;
                let result = this
                    .injection_manager()
                    .await
                    .map_err(classify_browser_err)?
                    .dropdown_options(&handle.object_id)
                    .await
                    .map_err(classify_inject_err)?;
                match parse_dropdown_outcome(result.get("value")) {
                    DropdownOutcome::Options(options) => {
                        let listing = options
                            .iter()
                            .map(|o| {
                                let mut flags = Vec::new();
                                if o.selected {
                                    flags.push("selected");
                                }
                                if o.disabled {
                                    flags.push("disabled");
                                }
                                let tail = if flags.is_empty() {
                                    String::new()
                                } else {
                                    format!(" [{}]", flags.join(","))
                                };
                                format!("- {:?} (value={:?}){tail}", o.label, o.value)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(ActResult {
                            message: format!(
                                "{} option(s) on {llm_ref}:\n{listing}",
                                options.len()
                            ),
                            effect: Effect {
                                changed: false,
                                before_anchor: None,
                                after_anchor: None,
                            },
                            success: true,
                        })
                    }
                    // 元素不是 <select>：良性失败如实（引导模型换 ref / 用其它动作）。
                    DropdownOutcome::NotSelect => Ok(ActResult {
                        message: format!("{llm_ref} is not a <select>; get_dropdown_options only works on dropdowns"),
                        effect: Effect {
                            changed: false,
                            before_anchor: None,
                            after_anchor: None,
                        },
                        success: false,
                    }),
                    DropdownOutcome::NotConnected => {
                        Err(RetryDecision::Retryable(BrowserError::NotConnected))
                    }
                    DropdownOutcome::Unknown => Err(RetryDecision::Retryable(BrowserError::Other(
                        "dropdown_options returned an unexpected shape".into(),
                    ))),
                }
            }
        })
        .await
    }

    /// **Cursor 分支**（C3，DESIGN §8/§9，Info 级只读）：报告页面上「可点击」（`cursor:pointer`）元素的
    /// 数量提示——对齐 observe 的 `[cursor=pointer]` 标记（vendored `hasPointerCursor` 判 `cursor==='pointer'`，
    /// isomorphic/ariaSnapshot.ts:65-66）。**只读零写**（仅读 computed style，不点不改 DOM）。页面级动作。
    ///
    /// 语义采纳：DESIGN §8 把 `[cursor=pointer]` 列为 aria-snapshot 承载的「可交互性」标记；§9 把 `cursor`
    /// 列为只读 Info 动作。最自洽的只读实现 = 给模型一个「页面有多少 cursor:pointer 可点元素」的提示
    /// （引导它优先 observe 看带 `[cursor=pointer]` 的 ref，而非滚动猜）。
    async fn act_cursor(&self, parent: &Progress) -> Result<ActResult, BrowserError> {
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            async move {
                let count = this
                    .act_count_pointer_cursor()
                    .await
                    .map_err(classify_browser_err)?;
                Ok(ActResult {
                    message: format!(
                        "{count} element(s) have a pointer cursor (clickable); observe to see them as [cursor=pointer] refs"
                    ),
                    effect: Effect {
                        changed: false,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **Wait 分支**（C3，DESIGN §22，Info 级）：固定等待若干毫秒（钳制到 [`WAIT_MS_CAP`] 防整轮挂死）。
    /// sleep 挂 `Progress::race`（abort 优先——等待期间 page.close/frame.detach 立即打断，不白等）。
    /// 页面级动作（无 DOM 改动）。被钳制时文案如实告知。
    async fn act_wait(&self, ms: u64, parent: &Progress) -> Result<ActResult, BrowserError> {
        let (clamped, was_capped) = clamp_wait_ms(ms);
        self.act_page_with_skeleton(parent, move |_attempt| {
            async move {
                // sleep 挂在 act_page_with_skeleton 内层 run_act_with_retry 的 progress.race 上：
                // op 本身就被 race 包裹（attempt 跑在 race 里），故这里直接 sleep 即与 abort/deadline 竞速。
                tokio::time::sleep(Duration::from_millis(clamped)).await;
                let message = if was_capped {
                    format!(
                        "waited {clamped} ms (requested {ms} ms was capped to the {WAIT_MS_CAP} ms limit)"
                    )
                } else {
                    format!("waited {clamped} ms")
                };
                Ok(ActResult {
                    message,
                    effect: Effect {
                        changed: false,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **WaitFor 分支**（C3，DESIGN §9/§22，Info 级）：轮询直到 `condition` 满足或 deadline
    /// （[`WAIT_FOR_DEFAULT_TIMEOUT`]）→ `Timeout{phase:Action}`。每次轮询挂 `Progress::race`（abort 优先），
    /// 退避 [`WAIT_FOR_POLL_INTERVAL`]。条件（[`WaitCondition`]）：
    /// - `UrlContains`：当前 URL 含子串（SPA 软导航降级可用——URL 变化即满足，不依赖 load 事件）；
    /// - `TextVisible`：某文本在页面可见（遍历可见文本节点，含 open shadow）；
    /// - `RefActionable`：某 ref 元素 actionable（resolve + check visible/enabled/stable）。
    ///
    /// **deadline 用独立短预算**（不并入父 30s）：轮询超过 [`WAIT_FOR_DEFAULT_TIMEOUT`] → Timeout{Action}。
    /// 父 `parent` 仍管 abort（page.close → 立即停）。**只读零写**（只检查条件，不点不改）。
    async fn act_wait_for(
        &self,
        condition: &WaitCondition,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        // 独立短 deadline（wait_for 专用，不并入父预算）：派生一个 WAIT_FOR_DEFAULT_TIMEOUT 子 Progress，
        // 绑父 token（page.close → 父取消 → 子立即取消）。轮询挂它的 race（abort 优先 timeout）。
        let poll_scope = Progress::child(WAIT_FOR_DEFAULT_TIMEOUT, parent.token());
        let deadline = tokio::time::Instant::now() + WAIT_FOR_DEFAULT_TIMEOUT;
        loop {
            // 每次检查条件：满足 → 立返成功。检查本身挂 race（abort 优先，且条件检查里的 CDP I/O 被打断）。
            let satisfied = poll_scope
                .race(self.act_check_wait_condition(condition))
                .await
                .map_err(map_progress_err)??;
            if satisfied {
                return Ok(ActResult {
                    message: format!("condition satisfied: {}", describe_wait_condition(condition)),
                    effect: Effect {
                        changed: false,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                });
            }
            // 未满足：超 deadline → Timeout{Action}；否则退避后重判（退避也挂 race，abort 立即打断）。
            if tokio::time::Instant::now() >= deadline {
                return Err(BrowserError::Timeout {
                    phase: crate::engine::NavPhase::Action,
                });
            }
            poll_scope
                .race(tokio::time::sleep(WAIT_FOR_POLL_INTERVAL))
                .await
                .map_err(map_progress_err)?;
        }
    }

    /// **[运行时] 检查一次 [`WaitCondition`] 是否满足**（wait_for 单次轮询）。只读，best-effort：
    /// 取不到（CDP 出错/元素暂不可解）→ `Ok(false)`（继续轮询，非致命；真致命如 page.close 由 race 的
    /// abort 打断）。
    async fn act_check_wait_condition(
        &self,
        condition: &WaitCondition,
    ) -> Result<bool, BrowserError> {
        match condition {
            WaitCondition::UrlContains { text } => {
                let url = self.act_current_url().await.unwrap_or_default();
                Ok(url.contains(text.as_str()))
            }
            WaitCondition::TextVisible { text } => {
                // 复用 C2 的文本查找（遍历可见文本节点 + open shadow），但**不滚动**——只判存在。
                self.act_text_present(text).await
            }
            WaitCondition::RefActionable { r#ref } => {
                // resolve（层①②③）→ check_states(visible/enabled/stable)。任一失败 → false（继续轮询）。
                let Ok(rec) = self.resolve_ref_record(r#ref).await else {
                    return Ok(false);
                };
                let seq = self.next_act_seq();
                let handle = match self.resolve_ref_to_object(&rec, seq).await {
                    Ok(h) => h,
                    Err(_) => {
                        // resolve 失败也可能已在 act-<seq> 组留下对象（层② NotConnected 路径在返 Err
                        // 前已 query 进组）→ 轮询多次故必须释放，否则每 poll 泄漏一个空组。
                        self.release_act_group(&rec, seq).await;
                        return Ok(false);
                    }
                };
                let actionable = matches!(
                    self.check_states(&handle, &["visible", "stable", "enabled"]).await,
                    Ok(crate::actionability::CheckResult::Pass)
                );
                // 释放本次 resolve 的句柄组（轮询多次，防泄漏）。
                self.release_act_group(&rec, seq).await;
                Ok(actionable)
            }
        }
    }

    // ── C3 只读运行时 helper（注入侧只读脚本；不改页面 DOM）─────────────────────────────

    /// **[运行时] 取整页可读文本**（get_page_text/search_page 的页面文本源）：在**当前作用帧**
    /// （[`Self::active_frame_eval`]，switch_frame 后是 iframe，否则主帧）跑 `document.body.innerText`
    /// （已折叠不可见/script/style，是「人读」文本）。空 body → 空串。异常 → 上抛（Fatal）。**只读**。
    async fn act_extract_page_text(&self) -> Result<String, BrowserError> {
        let expression =
            "(() => { try { return document.body ? document.body.innerText : ''; } catch (e) { return ''; } })()";
        let value = self.active_frame_eval(expression).await?;
        Ok(value.as_str().unwrap_or_default().to_string())
    }

    /// **[运行时] 判某文本是否在页面可见**（wait_for TextVisible）：在**当前作用帧**
    /// （[`Self::active_frame_eval`]）遍历可见文本节点（含 open shadow），大小写不敏感 contains，
    /// **不滚动**（区别于 C2 scroll_to_text）。best-effort：异常 → `Ok(false)`。
    async fn act_text_present(&self, text: &str) -> Result<bool, BrowserError> {
        let needle = serde_json::Value::String(text.to_string()).to_string();
        let expression = format!(
            "(() => {{ \
               const needle = {needle}.toLowerCase(); \
               if (!needle) return false; \
               const seen = new Set(); \
               const search = (root) => {{ \
                 let walker; \
                 try {{ walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); }} catch (e) {{ return false; }} \
                 let node; \
                 while ((node = walker.nextNode())) {{ \
                   if ((node.textContent || '').toLowerCase().includes(needle)) return true; \
                 }} \
                 return false; \
               }}; \
               const walkShadow = (root) => {{ \
                 if (search(root)) return true; \
                 let all; \
                 try {{ all = root.querySelectorAll('*'); }} catch (e) {{ return false; }} \
                 for (const el of all) {{ \
                   if (el.shadowRoot && !seen.has(el.shadowRoot)) {{ \
                     seen.add(el.shadowRoot); \
                     if (walkShadow(el.shadowRoot)) return true; \
                   }} \
                 }} \
                 return false; \
               }}; \
               return walkShadow(document); \
             }})()"
        );
        // best-effort：active_frame_eval 异常（含 JS 抛）→ 退 false（继续轮询，非致命）。
        Ok(self
            .active_frame_eval(&expression)
            .await
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// **[运行时] 数页面上 `cursor:pointer`（可点击）元素**（cursor 动作）：在**当前作用帧**
    /// （[`Self::active_frame_eval`]）遍历元素读 computed `cursor === 'pointer'`（与 vendored
    /// `hasPointerCursor` 同信号）。best-effort：异常 → `Ok(0)`。**只读**（只读 style，不改 DOM）。
    async fn act_count_pointer_cursor(&self) -> Result<u64, BrowserError> {
        let expression = "(() => { try { \
             let n = 0; \
             for (const el of document.querySelectorAll('*')) { \
               try { if (getComputedStyle(el).cursor === 'pointer') n++; } catch (e) {} \
             } \
             return n; \
           } catch (e) { return 0; } })()";
        Ok(self
            .active_frame_eval(expression)
            .await
            .ok()
            .and_then(|v| v.as_u64())
            .unwrap_or(0))
    }

    /// **[运行时] 某帧 observe 的 ref 前缀**（D4：find_elements 在作用帧上生成 ref 复用之，保持与
    /// snapshot 同形）。从当前代际 RefTable 找 `frame_id` 的任一 ref，取其 `f<seq>` 前缀；表为空 / 该帧
    /// 无 ref → 退 `"f0"`（主帧默认前缀，主帧 seq=0；非主帧无 observe 数据时退 "f0" 是 best-effort，注入
    /// 侧会先报 notobserved）。
    async fn frame_ref_prefix(&self, frame_id: &str) -> String {
        let Ok(ref_table) = self.ref_table_lock().await else {
            return "f0".to_string();
        };
        let guard = ref_table.lock().await;
        if let Some(table) = guard.as_ref() {
            for r in table.refs_for_frame(frame_id) {
                // ref = f<seq>e<n>：取到首个 'e' 之前作前缀（含 'f<seq>'）。
                if let Some(epos) = r.find('e') {
                    return r[..epos].to_string();
                }
            }
        }
        "f0".to_string()
    }

    /// **把 find_elements 登记的 ref 写进当前代际 RefTable**（C3 复用 P1 ref 登记，与 observe 同表同代际）。
    /// 注入侧已给元素打 `_ariaRef` + 写 `_lastAriaSnapshotForQuery.elements`（层②③可反解）；这里宿主侧把
    /// 每个 ref 登记进当前代际表（[`crate::aria_ref::RefTable::insert`]），使 [`Self::resolve_ref_record`]
    /// 的层① 也认它。表为空（还没 observe）则跳过（注入侧会先报 notobserved，不会走到这里）。
    /// session_id/frame_id 取主帧（find_elements 只查主帧）。**D1：经 active tab 解引用；active tab 缺失静默跳过**。
    async fn register_found_refs(&self, frame_id: &str, matches: &[FoundElement]) {
        let Ok(session_id) = self.page_session_id().await else {
            return;
        };
        let Ok(ref_table) = self.ref_table_lock().await else {
            return;
        };
        let mut guard = ref_table.lock().await;
        if let Some(table) = guard.as_mut() {
            for m in matches {
                table.insert(
                    &m.r#ref,
                    RefRecord {
                        session_id: session_id.clone(),
                        frame_id: frame_id.to_string(),
                        full_ref: m.r#ref.clone(),
                        role: m.role.clone(),
                        name: m.name.clone(),
                    },
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // F-actions：补全动作空间 upload_file / download / save_as_pdf / extract
    // （P2 DoD「完整动作空间 + extract」）。编排逻辑在此（skeleton/retry/RetryDecision/分类器）；
    // CDP 原语（set_file_input_files / trigger_anchor_download / poll_download_landed /
    // print_to_pdf / act_read_file_input / download_dir）在 backend/cdp.rs（持私有 self.conn 等）。
    // upload_file 走 ref 骨架（act_with_skeleton）；download/save_as_pdf/extract 是页面级（无 element
    // ref），走简化骨架（act_page_with_skeleton）。
    // ═══════════════════════════════════════════════════════════════════════

    /// **UploadFile 分支**（F-actions，DESIGN §9）：给一个 `<input type=file>` 设置上传文件路径——
    /// **不点系统文件对话框**（CDP 自动化点不到系统原生文件框），而是 `DOM.setFileInputFiles{files,
    /// objectId}` 直接把路径塞进 input.files（**这是绕过系统对话框的唯一正道，对标 Playwright**）。
    ///
    /// 步骤：resolve_ref（层①②③，须是 `<input type=file>`）→ check_states(visible/enabled)（不检
    /// editable——file input 不走 fill 路径）→ `DOM.setFileInputFiles`（用 utility-world 元素 objectId；
    /// DOM 域按 objectId 解析节点，跨 world 工作）→ verify 读回 `input.files.length` + 首文件名。元素非
    /// file input → CDP 报错 → Fatal（禁重试，引导换 ref）；detach → 可重试。
    ///
    /// **安全注意（SD-2 已实装）**：`paths` 经 [`validate_upload_paths`] 沙箱校验——必须
    /// canonicalize 落在 `workspace_dir` 内，否则 `Blocked`（fail-closed）。见 `validate_upload_path`。
    pub async fn act_upload_file(
        &self,
        llm_ref: &str,
        paths: &[PathBuf],
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        // SD-2 上传路径沙箱：校验 paths 在 workspace 内 / 拒绝敏感系统路径。Fail-closed。
        validate_upload_paths(paths, self.workspace_dir())?;

        let llm_ref_owned = llm_ref.to_string();
        // 路径转字符串（setFileInputFiles.files 是 Vec<String>，绝对路径）。lossy 兜底非法 UTF-8（罕见）。
        // `dunce::simplified` 剥掉 Windows verbatim `\\?\` 前缀（普通路径上是 no-op）——确保发给 Chrome 的
        // 永远是干净的 `C:\...` 形态，绝不让 `\\?\C:\...` 到达 setFileInputFiles（CDP 对 verbatim 路径可能
        // 解析失败）。非 Windows 上 simplified 直接原样返回。
        let file_strings: Vec<String> = paths
            .iter()
            .map(|p| dunce::simplified(p).to_string_lossy().into_owned())
            .collect();
        self.act_with_skeleton(llm_ref, parent, move |seq, rec| {
            let this = self;
            let llm_ref = llm_ref_owned.clone();
            let file_strings = file_strings.clone();
            async move {
                let handle = this
                    .resolve_ref_to_object(&rec, seq)
                    .await
                    .map_err(classify_browser_err)?;

                // file input 的 actionability：visible + enabled（不检 stable/editable——文件框不走 fill）。
                let cr = this
                    .check_states(&handle, &["visible", "enabled"])
                    .await
                    .map_err(classify_browser_err)?;
                gate_check_result(cr)?;

                // DOM.setFileInputFiles：把路径直接塞进 input.files（绕系统文件框）。元素非 file input →
                // CDP 报错 → Fatal（禁重试，引导换 ref）；detach → 可重试。
                this.set_file_input_files(&rec.session_id, &handle.object_id, &file_strings)
                    .await
                    .map_err(|e| match e {
                        BrowserError::NotConnected => {
                            RetryDecision::Retryable(BrowserError::NotConnected)
                        }
                        // 元素不是 file input / 其它 CDP 错误：Fatal（禁重试，文案引导换 ref）。
                        other => RetryDecision::Fatal(other),
                    })?;

                // verify：读回 input.files.length + 首文件名（证真设进去了）。best-effort（读不到不致命）。
                let after = this.act_read_file_input(&handle.object_id).await;
                let count = after
                    .as_ref()
                    .and_then(|v| v.get("count"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let changed = count > 0;
                let message = format!(
                    "set {} file(s) on {llm_ref} (file input now holds {count}); re-observe to confirm",
                    file_strings.len()
                );
                Ok(ActResult {
                    message,
                    effect: Effect {
                        changed,
                        // 前锚点：上传前文件框一般为空（不读前值，省一次往返）。后锚点 = files 摘要
                        // （count + 首文件名；文件名是用户给的本地路径片段，非页面机密，可入锚点）。
                        before_anchor: None,
                        after_anchor: after,
                    },
                    // CDP setFileInputFiles 没报错即视作成功（count==0 极罕见——某些受控组件异步反映
                    // files；仍 success=true 但 changed 据 count 判，文案引导 re-observe）。
                    success: true,
                })
            }
        })
        .await
    }

    /// **Download 分支**（F-actions，DESIGN §9 / 裁决⑩，复用 E4 沙箱）：触发下载某 `url`，落进 E4 的
    /// **隔离 downloads 目录**（per-pet workspace/downloads）——denylist 红线（可执行/脚本下载在
    /// downloadWillBegin 被取消）+ Win MOTW（落盘后 `Zone.Identifier` ADS）+ **不自动打开**，全链复用 E4。
    ///
    /// **触发路径（选项 A，不扰当前页）**：注入隐藏 `<a href=url download>` 并 click（浏览器把它当用户
    /// 发起的下载，走沙箱事件循环）。**不**用 `Page.navigate(url)`（那会动当前页）。可执行 url 下载经 E4
    /// 在 downloadWillBegin 阶段 cancelDownload 拒（红线，yolo/companion 也拒）。
    ///
    /// **verify（落盘探测）**：触发后**轮询隔离 downloads 目录**至出现新增文件（size>0）或短超时；超时 →
    /// success=false 如实（被红线取消 / url 无附件 / 仍在传），**非报错**（良性态）。`download_dir` 为
    /// `None`（无沙箱）→ 仍触发，但无落点探测 → success=false 如实。
    pub async fn act_download(&self, url: &str, parent: &Progress) -> Result<ActResult, BrowserError> {
        let url_owned = url.to_string();
        let download_dir = self.download_dir().map(|s| s.to_string());
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            let url = url_owned.clone();
            let download_dir = download_dir.clone();
            async move {
                // 落盘探测基线：触发前 downloads 目录已有文件集（探测「新增」，避免把旧下载误判为本次）。
                let before_set = download_dir
                    .as_deref()
                    .map(crate::backend::cdp::list_dir_files)
                    .unwrap_or_default();

                // 选项 A：注入隐藏 `<a download>` + click 触发（走 E4 downloadWillBegin/Progress 沙箱）。
                this.trigger_anchor_download(&url)
                    .await
                    .map_err(classify_browser_err)?;

                // 轮询隔离 downloads 目录至出现**新增**文件（size>0）或短 deadline；无 download_dir → 不探测。
                let landed = match download_dir.as_deref() {
                    Some(dir) => Some(this.poll_download_landed(dir, &before_set).await),
                    None => None,
                };

                let (message, changed, after) = match landed {
                    Some(Some((name, size))) => (
                        format!(
                            "download of {url:?} landed in the sandboxed downloads directory as {name:?} ({size} bytes); \
                             it was NOT opened. re-observe if the page changed."
                        ),
                        true,
                        Some(serde_json::json!({ "downloaded_file": name, "bytes": size })),
                    ),
                    Some(None) => (
                        format!(
                            "triggered a download of {url:?} but no new file appeared in the sandboxed downloads \
                             directory within the wait window (it may have been blocked as an executable red-line, \
                             may not be an attachment, or may still be in flight)"
                        ),
                        false,
                        None,
                    ),
                    None => (
                        format!(
                            "triggered a download of {url:?} (no sandboxed downloads directory is configured for this \
                             session, so the landing could not be verified)"
                        ),
                        false,
                        None,
                    ),
                };
                Ok(ActResult {
                    message,
                    effect: Effect {
                        changed,
                        before_anchor: None,
                        after_anchor: after,
                    },
                    // 落盘成功 → success=true；触发但未落盘 / 无沙箱 → success=false 如实（良性，非报错）。
                    success: changed,
                })
            }
        })
        .await
    }

    /// **SaveAsPdf 分支**（F-actions，DESIGN §9）：把当前页另存为 PDF——`Page.printToPDF` 取 PDF 字节 →
    /// 写进 E4 的**隔离 downloads 目录**（与 download 同落点，复用沙箱隔离）→ 返路径。verify：文件 size>0。
    ///
    /// **headless vs headful**：`Page.printToPDF` 在 **headless** 可靠；headful Chrome 历史上曾有限制。
    /// **已实测（2026-06-20, Chrome 149 PINNED, macOS headful 真窗口）：headful 下 printToPDF 正常产
    /// 非空 PDF**（`integration_factions::save_as_pdf_headful_writes_pdf_or_reports_cleanly`，46KB 真
    /// `%PDF`）——现代 Chrome 无此限制。若某版本仍受限，CDP 回 error → 经 map_transport_err 成 `Other`、
    /// success=false 如实（非 panic、不写半截文件），契约不变。`download_dir` 为 `None`（无沙箱）→
    /// `Unsupported`（无落点）。
    pub async fn act_save_as_pdf(&self, parent: &Progress) -> Result<ActResult, BrowserError> {
        let download_dir = self.download_dir().map(|s| s.to_string());
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            let download_dir = download_dir.clone();
            async move {
                // 无沙箱落点 → Unsupported（PDF 无处落；讲清需 per-pet workspace 配置）。
                let Some(dir) = download_dir.clone() else {
                    return Err(RetryDecision::Fatal(BrowserError::Unsupported {
                        capability: "save_as_pdf".into(),
                        hint: "no sandboxed downloads directory is configured for this session; \
                               save_as_pdf needs a per-pet workspace to write the PDF into".into(),
                    }));
                };

                // Page.printToPDF（默认参数 + print_background）。headful 已实测可用；若某版本受限 → Err（Fatal）。
                let pdf_bytes = this.print_to_pdf().await.map_err(RetryDecision::Fatal)?;
                if pdf_bytes.is_empty() {
                    return Err(RetryDecision::Fatal(BrowserError::Other(
                        "Page.printToPDF returned empty PDF data".into(),
                    )));
                }

                // 写进隔离 downloads 目录（与 download 同落点）。文件名带时间戳防覆盖。best-effort mkdir。
                let path = crate::backend::cdp::pdf_output_path(&dir);
                if let Some(parent_dir) = path.parent() {
                    let _ = std::fs::create_dir_all(parent_dir);
                }
                std::fs::write(&path, &pdf_bytes).map_err(|e| {
                    RetryDecision::Fatal(BrowserError::Other(format!(
                        "failed to write the PDF to the sandboxed downloads directory: {e}"
                    )))
                })?;
                let size = pdf_bytes.len() as u64;
                let path_str = path.to_string_lossy().into_owned();
                Ok(ActResult {
                    message: format!(
                        "saved the current page as a PDF to {path_str:?} ({size} bytes) in the sandboxed \
                         downloads directory; it was NOT opened"
                    ),
                    effect: Effect {
                        changed: true,
                        before_anchor: None,
                        after_anchor: Some(serde_json::json!({ "pdf_path": path_str, "bytes": size })),
                    },
                    success: true,
                })
            }
        })
        .await
    }

    /// **Extract 分支**（F-actions，DESIGN §9，**deterministic plumbing**）：按 `schema` 请求的字段，
    /// 返回当前页的**结构化表示**供上层 LLM 抽取——P2 给确定性的页面捕获（aria snapshot + 可见文本），
    /// **不在引擎内塞 LLM 调用**（P2 引擎无 LLM；真 LLM-driven 字段抽取需 nomi 集成 = P3）。
    ///
    /// 捕获两路（都经 redact + `<data>` wrap，喂 LLM 的**不可信内容**，镜像 observe/get_page_text 安全契约）：
    /// 1. **aria snapshot YAML**（[`CdpBackend::observe_impl`]，已 redact + `<data>`-wrap）——结构化可交互树。
    /// 2. **可见页面文本**（[`Self::act_extract_page_text`] → redact → wrap）——补充正文内容。
    ///
    /// `schema` 是 agent 给的 JSON schema（要抽哪些字段）：P2 把它**回显**作「请求字段」提示，连同上面
    /// 两路结构化表示一起返回，供上层（P3 nomi 集成）据 schema 做真正的字段抽取。**脱敏铁律**：返回内容
    /// 已 redact（页面密文不进输出）+ `<data>` wrap（防提示注入越狱）——LLM 永不见明文 secret。
    ///
    /// **TODO(P3): LLM-driven extraction** —— P3 在 nomi 集成层把本 deterministic 表示 + schema 喂给
    /// LLM 产出结构化 JSON（引擎层仍不持有 LLM；抽取在上层）。
    ///
    /// 只读零写（Info 级）。页面级动作，走简化骨架（abort+retry，无 ref）。
    pub async fn act_extract(
        &self,
        schema: &serde_json::Value,
        parent: &Progress,
    ) -> Result<ActResult, BrowserError> {
        let schema_owned = schema.clone();
        self.act_page_with_skeleton(parent, move |_attempt| {
            let this = self;
            let schema = schema_owned.clone();
            async move {
                // ① aria snapshot（已 redact + <data>-wrap）：结构化可交互树。
                let obs = this
                    .observe_impl(&crate::engine::ObserveOpts::default())
                    .await
                    .map_err(classify_browser_err)?;
                // ② 可见页面文本（redact + wrap，与 get_page_text 同契约：不可信内容）。
                let raw_text = this
                    .act_extract_page_text()
                    .await
                    .map_err(classify_browser_err)?;
                let url = this.act_current_url().await;
                let redacted_text = crate::redact::redact_yaml(&raw_text);
                let wrapped_text = crate::redact::wrap_untrusted(&redacted_text, url.as_deref());

                // 回显 schema 请求字段（紧凑 JSON）作「请求字段」提示——P2 deterministic plumbing：把结构化
                // 表示 + schema 一起返回供上层 LLM 抽取（P3）。schema 是 agent 给的请求规格、非页面内容，不脱敏。
                let schema_compact = serde_json::to_string(&schema).unwrap_or_else(|_| "{}".into());
                let message = format!(
                    "structured page representation for extraction (untrusted, redacted). \
                     Requested schema: {schema_compact}\n\n\
                     [accessibility snapshot]\n{}\n\n\
                     [visible text]\n{wrapped_text}\n\n\
                     NOTE: P2 returns a deterministic page representation; perform the field \
                     extraction against the `schema` above. (TODO(P3): LLM-driven extraction wired \
                     in the nomi-integration layer — the engine holds no LLM.)",
                    obs.yaml
                );
                Ok(ActResult {
                    // 只读动作：changed=false，内容在 message。
                    message,
                    effect: Effect {
                        changed: false,
                        before_anchor: None,
                        after_anchor: None,
                    },
                    success: true,
                })
            }
        })
        .await
    }
}

/// **[纯逻辑] 人读的 WaitCondition 描述**（wait_for 成功文案）。
fn describe_wait_condition(condition: &WaitCondition) -> String {
    match condition {
        WaitCondition::UrlContains { text } => format!("URL contains {text:?}"),
        WaitCondition::TextVisible { text } => format!("text {text:?} is visible"),
        WaitCondition::RefActionable { r#ref } => format!("{ref} is actionable"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SD-2 上传路径沙箱：纯逻辑校验，无浏览器依赖。
// ═══════════════════════════════════════════════════════════════════════

/// **Well-known sensitive root prefixes** — belt-and-suspenders deny even if somehow under workspace.
///
/// 平台感知（防御纵深在 Windows 上**不静默失效**）：POSIX 系统根在 `#[cfg(unix)]`；Windows 在
/// `#[cfg(windows)]` 下用 Windows 目录（`%SystemRoot%`/`%windir%`，否则兜底 `C:\Windows`）。
/// 这些字符串经 `canonicalize` 后用 [`std::path::Path::starts_with`] 做**组件级**比较（分隔符无关、
/// 大小写由 canonicalize 在 Windows 上归一处理）。
fn sensitive_roots() -> Vec<std::path::PathBuf> {
    #[cfg(unix)]
    {
        ["/etc", "/private/etc", "/proc", "/sys"]
            .iter()
            .map(std::path::PathBuf::from)
            .collect()
    }
    #[cfg(windows)]
    {
        let windir = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("windir"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
        vec![windir]
    }
    #[cfg(not(any(unix, windows)))]
    {
        Vec::new()
    }
}

/// **SD-2 上传路径沙箱校验**：对单条路径做 canonicalize + workspace 包含判定 + 敏感根拒绝。
///
/// 返回 `Ok(())` 当且仅当：
/// 1. `workspace` 不为 `None`（fail-closed：无沙箱根 ⇒ 拒绝所有上传）。
/// 2. `path` 能 `canonicalize` 成功（文件存在 + 路径合法；解析 symlink / `..`）。
/// 3. canonicalized path **以** canonicalized workspace 为前缀（在 workspace 内）。
/// 4. canonicalized path **不在** well-known 敏感根下（belt-and-suspenders）。
///
/// **跨平台不变量**：包含判定全程用 [`std::path::Path::starts_with`]（**组件级**比较），而非比较
/// canonical **字符串** + 硬编码 `b'/'` 分隔符字节边界。Windows `canonicalize` 返回 verbatim
/// `\\?\C:\...`（反斜杠分隔符），旧的 `/` 字节边界判定永不命中 → in-workspace 文件被误判逃逸。
/// 因 workspace 与 path **两侧同样 canonicalize**，`\\?\` 前缀与分隔符一致，`starts_with` 组件级
/// 比较在三平台都正确，且 symlink/`..` 解析后逃逸仍返 false（安全不变量保持）。
///
/// 失败 ⇒ `BrowserError::Blocked { reason }`（不日志文件内容，仅路径字符串）。
pub(crate) fn validate_upload_path(
    path: &std::path::Path,
    workspace: Option<&std::path::Path>,
) -> Result<(), BrowserError> {
    // 1. No workspace configured → fail-closed.
    let workspace = workspace.ok_or_else(|| BrowserError::Blocked {
        reason: "upload denied: no workspace_dir configured (fail-closed)".into(),
    })?;

    // 2. Canonicalize workspace root (must succeed; if workspace itself is invalid, deny).
    let canon_workspace = std::fs::canonicalize(workspace).map_err(|e| BrowserError::Blocked {
        reason: format!(
            "upload denied: cannot canonicalize workspace {}: {e}",
            workspace.display()
        ),
    })?;

    // 3. Canonicalize the upload path (resolves symlinks + `..`; fails if file doesn't exist).
    let canon_path = std::fs::canonicalize(path).map_err(|e| BrowserError::Blocked {
        reason: format!(
            "upload denied: cannot canonicalize path {}: {e}",
            path.display()
        ),
    })?;

    // 4. Belt-and-suspenders: deny well-known sensitive roots even if under workspace.
    //    组件级 `Path::starts_with`（分隔符 / `\\?\` 前缀无关；两侧都 canonical）。敏感根字符串
    //    本身先 canonicalize（存在才比；不存在的根无从逃逸，跳过即可——同时归一 `\\?\`/大小写）。
    for root in sensitive_roots() {
        if let Ok(canon_root) = std::fs::canonicalize(&root) {
            if canon_path.starts_with(&canon_root) {
                return Err(BrowserError::Blocked {
                    reason: format!(
                        "upload denied: path resolves to sensitive root ({})",
                        root.display()
                    ),
                });
            }
        }
    }

    // Also deny ~/.ssh (expand ~ to home dir). 组件级 starts_with（同上）。
    let home_dir = {
        #[cfg(unix)]
        { std::env::var("HOME").ok().map(PathBuf::from) }
        #[cfg(windows)]
        { std::env::var("USERPROFILE").ok().map(PathBuf::from) }
        #[cfg(not(any(unix, windows)))]
        { None::<PathBuf> }
    };
    if let Some(home) = home_dir {
        let ssh_dir = home.join(".ssh");
        if let Ok(canon_ssh) = std::fs::canonicalize(&ssh_dir) {
            if canon_path.starts_with(&canon_ssh) {
                return Err(BrowserError::Blocked {
                    reason: "upload denied: path resolves under ~/.ssh".into(),
                });
            }
        }
    }

    // 5. Workspace containment check: canonical path must start with canonical workspace
    //    (组件级 `Path::starts_with`——分隔符 / verbatim `\\?\` 前缀无关，两侧同样 canonical)。
    if !canon_path.starts_with(&canon_workspace) {
        return Err(BrowserError::Blocked {
            reason: format!(
                "upload denied: path escapes workspace (workspace={}, resolved={})",
                workspace.display(),
                canon_path.display()
            ),
        });
    }

    Ok(())
}

/// **SD-2 批量校验**：对一组路径逐个调 [`validate_upload_path`]，首个失败即短路返回。
pub(crate) fn validate_upload_paths(
    paths: &[PathBuf],
    workspace: Option<&std::path::Path>,
) -> Result<(), BrowserError> {
    for p in paths {
        validate_upload_path(p, workspace)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ═══════════════════════════════════════════════════════════════════════
    // SD-2 上传路径沙箱纯逻辑单测（无浏览器；使用 tempdir 构造 workspace 场景）。
    // ═══════════════════════════════════════════════════════════════════════

    /// workspace_dir = None → fail-closed（拒绝所有上传）。
    #[test]
    fn upload_rejects_when_no_workspace_configured() {
        let tmp = std::env::temp_dir().join("nomifun-sd2-test-noworkspace.txt");
        std::fs::write(&tmp, b"test").unwrap();
        let result = validate_upload_path(&tmp, None);
        let _ = std::fs::remove_file(&tmp);
        assert!(
            matches!(&result, Err(BrowserError::Blocked { reason }) if reason.contains("no workspace_dir")),
            "expected Blocked with 'no workspace_dir', got {result:?}"
        );
    }

    /// Path outside workspace (e.g. /etc/passwd or any file not under workspace) → Blocked.
    #[test]
    fn upload_rejects_path_outside_workspace() {
        // Create a workspace dir and a file outside of it.
        let workspace = std::env::temp_dir().join("nomifun-sd2-workspace-a");
        std::fs::create_dir_all(&workspace).unwrap();
        let outside = std::env::temp_dir().join("nomifun-sd2-outside-file.txt");
        std::fs::write(&outside, b"outside").unwrap();

        let result = validate_upload_path(&outside, Some(&workspace));
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&workspace);
        assert!(
            matches!(&result, Err(BrowserError::Blocked { reason }) if reason.contains("escapes workspace")),
            "expected Blocked with 'escapes workspace', got {result:?}"
        );
    }

    /// Path inside workspace → Ok.
    #[test]
    fn upload_allows_path_inside_workspace() {
        let workspace = std::env::temp_dir().join("nomifun-sd2-workspace-b");
        std::fs::create_dir_all(&workspace).unwrap();
        let inside = workspace.join("allowed-file.txt");
        std::fs::write(&inside, b"inside").unwrap();

        let result = validate_upload_path(&inside, Some(&workspace));
        let _ = std::fs::remove_file(&inside);
        let _ = std::fs::remove_dir_all(&workspace);
        assert!(result.is_ok(), "expected Ok for in-workspace path, got {result:?}");
    }

    /// Symlink escaping workspace → Blocked (canonicalize resolves it).
    #[test]
    fn upload_rejects_symlink_escaping_workspace() {
        let workspace = std::env::temp_dir().join("nomifun-sd2-workspace-c");
        std::fs::create_dir_all(&workspace).unwrap();
        // Create a file outside workspace.
        let outside = std::env::temp_dir().join("nomifun-sd2-symlink-target.txt");
        std::fs::write(&outside, b"secret").unwrap();
        // Create symlink inside workspace pointing outside.
        let link_path = workspace.join("sneaky-link.txt");
        let _ = std::fs::remove_file(&link_path); // cleanup prior run
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link_path).unwrap();
        #[cfg(windows)]
        {
            // Windows symlink 需 SeCreateSymbolicLinkPrivilege（开发者模式或管理员）；缺权时
            // `symlink_file` 返回 PermissionDenied（os error 1314）。优雅跳过而非 unwrap panic——
            // canonicalize-then-`Path::starts_with` 的安全不变量本身与是否在 CI 能建 symlink 无关。
            if let Err(e) = std::os::windows::fs::symlink_file(&outside, &link_path) {
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(1314)
                {
                    eprintln!(
                        "skipping upload_rejects_symlink_escaping_workspace: \
                         cannot create symlink without SeCreateSymbolicLinkPrivilege \
                         (enable Developer Mode): {e}"
                    );
                    let _ = std::fs::remove_file(&outside);
                    let _ = std::fs::remove_dir_all(&workspace);
                    return;
                }
                // 其它 IO 错误属真失败。
                panic!("unexpected error creating symlink: {e}");
            }
        }

        let result = validate_upload_path(&link_path, Some(&workspace));
        let _ = std::fs::remove_file(&link_path);
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&workspace);
        assert!(
            matches!(&result, Err(BrowserError::Blocked { reason }) if reason.contains("escapes workspace")),
            "expected Blocked for symlink escape, got {result:?}"
        );
    }

    /// Path traversal with `..` that escapes → Blocked.
    #[test]
    fn upload_rejects_dotdot_traversal() {
        let workspace = std::env::temp_dir().join("nomifun-sd2-workspace-d");
        let sub = workspace.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        // Create file outside workspace.
        let outside = std::env::temp_dir().join("nomifun-sd2-dotdot-target.txt");
        std::fs::write(&outside, b"secret").unwrap();

        // Construct a path that uses `..` to escape: workspace/sub/../../nomifun-sd2-dotdot-target.txt
        let traversal = sub.join("..").join("..").join("nomifun-sd2-dotdot-target.txt");
        let result = validate_upload_path(&traversal, Some(&workspace));
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&workspace);
        assert!(
            matches!(&result, Err(BrowserError::Blocked { reason }) if reason.contains("escapes workspace")),
            "expected Blocked for .. traversal, got {result:?}"
        );
    }

    /// Non-existent file → Blocked (canonicalize fails).
    #[test]
    fn upload_rejects_nonexistent_file() {
        let workspace = std::env::temp_dir().join("nomifun-sd2-workspace-e");
        std::fs::create_dir_all(&workspace).unwrap();
        let ghost = workspace.join("does-not-exist.txt");

        let result = validate_upload_path(&ghost, Some(&workspace));
        let _ = std::fs::remove_dir_all(&workspace);
        assert!(
            matches!(&result, Err(BrowserError::Blocked { reason }) if reason.contains("cannot canonicalize path")),
            "expected Blocked for nonexistent file, got {result:?}"
        );
    }

    /// validate_upload_paths batch: first bad path short-circuits.
    #[test]
    fn upload_batch_rejects_on_first_bad_path() {
        let workspace = std::env::temp_dir().join("nomifun-sd2-workspace-f");
        std::fs::create_dir_all(&workspace).unwrap();
        let good = workspace.join("good.txt");
        std::fs::write(&good, b"ok").unwrap();
        let outside = std::env::temp_dir().join("nomifun-sd2-batch-outside.txt");
        std::fs::write(&outside, b"bad").unwrap();

        let result = validate_upload_paths(
            &[good.clone(), outside.clone()],
            Some(&workspace),
        );
        let _ = std::fs::remove_file(&good);
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&workspace);
        assert!(
            matches!(&result, Err(BrowserError::Blocked { .. })),
            "expected Blocked for batch with outside path, got {result:?}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // B6 重试编排纯逻辑单测（无浏览器；虚拟时钟 start_paused 确定性推进退避）。
    // op 是构造的闭包（按 attempt 序号返预设裁决），断言：尝试次数、退避累计 sleep、
    // IRREVERSIBLE 只试一次、Fatal 立返、退避耗尽错误。
    // ═══════════════════════════════════════════════════════════════════════

    use std::cell::Cell;
    use std::rc::Rc;
    use tokio::time::Duration as TDuration;

    /// 退避序列正确：op 连续返 Retryable N 次后第 N 次成功 → 尝试数 == N+1，
    /// 总累计 sleep == BACKOFF 前 (N+1) 项和（虚拟时钟自动推进到每个 timer，elapsed 即累计 sleep）。
    #[tokio::test(start_paused = true)]
    async fn backoff_sequence_retries_then_succeeds() {
        // 前 3 次（attempt 0,1,2）返 Retryable，第 4 次（attempt 3）成功。
        let fail_until = 3usize;
        let calls = Rc::new(Cell::new(0usize));
        let calls2 = calls.clone();
        // 大 deadline：不让 timeout 干扰退避测试（退避总和远小于此）。
        let p = Progress::new(Duration::from_secs(3600));

        let start = tokio::time::Instant::now();
        let out: Result<u32, BrowserError> = run_act_with_retry(&p, false, move |attempt| {
            let calls = calls2.clone();
            async move {
                calls.set(calls.get() + 1);
                if attempt < fail_until {
                    Err(RetryDecision::Retryable(BrowserError::NotConnected))
                } else {
                    Ok(42u32)
                }
            }
        })
        .await;
        let elapsed = start.elapsed();

        assert!(matches!(out, Ok(42)), "expected Ok(42), got {out:?}");
        // attempt 0..=3 共 4 次 op 调用。
        assert_eq!(calls.get(), 4, "op must be called once per attempt (4 total)");
        // 累计 sleep = BACKOFF[0..=3] 之和 = 0+20+50+100 = 170ms（第 0 次 delay==0 不 sleep）。
        let expected: u64 = BACKOFF[0..=3].iter().sum();
        assert_eq!(expected, 170, "backoff prefix sanity");
        // 虚拟时钟自动推进到每个 sleep timer，故 elapsed == 累计退避 sleep（误差容忍 1ms 取整）。
        assert_eq!(
            elapsed.as_millis() as u64,
            expected,
            "total elapsed must equal sum of BACKOFF[0..=3] sleeps, got {elapsed:?}"
        );
    }

    /// 第 0 次即成功：op 只调用一次，零退避（elapsed == 0）。
    #[tokio::test(start_paused = true)]
    async fn first_attempt_success_no_backoff() {
        let calls = Rc::new(Cell::new(0usize));
        let calls2 = calls.clone();
        let p = Progress::new(Duration::from_secs(3600));
        let start = tokio::time::Instant::now();
        let out: Result<&str, BrowserError> = run_act_with_retry(&p, false, move |_attempt| {
            let calls = calls2.clone();
            async move {
                calls.set(calls.get() + 1);
                Ok("ok")
            }
        })
        .await;
        assert_eq!(out.unwrap(), "ok");
        assert_eq!(calls.get(), 1, "success on attempt 0 → exactly one op call");
        assert_eq!(start.elapsed().as_millis(), 0, "no backoff before first attempt");
    }

    /// **IRREVERSIBLE 禁重试**：irreversible=true + op 恒返 Retryable → op **只调用一次**（attempt==1），
    /// 立返该 Retryable 的载荷错误（绝不进退避循环）。
    #[tokio::test(start_paused = true)]
    async fn irreversible_only_attempts_once() {
        let calls = Rc::new(Cell::new(0usize));
        let calls2 = calls.clone();
        let p = Progress::new(Duration::from_secs(3600));
        let start = tokio::time::Instant::now();
        let out: Result<u32, BrowserError> = run_act_with_retry(&p, true, move |_attempt| {
            let calls = calls2.clone();
            async move {
                calls.set(calls.get() + 1);
                // 即便返「可重试」，不可逆动作也绝不重试。
                Err(RetryDecision::Retryable(BrowserError::NotConnected))
            }
        })
        .await;
        // 关键断言：op 计数 == 1（只试一次）。
        assert_eq!(calls.get(), 1, "IRREVERSIBLE must call op exactly once (no retry)");
        // 零退避（没有第二次尝试 → 没有 sleep）。
        assert_eq!(start.elapsed().as_millis(), 0, "IRREVERSIBLE must not back off");
        // 上抛的是 Retryable 的载荷错误（保留原分类）。
        assert!(
            matches!(out, Err(BrowserError::NotConnected)),
            "IRREVERSIBLE retryable failure must surface its payload error, got {out:?}"
        );
    }

    /// IRREVERSIBLE + 第 0 次即成功：仍然只试一次，正常返回（不可逆且成功是常态）。
    #[tokio::test(start_paused = true)]
    async fn irreversible_success_first_try() {
        let calls = Rc::new(Cell::new(0usize));
        let calls2 = calls.clone();
        let p = Progress::new(Duration::from_secs(3600));
        let out: Result<u32, BrowserError> = run_act_with_retry(&p, true, move |_attempt| {
            let calls = calls2.clone();
            async move {
                calls.set(calls.get() + 1);
                Ok(7u32)
            }
        })
        .await;
        assert_eq!(out.unwrap(), 7);
        assert_eq!(calls.get(), 1, "IRREVERSIBLE success still calls op once");
    }

    /// Fatal 立返不重试：op 第 0 次返 Fatal(Blocked) → op 只调用一次，立返该错误（即便不是不可逆）。
    #[tokio::test(start_paused = true)]
    async fn fatal_returns_immediately_without_retry() {
        let calls = Rc::new(Cell::new(0usize));
        let calls2 = calls.clone();
        let p = Progress::new(Duration::from_secs(3600));
        let start = tokio::time::Instant::now();
        let out: Result<u32, BrowserError> = run_act_with_retry(&p, false, move |_attempt| {
            let calls = calls2.clone();
            async move {
                calls.set(calls.get() + 1);
                Err(RetryDecision::Fatal(BrowserError::Blocked {
                    reason: "non-editable".into(),
                }))
            }
        })
        .await;
        assert_eq!(calls.get(), 1, "Fatal must not retry (one op call)");
        assert_eq!(start.elapsed().as_millis(), 0, "Fatal returns before any backoff");
        match out {
            Err(BrowserError::Blocked { reason }) => assert_eq!(reason, "non-editable"),
            other => panic!("expected Blocked(non-editable), got {other:?}"),
        }
    }

    /// Fatal 在第 2 次尝试出现：前两次 Retryable 后退避，第 3 次返 Fatal → 立返，不再退避。
    /// 验证 Fatal 在循环中段也立返（而非继续退避到耗尽）。
    #[tokio::test(start_paused = true)]
    async fn fatal_mid_loop_returns_immediately() {
        let calls = Rc::new(Cell::new(0usize));
        let calls2 = calls.clone();
        let p = Progress::new(Duration::from_secs(3600));
        let start = tokio::time::Instant::now();
        let out: Result<u32, BrowserError> = run_act_with_retry(&p, false, move |attempt| {
            let calls = calls2.clone();
            async move {
                calls.set(calls.get() + 1);
                if attempt < 2 {
                    Err(RetryDecision::Retryable(BrowserError::NotConnected))
                } else {
                    Err(RetryDecision::Fatal(BrowserError::NodeStale { generation: 5 }))
                }
            }
        })
        .await;
        // attempt 0,1 (Retryable) + attempt 2 (Fatal) = 3 次 op 调用。
        assert_eq!(calls.get(), 3, "two retries then a Fatal → three op calls");
        // 退避只在 attempt 1,2 之前发生：BACKOFF[1]+BACKOFF[2] = 20+50 = 70ms。
        assert_eq!(
            start.elapsed().as_millis() as u64,
            BACKOFF[1] + BACKOFF[2],
            "backoff only for attempts 1 and 2"
        );
        assert!(matches!(out, Err(BrowserError::NodeStale { generation: 5 })));
    }

    /// 退避耗尽（op 恒返 Retryable，超 6 槽仍不成功）→ op 调用 BACKOFF.len() 次后上抛最后一次瞬态错误。
    #[tokio::test(start_paused = true)]
    async fn backoff_exhausted_surfaces_last_retryable() {
        let calls = Rc::new(Cell::new(0usize));
        let calls2 = calls.clone();
        let p = Progress::new(Duration::from_secs(3600));
        let start = tokio::time::Instant::now();
        let out: Result<u32, BrowserError> = run_act_with_retry(&p, false, move |_attempt| {
            let calls = calls2.clone();
            async move {
                calls.set(calls.get() + 1);
                Err(RetryDecision::Retryable(BrowserError::NotConnected))
            }
        })
        .await;
        // 共 BACKOFF.len() == 6 次 op 调用（首次 + 5 次退避后重试），之后耗尽。
        assert_eq!(calls.get(), BACKOFF.len(), "exhaust all backoff slots");
        // 累计 sleep = 全部 BACKOFF 之和 = 0+20+50+100+100+500 = 770ms。
        let total: u64 = BACKOFF.iter().sum();
        assert_eq!(total, 770, "full backoff sum sanity");
        assert_eq!(
            start.elapsed().as_millis() as u64,
            total,
            "exhausted retry must have slept the full backoff series"
        );
        // 上抛最后一次瞬态分类（保留语义）。
        assert!(
            matches!(out, Err(BrowserError::NotConnected)),
            "exhausted retry surfaces last Retryable payload, got {out:?}"
        );
    }

    /// **deadline 先到 → Timeout{Action}**：op 永挂（pending），Progress 短 deadline → race 在第 0 次
    /// 尝试就 timeout，经 map_progress_err 成 `Timeout{phase:Action}`（动作层超时语义）。
    #[tokio::test(start_paused = true)]
    async fn deadline_during_attempt_maps_to_action_timeout() {
        use crate::engine::NavPhase;
        use std::future::pending;
        let p = std::sync::Arc::new(Progress::new(Duration::from_millis(30)));
        let p2 = p.clone();
        let handle = tokio::spawn(async move {
            run_act_with_retry::<_, _, ()>(&p2, false, |_attempt| async { pending().await }).await
        });
        // 推进虚拟时钟越过 deadline。
        tokio::time::advance(Duration::from_millis(31)).await;
        let out = handle.await.expect("task panicked");
        assert!(
            matches!(out, Err(BrowserError::Timeout { phase: NavPhase::Action })),
            "deadline during attempt must map to Timeout{{Action}}, got {out:?}"
        );
    }

    /// **进行中 abort（PageClosed）→ TargetClosed**：op 永挂，另一处 progress.abort(PageClosed) →
    /// race 立即（远早于大 deadline）以 Aborted 返回，经 map_progress_err 成 `TargetClosed`。
    /// 这是 detach/crash 事件源接线在重试编排里的语义验证（事件源真接线在 cdp.rs + 集成测试）。
    #[tokio::test]
    async fn in_flight_page_closed_abort_maps_to_target_closed() {
        use std::future::pending;
        let p = std::sync::Arc::new(Progress::new(Duration::from_secs(3600)));
        let p2 = p.clone();
        let race = tokio::spawn(async move {
            run_act_with_retry::<_, _, ()>(&p2, false, |_attempt| async { pending().await }).await
        });
        // 让 op 进入等待，再在另一处 abort（模拟 detach/crash 事件源）。
        tokio::task::yield_now().await;
        p.abort(crate::progress::AbortReason::PageClosed);
        // 必须远早于 3600s deadline 返回（用保守上限断言「立即」）。
        let out = tokio::time::timeout(TDuration::from_secs(5), race)
            .await
            .expect("retry did not return promptly after abort")
            .expect("task panicked");
        assert!(
            matches!(out, Err(BrowserError::TargetClosed)),
            "PageClosed abort must map to TargetClosed, got {out:?}"
        );
    }

    /// **进行中 abort（FrameDetached）→ Detached{Frame}**：同上，但 abort 原因是帧 detach。
    #[tokio::test]
    async fn in_flight_frame_detached_abort_maps_to_detached() {
        use crate::engine::DetachKind;
        use std::future::pending;
        let p = std::sync::Arc::new(Progress::new(Duration::from_secs(3600)));
        let p2 = p.clone();
        let race = tokio::spawn(async move {
            run_act_with_retry::<_, _, ()>(&p2, false, |_attempt| async { pending().await }).await
        });
        tokio::task::yield_now().await;
        p.abort(crate::progress::AbortReason::FrameDetached);
        let out = tokio::time::timeout(TDuration::from_secs(5), race)
            .await
            .expect("retry did not return promptly after abort")
            .expect("task panicked");
        assert!(
            matches!(out, Err(BrowserError::Detached { kind: DetachKind::Frame })),
            "FrameDetached abort must map to Detached{{Frame}}, got {out:?}"
        );
    }

    /// **退避期间 abort 也立即打断**：op 第 0 次返 Retryable 进入退避 sleep，退避中 abort → 不白等，
    /// 立返 Aborted 映射的错误。验证退避 sleep 也跑在 progress.race 上（page.close 不应等退避走完）。
    #[tokio::test]
    async fn abort_during_backoff_interrupts_immediately() {
        let p = std::sync::Arc::new(Progress::new(Duration::from_secs(3600)));
        let p2 = p.clone();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls2 = calls.clone();
        let race = tokio::spawn(async move {
            run_act_with_retry::<_, _, ()>(&p2, false, move |_attempt| {
                let calls = calls2.clone();
                async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    // 永远返 Retryable → 第 0 次后进入退避（BACKOFF[1]=20ms 真实时钟）。
                    Err(RetryDecision::Retryable(BrowserError::NotConnected))
                }
            })
            .await
        });
        // 等第 0 次 op 跑完进入退避后再 abort（让 op 至少被调用一次）。
        tokio::time::sleep(TDuration::from_millis(5)).await;
        p.abort(crate::progress::AbortReason::PageClosed);
        let out = tokio::time::timeout(TDuration::from_secs(5), race)
            .await
            .expect("retry did not return promptly when aborted during backoff")
            .expect("task panicked");
        assert!(
            matches!(out, Err(BrowserError::TargetClosed)),
            "abort during backoff must interrupt and map to TargetClosed, got {out:?}"
        );
    }

    /// attempt 序号正确递增（0,1,2,…）传给 op：用它做按尝试调整策略（如 scroll alignment 轮转）。
    #[tokio::test(start_paused = true)]
    async fn attempt_index_increments_per_call() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::<usize>::new()));
        let seen2 = seen.clone();
        let p = Progress::new(Duration::from_secs(3600));
        let _out: Result<u32, BrowserError> = run_act_with_retry(&p, false, move |attempt| {
            let seen = seen2.clone();
            async move {
                seen.borrow_mut().push(attempt);
                if attempt < 2 {
                    Err(RetryDecision::Retryable(BrowserError::NotConnected))
                } else {
                    Ok(0u32)
                }
            }
        })
        .await;
        assert_eq!(*seen.borrow(), vec![0, 1, 2], "attempt index must be 0,1,2,...");
    }

    #[test]
    fn actspec_serde_roundtrip_click() {
        let s = ActSpec::Click {
            r#ref: "f0e3".into(),
        };
        let j = serde_json::to_value(&s).unwrap();
        let back: ActSpec = serde_json::from_value(j).unwrap();
        assert!(matches!(back, ActSpec::Click { .. }));
    }

    #[test]
    fn type_input_secret_debug_does_not_leak() {
        let t = TypeInput::Secret("hunter2supersecret".into());
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("hunter2"), "secret leaked in Debug: {dbg}");
    }

    /// **A 安全红线：`SetValue { secret: true }` 的明文绝不进 Debug**（镜像 TypeInput::Secret）。
    /// 即便把整个 spec `dbg!`/`{:?}` 出去（日志/tracing），也不泄漏 set_value 的 secret 明文。
    #[test]
    fn set_value_secret_debug_does_not_leak() {
        let s = ActSpec::SetValue {
            r#ref: "f0e1".into(),
            value: "TOP-SECRET-SETVALUE-PLAINTEXT".into(),
            secret: true,
        };
        let dbg = format!("{s:?}");
        assert!(
            !dbg.contains("TOP-SECRET-SETVALUE-PLAINTEXT"),
            "set_value secret leaked in ActSpec Debug: {dbg}"
        );
        // 应显示脱敏占位 + 仍标注 secret=true（让读日志的人知道这里是被脱敏的凭据）。
        assert!(dbg.contains("<redacted>"), "expected <redacted> placeholder: {dbg}");
        assert!(dbg.contains("secret: true"), "expected secret flag in Debug: {dbg}");
    }

    /// **对照：非 secret 的 SetValue 不脱敏**（明文 value 可见，可进日志诊断）。
    #[test]
    fn set_value_non_secret_debug_shows_value() {
        let s = ActSpec::SetValue {
            r#ref: "f0e1".into(),
            value: "plain visible value".into(),
            secret: false,
        };
        let dbg = format!("{s:?}");
        assert!(dbg.contains("plain visible value"), "non-secret value should be visible: {dbg}");
        assert!(dbg.contains("secret: false"), "{dbg}");
    }

    /// **A：`SetValue { secret }` serde round-trip**（含 secret 标志的线格契约）。Serialize 仍透出明文
    /// value（写回密码字段需要真值），脱敏只针对 Debug——与 TypeInput 同契约。
    #[test]
    fn set_value_secret_serde_roundtrip_preserves_value_and_flag() {
        let s = ActSpec::SetValue {
            r#ref: "f0e1".into(),
            value: "topsecret".into(),
            secret: true,
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j.get("action").and_then(|v| v.as_str()), Some("set_value"));
        assert_eq!(j.get("secret").and_then(|v| v.as_bool()), Some(true));
        // Serialize 透出明文（写回需要）；脱敏只在 Debug。
        assert!(serde_json::to_string(&j).unwrap().contains("topsecret"), "serialize must preserve value");
        let back: ActSpec = serde_json::from_value(j).unwrap();
        assert!(matches!(back, ActSpec::SetValue { secret: true, .. }));
    }

    /// **A：`secret` 字段 `#[serde(default)]` 向后兼容**——旧形态（无 `secret` 键）反序列化为
    /// `secret: false`（不破坏既有调用 / 回放）。
    #[test]
    fn set_value_secret_defaults_false_when_absent() {
        let j = serde_json::json!({"action": "set_value", "ref": "f0e1", "value": "v"});
        let back: ActSpec = serde_json::from_value(j).unwrap();
        assert!(matches!(back, ActSpec::SetValue { secret: false, .. }), "absent secret → false");
    }

    #[test]
    fn actspec_tag_is_action_snake_case() {
        // tag="action" + snake_case：导出形态是 {"action":"open_link_new_tab", ...}。
        let s = ActSpec::OpenLinkNewTab {
            url: "https://example.com".into(),
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j.get("action").and_then(|v| v.as_str()), Some("open_link_new_tab"));
    }

    #[test]
    fn actspec_unit_variant_roundtrips() {
        // 无字段变体（如 GetPageText）仍按 tag 形态 round-trip。
        let s = ActSpec::GetPageText;
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j.get("action").and_then(|v| v.as_str()), Some("get_page_text"));
        let back: ActSpec = serde_json::from_value(j).unwrap();
        assert!(matches!(back, ActSpec::GetPageText));
    }

    #[test]
    fn type_input_literal_debug_shows_value() {
        // 明文变体不脱敏（可进日志）。
        let t = TypeInput::Literal("hello world".into());
        let dbg = format!("{t:?}");
        assert!(dbg.contains("hello world"), "literal should be visible: {dbg}");
    }

    #[test]
    fn type_input_secret_serialize_preserves_value() {
        // 安全契约：Serialize 仍透出原值（写回密码字段需要），脱敏只针对 Debug。
        let t = TypeInput::Secret("topsecret".into());
        let j = serde_json::to_value(&t).unwrap();
        assert!(
            serde_json::to_string(&j).unwrap().contains("topsecret"),
            "serialize must preserve secret value for write-back"
        );
    }

    #[test]
    fn nested_action_roundtrips_scroll() {
        // 带嵌套枚举（ScrollTarget/ScrollDir）的变体也能 round-trip。
        let s = ActSpec::Scroll {
            target: ScrollTarget::Element {
                r#ref: "f1e2".into(),
            },
            direction: ScrollDir::Down,
            amount: Some(240.0),
        };
        let j = serde_json::to_value(&s).unwrap();
        let back: ActSpec = serde_json::from_value(j).unwrap();
        assert!(matches!(
            back,
            ActSpec::Scroll {
                direction: ScrollDir::Down,
                ..
            }
        ));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // C1 fill 三态兜底决策 + TypeInput 文本提取（[纯逻辑]，喂构造 Value，不进浏览器）。
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn parse_fill_outcome_done() {
        // 'done'：set-value 类控件已直接设值 + 派发事件 → 无需 insertText（成功）。
        let v = serde_json::Value::String("done".into());
        assert_eq!(parse_fill_outcome(Some(&v)), FillOutcome::Done);
    }

    #[test]
    fn parse_fill_outcome_needsinput() {
        // 'needsinput'：text/textarea 已 focus+全选 → tier2 走 insertText 真键入。
        let v = serde_json::Value::String("needsinput".into());
        assert_eq!(parse_fill_outcome(Some(&v)), FillOutcome::NeedsInput);
    }

    #[test]
    fn parse_fill_outcome_notconnected() {
        // 'error:notconnected'：元素 detach → 可重试/重拍。
        let v = serde_json::Value::String("error:notconnected".into());
        assert_eq!(parse_fill_outcome(Some(&v)), FillOutcome::NotConnected);
    }

    #[test]
    fn parse_fill_outcome_unknown_shapes_are_conservatively_notconnected() {
        // fill 契约只产上述三态；任何陌生形状保守判 NotConnected（不静默当成功）。
        assert_eq!(parse_fill_outcome(None), FillOutcome::NotConnected);
        assert_eq!(
            parse_fill_outcome(Some(&serde_json::Value::String("weird".into()))),
            FillOutcome::NotConnected
        );
        assert_eq!(
            parse_fill_outcome(Some(&serde_json::json!(7))),
            FillOutcome::NotConnected
        );
        assert_eq!(
            parse_fill_outcome(Some(&serde_json::Value::Null)),
            FillOutcome::NotConnected
        );
        assert_eq!(
            parse_fill_outcome(Some(&serde_json::json!({"x": 1}))),
            FillOutcome::NotConnected
        );
    }

    #[test]
    fn type_input_text_extracts_literal_and_secret_value() {
        // Literal / Secret 都返其内含字符串（C1：secret 走原值 insertText 路径，值不过 LLM）。
        assert_eq!(type_input_text(&TypeInput::Literal("hello".into())), "hello");
        assert_eq!(type_input_text(&TypeInput::Secret("hunter2".into())), "hunter2");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // F2：compose_click_anchor 合成 + changed 判定（[纯逻辑]，不进浏览器）。
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn compose_click_anchor_url_only() {
        // 只有 URL（元素态读不到）→ 对象含 url 键。
        let a = compose_click_anchor(Some("https://x.com/a"), None).unwrap();
        assert_eq!(a.get("url").and_then(|v| v.as_str()), Some("https://x.com/a"));
    }

    #[test]
    fn compose_click_anchor_merges_url_and_element_state() {
        // URL + 元素态（checkbox checked）→ 合并对象（url + checked + text）。
        let el = serde_json::json!({"checked": true, "text": "Agree"});
        let a = compose_click_anchor(Some("https://x.com"), Some(&el)).unwrap();
        assert_eq!(a.get("url").and_then(|v| v.as_str()), Some("https://x.com"));
        assert_eq!(a.get("checked").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(a.get("text").and_then(|v| v.as_str()), Some("Agree"));
    }

    #[test]
    fn compose_click_anchor_element_only() {
        // 无 URL（取不到）但有元素态 → 对象只含元素键（无 url）。
        let el = serde_json::json!({"value": "v"});
        let a = compose_click_anchor(None, Some(&el)).unwrap();
        assert!(a.get("url").is_none(), "no url key when url is None");
        assert_eq!(a.get("value").and_then(|v| v.as_str()), Some("v"));
    }

    #[test]
    fn compose_click_anchor_both_none_is_none() {
        // 两端都缺 → None（Effect 该端为 None，不构造空对象）。
        assert!(compose_click_anchor(None, None).is_none());
        // 元素态是非对象（不该发生）也不并入 → 只剩空 → None。
        assert!(compose_click_anchor(None, Some(&serde_json::json!("not-an-object"))).is_none());
    }

    /// **F2 changed 判定（before==after → false / != → true）**：用 compose_click_anchor 构造的两端锚点
    /// 直接比较——这是 act_click 里 `before != after` 的纯逻辑等价（checkbox 勾选前后 checked 翻转 →
    /// 锚点不等 → changed=true；点一个无副作用元素两次态相同 → 相等 → changed=false）。
    #[test]
    fn changed_judgment_before_ne_after() {
        // checkbox：点击前 unchecked、点击后 checked → 锚点不等 → changed=true。
        let before = compose_click_anchor(
            Some("https://x.com"),
            Some(&serde_json::json!({"checked": false})),
        );
        let after = compose_click_anchor(
            Some("https://x.com"),
            Some(&serde_json::json!({"checked": true})),
        );
        assert_ne!(before, after, "checked false→true must make anchors differ (changed=true)");

        // 无变化（点一个不改变态/不导航的元素）→ 锚点相等 → changed=false。
        let same_a = compose_click_anchor(Some("https://x.com"), Some(&serde_json::json!({"text": "Hi"})));
        let same_b = compose_click_anchor(Some("https://x.com"), Some(&serde_json::json!({"text": "Hi"})));
        assert_eq!(same_a, same_b, "identical state must make anchors equal (changed=false)");

        // 导航（URL 变，元素态同）→ 锚点不等 → changed=true。
        let nav_before = compose_click_anchor(Some("https://x.com/a"), None);
        let nav_after = compose_click_anchor(Some("https://x.com/b"), None);
        assert_ne!(nav_before, nav_after, "URL change must make anchors differ (changed=true)");
    }

    #[test]
    fn classify_inject_err_non_editable_is_fatal_blocked() {
        // 元素类型根本不支持编辑（createStacklessError 文案）→ Fatal(Blocked)，禁重试（B3 语义）。
        let e = InjectError::JsException(
            "Element is not an <input>, <textarea> or [contenteditable] element".into(),
        );
        match classify_inject_err(e) {
            RetryDecision::Fatal(BrowserError::Blocked { .. }) => {}
            other => panic!("non-editable must be Fatal(Blocked), got {other:?}"),
        }
    }

    #[test]
    fn classify_inject_err_context_not_ready_is_retryable() {
        // utility world 还没物化（导航中）→ 瞬态可重试。
        let e = InjectError::ContextNotReady {
            frame_id: "F0".into(),
        };
        assert!(matches!(classify_inject_err(e), RetryDecision::Retryable(_)));
    }

    #[test]
    fn classify_inject_err_other_js_exception_is_fatal() {
        // 其它注入异常（非不可编辑特例）→ Fatal（非瞬态缺态，不靠动作层短重试自愈）。
        let e = InjectError::JsException("TypeError: x is not a function".into());
        assert!(matches!(classify_inject_err(e), RetryDecision::Fatal(_)));
    }

    #[test]
    fn gate_check_result_pass_missing_notconnected() {
        use crate::actionability::CheckResult;
        // Pass → 放行。
        assert!(gate_check_result(CheckResult::Pass).is_ok());
        // Missing → 可重试（瞬态缺态）。
        assert!(matches!(
            gate_check_result(CheckResult::Missing("enabled".into())),
            Err(RetryDecision::Retryable(_))
        ));
        // NotConnected → 可重试（漂移，重定位）。
        assert!(matches!(
            gate_check_result(CheckResult::NotConnected),
            Err(RetryDecision::Retryable(BrowserError::NotConnected))
        ));
    }

    #[test]
    fn gate_geom_err_is_retryable() {
        use crate::input::GeomError;
        // NotVisible / NotInViewport 都按瞬态可重试（C1 最简，scroll 逃逸留 C2）。
        assert!(matches!(
            gate_geom_err(GeomError::NotVisible),
            RetryDecision::Retryable(_)
        ));
        assert!(matches!(
            gate_geom_err(GeomError::NotInViewport),
            RetryDecision::Retryable(_)
        ));
    }

    #[test]
    fn geom_err_reason_distinguishes_notvisible_and_notinviewport() {
        use crate::input::GeomError;
        // NotVisible 与 NotInViewport 文案不同(供 LLM 路由)。
        assert!(geom_err_reason(GeomError::NotVisible).contains("not visible"));
        assert!(geom_err_reason(GeomError::NotInViewport).contains("viewport"));
        assert_ne!(
            geom_err_reason(GeomError::NotVisible),
            geom_err_reason(GeomError::NotInViewport)
        );
    }

    #[test]
    fn classify_browser_err_notconnected_blocked_retryable_others_fatal() {
        // NotConnected → 可重试（漂移）。
        assert!(matches!(
            classify_browser_err(BrowserError::NotConnected),
            RetryDecision::Retryable(BrowserError::NotConnected)
        ));
        // Blocked（遮挡）→ 可重试一次让上层重判遮挡（DESIGN：遮挡通常 Retryable）。
        assert!(matches!(
            classify_browser_err(BrowserError::Blocked { reason: "DIV#overlay".into() }),
            RetryDecision::Retryable(BrowserError::Blocked { .. })
        ));
        // NodeStale（代际层）→ Fatal（动作层短重试无用，须上层重 observe）。
        assert!(matches!(
            classify_browser_err(BrowserError::NodeStale { generation: 3 }),
            RetryDecision::Fatal(BrowserError::NodeStale { .. })
        ));
        // 生命周期/超时终态 → Fatal。
        assert!(matches!(
            classify_browser_err(BrowserError::TargetClosed),
            RetryDecision::Fatal(BrowserError::TargetClosed)
        ));
    }

    /// **不可编辑 type 禁重试（不变量④）**：editable 检查路径的 `Blocked`（非可编辑特例）必须判
    /// `Fatal`（「只试一次」），**不**走 `classify_browser_err`（那会误判 Retryable → 770ms 退避）。
    /// 这是纯逻辑守卫：直接断言分类结果是 Fatal(Blocked)，证 act_type/act_set_value 收到不可编辑
    /// check 结果时立返而非退避（集成测的耗时断言是其行为佐证，本测是其类型保证）。
    #[test]
    fn classify_editable_check_err_non_editable_blocked_is_fatal() {
        // editable 检查返 Blocked（is_non_editable_error 命中，元素类型根本不支持编辑）→ Fatal 禁重试。
        match classify_editable_check_err(BrowserError::Blocked {
            reason: "Element is not an <input>, <textarea> ...".into(),
        }) {
            RetryDecision::Fatal(BrowserError::Blocked { .. }) => {}
            other => panic!("non-editable editable-check Blocked must be Fatal, got {other:?}"),
        }
    }

    /// **对照**：通用分类器仍把 Blocked 判 Retryable（click 遮挡路径不变），两分类器对 Blocked 分流相反。
    #[test]
    fn editable_and_generic_classifiers_diverge_on_blocked() {
        let blocked = || BrowserError::Blocked { reason: "X".into() };
        // 通用（click 遮挡）：Retryable（遮挡可瞬态散去）。
        assert!(matches!(
            classify_browser_err(blocked()),
            RetryDecision::Retryable(BrowserError::Blocked { .. })
        ));
        // editable 检查（不可编辑终态）：Fatal（禁重试）。
        assert!(matches!(
            classify_editable_check_err(blocked()),
            RetryDecision::Fatal(BrowserError::Blocked { .. })
        ));
    }

    /// editable 分类器对**非 Blocked** 错误语义与通用一致：NotConnected 仍可重试（漂移重定位）、
    /// 终态仍 Fatal。确保只「劫持」Blocked，不误伤 readOnly 可重试链（readOnly 走 Ok(Missing) 不到
    /// 这里，故此处验 NotConnected/终态即可代表「其它分支委托 classify_browser_err」）。
    #[test]
    fn classify_editable_check_err_non_blocked_matches_generic() {
        // NotConnected（漂移）→ 仍可重试（与 classify_browser_err 一致）。
        assert!(matches!(
            classify_editable_check_err(BrowserError::NotConnected),
            RetryDecision::Retryable(BrowserError::NotConnected)
        ));
        // NodeStale（代际层）→ Fatal（与 classify_browser_err 一致）。
        assert!(matches!(
            classify_editable_check_err(BrowserError::NodeStale { generation: 9 }),
            RetryDecision::Fatal(BrowserError::NodeStale { .. })
        ));
        // 终态 → Fatal。
        assert!(matches!(
            classify_editable_check_err(BrowserError::TargetClosed),
            RetryDecision::Fatal(BrowserError::TargetClosed)
        ));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // C2 纯逻辑：press_key Enter-in-form → IRREVERSIBLE 检测 + select_options 返回解析。
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn press_key_enter_in_form_is_irreversible() {
        // 裸 Enter + 焦点在 form 内 → IRREVERSIBLE（隐式提交风险，裁决⑧）。
        assert!(press_key_is_irreversible("Enter", true));
        assert!(press_key_is_irreversible("enter", true), "case-insensitive");
        assert!(press_key_is_irreversible("Return", true), "Return alias");
    }

    #[test]
    fn press_key_enter_not_in_form_is_reversible() {
        // 裸 Enter 但焦点**不**在 form 内 → 不升级（普通 Exec 级）。
        assert!(!press_key_is_irreversible("Enter", false));
    }

    #[test]
    fn press_key_non_enter_is_reversible() {
        // 其它键（即便在 form 内）→ 不升级。
        assert!(!press_key_is_irreversible("Tab", true));
        assert!(!press_key_is_irreversible("Escape", true));
        assert!(!press_key_is_irreversible("a", true));
        assert!(!press_key_is_irreversible("ArrowDown", true));
    }

    #[test]
    fn press_key_modified_enter_is_reversible() {
        // 带修饰键的 Enter（Ctrl+Enter / Shift+Enter）通常是「换行/另存」非隐式提交 → 不升级。
        assert!(!press_key_is_irreversible("Ctrl+Enter", true));
        assert!(!press_key_is_irreversible("Shift+Enter", true));
        assert!(!press_key_is_irreversible("Meta+Enter", true));
    }

    #[test]
    fn press_key_malformed_keys_is_reversible() {
        // 解析失败（畸形/未知键）→ 保守不升级（非阻塞，真执行会另报 key combo 错）。
        assert!(!press_key_is_irreversible("", true));
        assert!(!press_key_is_irreversible("Ctrl+", true));
        assert!(!press_key_is_irreversible("Frobnicate", true));
    }

    #[test]
    fn parse_select_outcome_selected_array() {
        // string[] → Selected(values)（多选含全部命中 value）。
        let v = serde_json::json!(["opt2"]);
        assert_eq!(
            parse_select_outcome(Some(&v)),
            SelectOutcome::Selected(vec!["opt2".into()])
        );
        let v = serde_json::json!(["a", "b"]);
        assert_eq!(
            parse_select_outcome(Some(&v)),
            SelectOutcome::Selected(vec!["a".into(), "b".into()])
        );
        // 空数组（没选中任何项，多选清空）→ Selected([])。
        assert_eq!(
            parse_select_outcome(Some(&serde_json::json!([]))),
            SelectOutcome::Selected(vec![])
        );
    }

    #[test]
    fn parse_select_outcome_error_strings() {
        assert_eq!(
            parse_select_outcome(Some(&serde_json::json!("error:notconnected"))),
            SelectOutcome::NotConnected
        );
        assert_eq!(
            parse_select_outcome(Some(&serde_json::json!("error:optionsnotfound"))),
            SelectOutcome::OptionsNotFound
        );
        assert_eq!(
            parse_select_outcome(Some(&serde_json::json!("error:optionnotenabled"))),
            SelectOutcome::OptionNotEnabled
        );
    }

    #[test]
    fn parse_select_outcome_unknown_shapes() {
        // 注入契约只产 string[] | 上述错误串；其它形状 → Unknown（保守失败，不静默成功）。
        assert_eq!(parse_select_outcome(None), SelectOutcome::Unknown);
        assert_eq!(
            parse_select_outcome(Some(&serde_json::json!("weird"))),
            SelectOutcome::Unknown
        );
        assert_eq!(
            parse_select_outcome(Some(&serde_json::json!(7))),
            SelectOutcome::Unknown
        );
        assert_eq!(
            parse_select_outcome(Some(&serde_json::json!({"x": 1}))),
            SelectOutcome::Unknown
        );
    }

    #[test]
    fn scroll_alignments_has_four_distinct() {
        // 4 alignment 逃 sticky（裁决⑮）：center 优先，4 个互异。
        assert_eq!(SCROLL_ALIGNMENTS.len(), 4);
        assert_eq!(SCROLL_ALIGNMENTS[0], "center");
        let set: std::collections::HashSet<_> = SCROLL_ALIGNMENTS.iter().collect();
        assert_eq!(set.len(), 4, "alignments must be distinct");
    }

    #[test]
    fn c2_actspec_roundtrips() {
        // C2 涉及的 ActSpec 变体 serde round-trip（facade 解析入口契约）。
        for (spec, tag) in [
            (ActSpec::Hover { r#ref: "f0e1".into() }, "hover"),
            (
                ActSpec::SelectOption {
                    r#ref: "f0e2".into(),
                    options: vec!["a".into()],
                },
                "select_option",
            ),
            (ActSpec::PressKey { keys: "Enter".into() }, "press_key"),
            (ActSpec::ScrollToText { text: "footer".into() }, "scroll_to_text"),
        ] {
            let j = serde_json::to_value(&spec).unwrap();
            assert_eq!(j.get("action").and_then(|v| v.as_str()), Some(tag));
            let _back: ActSpec = serde_json::from_value(j).unwrap();
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // C3 纯逻辑：Wait ms 钳制 + search grep + dropdown/find 解析 + wait_for 超时映射 +
    // C3 ActSpec round-trip。全只读类。
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn clamp_wait_ms_caps_at_limit() {
        // 上限内不动。
        assert_eq!(clamp_wait_ms(0), (0, false));
        assert_eq!(clamp_wait_ms(500), (500, false));
        assert_eq!(clamp_wait_ms(WAIT_MS_CAP), (WAIT_MS_CAP, false));
        // 超上限钳制 + 标记 capped。
        assert_eq!(clamp_wait_ms(WAIT_MS_CAP + 1), (WAIT_MS_CAP, true));
        assert_eq!(clamp_wait_ms(u64::MAX), (WAIT_MS_CAP, true));
    }

    #[test]
    fn grep_page_text_hits_case_insensitive_substring() {
        let text = "Welcome to NomiFun\nYour Order #42 is ready\nContact support";
        // 命中（大小写不敏感子串）。
        let hits = grep_page_text(text, "order", 0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, "Your Order #42 is ready");
        // 大写 query 同样命中。
        assert_eq!(grep_page_text(text, "NOMIFUN", 0).len(), 1);
        // 多行命中（"o" 出现在多行）。
        assert!(grep_page_text(text, "o", 0).len() >= 2);
    }

    #[test]
    fn grep_page_text_miss_and_empty_query() {
        let text = "alpha\nbeta\ngamma";
        // 未命中 → 空（非报错）。
        assert!(grep_page_text(text, "zzz-nonexistent", 0).is_empty());
        // 空 query → 空（良性，不报错）。
        assert!(grep_page_text(text, "", 0).is_empty());
    }

    #[test]
    fn grep_page_text_respects_cap_and_skips_blank_lines() {
        let text = "match A\n\n   \nmatch B\nmatch C";
        // cap=2 → 只前两条。
        let hits = grep_page_text(text, "match", 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, "match A");
        assert_eq!(hits[1].line, "match B");
        // 空行不计入（即便 query 是空白也不命中空行，且空行被跳过）。
        let all = grep_page_text(text, "match", 0);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn parse_dropdown_outcome_options() {
        let v = serde_json::json!({
            "ok": true,
            "options": [
                {"value": "opt1", "label": "First", "selected": true, "disabled": false},
                {"value": "opt2", "label": "Second", "selected": false, "disabled": true}
            ]
        });
        match parse_dropdown_outcome(Some(&v)) {
            DropdownOutcome::Options(opts) => {
                assert_eq!(opts.len(), 2);
                assert_eq!(opts[0], DropdownOption {
                    value: "opt1".into(), label: "First".into(), selected: true, disabled: false
                });
                assert_eq!(opts[1].value, "opt2");
                assert!(opts[1].disabled);
            }
            other => panic!("expected Options, got {other:?}"),
        }
        // 空 select（无 option）→ Options([])。
        assert_eq!(
            parse_dropdown_outcome(Some(&serde_json::json!({"ok": true, "options": []}))),
            DropdownOutcome::Options(vec![])
        );
    }

    #[test]
    fn parse_dropdown_outcome_error_strings_and_unknown() {
        assert_eq!(
            parse_dropdown_outcome(Some(&serde_json::json!("error:notselect"))),
            DropdownOutcome::NotSelect
        );
        assert_eq!(
            parse_dropdown_outcome(Some(&serde_json::json!("error:notconnected"))),
            DropdownOutcome::NotConnected
        );
        // 陌生形状 → Unknown（保守失败）。
        assert_eq!(parse_dropdown_outcome(None), DropdownOutcome::Unknown);
        assert_eq!(parse_dropdown_outcome(Some(&serde_json::json!("weird"))), DropdownOutcome::Unknown);
        assert_eq!(parse_dropdown_outcome(Some(&serde_json::json!(7))), DropdownOutcome::Unknown);
        assert_eq!(parse_dropdown_outcome(Some(&serde_json::json!({"x": 1}))), DropdownOutcome::Unknown);
    }

    #[test]
    fn parse_find_outcome_found_with_total() {
        let v = serde_json::json!({
            "ok": true,
            "matches": [
                {"ref": "f0e1000001", "role": "button", "name": "Submit"},
                {"ref": "f0e1000002", "role": "link", "name": ""}
            ],
            "total": 5
        });
        match parse_find_outcome(Some(&v)) {
            FindOutcome::Found { matches, total } => {
                assert_eq!(total, 5, "total may exceed matches.len() when capped");
                assert_eq!(matches.len(), 2);
                assert_eq!(matches[0], FoundElement {
                    r#ref: "f0e1000001".into(), role: "button".into(), name: "Submit".into()
                });
                assert_eq!(matches[1].name, "");
            }
            other => panic!("expected Found, got {other:?}"),
        }
        // 无 total → 默认 matches.len()。
        let v2 = serde_json::json!({"matches": []});
        assert_eq!(parse_find_outcome(Some(&v2)), FindOutcome::Found { matches: vec![], total: 0 });
    }

    #[test]
    fn parse_find_outcome_notobserved_and_unknown() {
        assert_eq!(
            parse_find_outcome(Some(&serde_json::json!("error:notobserved"))),
            FindOutcome::NotObserved
        );
        // 陌生形状 → Unknown。
        assert_eq!(parse_find_outcome(None), FindOutcome::Unknown);
        assert_eq!(parse_find_outcome(Some(&serde_json::json!("weird"))), FindOutcome::Unknown);
        assert_eq!(parse_find_outcome(Some(&serde_json::json!(7))), FindOutcome::Unknown);
    }

    /// **wait_for 超时 → Timeout{Action}**：deadline 已过且条件未满足 → run_act_with_retry 外的
    /// wait_for 轮询返 `Timeout{phase:Action}`。这里用 errmap 直接验「Progress timeout 在动作层映射成
    /// Timeout{Action}」（wait_for 的超时分支与之同形——超 deadline 即返该错误）。
    #[test]
    fn wait_for_timeout_maps_to_action_timeout() {
        use crate::engine::NavPhase;
        use crate::errmap::map_progress_err;
        use crate::progress::ProgressError;
        // wait_for 超 deadline 返 BrowserError::Timeout{Action}（与 map_progress_err 的 Timeout 映射同形）。
        assert!(matches!(
            map_progress_err(ProgressError::Timeout),
            BrowserError::Timeout { phase: NavPhase::Action }
        ));
        // 直接构造 wait_for 超时分支返回的错误，确认其形态。
        let timeout = BrowserError::Timeout { phase: NavPhase::Action };
        assert!(format!("{timeout}").to_lowercase().contains("timeout"));
    }

    #[test]
    fn describe_wait_condition_renders() {
        assert_eq!(
            describe_wait_condition(&WaitCondition::UrlContains { text: "/done".into() }),
            "URL contains \"/done\""
        );
        assert_eq!(
            describe_wait_condition(&WaitCondition::TextVisible { text: "Loaded".into() }),
            "text \"Loaded\" is visible"
        );
        assert_eq!(
            describe_wait_condition(&WaitCondition::RefActionable { r#ref: "f0e7".into() }),
            "f0e7 is actionable"
        );
    }

    #[test]
    fn c3_actspec_roundtrips() {
        // C3 涉及的 ActSpec 变体 serde round-trip（facade 解析入口契约）。
        for (spec, tag) in [
            (ActSpec::GetPageText, "get_page_text"),
            (ActSpec::SearchPage { query: "order".into() }, "search_page"),
            (ActSpec::FindElements { selector: "button.primary".into() }, "find_elements"),
            (ActSpec::GetDropdownOptions { r#ref: "f0e3".into() }, "get_dropdown_options"),
            (ActSpec::Cursor, "cursor"),
            (ActSpec::Wait { ms: 500 }, "wait"),
            (ActSpec::WaitFor { condition: WaitCondition::UrlContains { text: "/ok".into() } }, "wait_for"),
        ] {
            let j = serde_json::to_value(&spec).unwrap();
            assert_eq!(j.get("action").and_then(|v| v.as_str()), Some(tag), "tag for {spec:?}");
            let _back: ActSpec = serde_json::from_value(j).unwrap();
        }
    }

    #[test]
    fn wait_condition_kind_tag_roundtrips() {
        // WaitCondition 用 #[serde(tag="kind")]：{"kind":"url_contains","text":"..."}。
        let c = WaitCondition::TextVisible { text: "hello".into() };
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(j.get("kind").and_then(|v| v.as_str()), Some("text_visible"));
        let back: WaitCondition = serde_json::from_value(j).unwrap();
        assert!(matches!(back, WaitCondition::TextVisible { .. }));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ActMode 三档门控（[纯逻辑]，force/trial 骨架；不进浏览器）
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn act_mode_default_is_actionable() {
        // 默认档 = Actionable（接线缺省，行为与历史一致）。
        assert_eq!(ActMode::default(), ActMode::Actionable);
    }

    #[test]
    fn actionable_runs_checks_and_dispatches() {
        // actionable：跑全部检查（不跳过）+ 真投递。
        assert!(!mode_skips_checks(ActMode::Actionable));
        assert!(mode_dispatches(ActMode::Actionable));
    }

    #[test]
    fn force_skips_checks_but_dispatches() {
        // force：绕过 actionability 检查（直 dispatchEvent），但仍真投递。
        assert!(mode_skips_checks(ActMode::Force));
        assert!(mode_dispatches(ActMode::Force));
    }

    #[test]
    fn trial_runs_checks_but_does_not_dispatch() {
        // trial：跑检查判定可达性，但**不**真投递（只判定不执行）。
        assert!(!mode_skips_checks(ActMode::Trial));
        assert!(!mode_dispatches(ActMode::Trial));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // #[ignore] 真 Chrome 集成测试：sticky 逃逸端到端验证
    // ═══════════════════════════════════════════════════════════════════════

    /// **click 逃 sticky / 滚入视口 端到端**（C2，DESIGN §11 裁决⑮）：目标按钮初始在**视口外**
    /// （`top:2000px`，远低于初始视口），且页面顶部有 80px 的 fixed sticky header。点击时
    /// `pick_click_point` 报 `NotInViewport` → `scroll_escape_sticky` 按 4-alignment 逃逸（首个
    /// `center` 把按钮滚到视口中部、避开顶部 sticky header）后重取点点中 → click `success`。
    ///
    /// 手动跑：
    ///   NOMIFUN_CHROME_BINARY="/path/to/chrome" \
    ///   cargo nextest run -p nomi-browser-engine --run-ignored all click_escapes_sticky_header
    #[tokio::test]
    #[ignore = "requires NOMIFUN_CHROME_BINARY (real Chrome)"]
    async fn click_escapes_sticky_header_and_succeeds() {
        use base64::Engine as _;
        use std::time::Duration;

        // 按钮放在 top:2000px（初始视口外）→ 取点报 NotInViewport → 触发 scroll_escape_sticky。
        // 顶部 80px fixed sticky header 验证逃逸的 center alignment 把按钮滚到中部、不被 header 盖住。
        let html = r#"<!doctype html><html><body style="margin:0;height:3000px">
          <header style="position:fixed;top:0;left:0;width:100%;height:80px;
                         background:dimgray;z-index:9999">sticky</header>
          <button id="b" style="position:absolute;top:2000px;left:10px"
                  onclick="this.textContent='clicked'">target</button>
        </body></html>"#;
        // base64 data URL：稳健编码（原始 `#`/换行/引号在裸 data: URL 里会截断/破坏，故必须编码）。
        let data_url = format!(
            "data:text/html;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(html)
        );

        let engine = crate::create_engine(crate::EngineConfig::default())
            .await
            .expect("create_engine (set NOMIFUN_CHROME_BINARY)");
        engine
            .navigate(&data_url, false)
            .await
            .expect("navigate data URL");

        let obs = engine
            .observe(&crate::engine::ObserveOpts::default())
            .await
            .expect("observe");

        // 取按钮 ref（role=button, name=target）。
        let button = obs
            .entries
            .iter()
            .find(|e| e.role == "button" && e.name == "target")
            .expect("fixture should expose a button named \"target\"");
        let r#ref = button.r#ref.clone();

        let p = crate::progress::Progress::new(Duration::from_secs(15));
        let result = engine
            .act(&ActSpec::Click { r#ref }, &p)
            .await
            .expect("click should not error");
        assert!(
            result.success,
            "click after sticky/viewport escape must succeed (button was at top:2000px, out of viewport)"
        );
    }
}
