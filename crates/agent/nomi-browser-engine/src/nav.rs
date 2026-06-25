//! **navigate settle 升级**（P2 D2，DESIGN §12 + 裁决⑤）。
//!
//! P0 的 navigate 只等 `Page.loadEventFired`（粗糙：SPA 软导航不触发 load 会白等到超时；不分阶段；
//! redirect 用裸 `!=` 判定会被 trailing-slash 误报；无 http_status；load_state 是弱类型字符串）。
//!
//! D2 把它升级为成熟的导航判定，拆成两类逻辑：
//!
//! ## 纯逻辑（本模块 `#[cfg(test)]` 全覆盖，不进浏览器）
//! - [`InflightCounter`]：networkidle 的 inflight 请求计数器。按 CDP `Network.*` 事件 +1/-1。
//! - [`url_normalize`] / [`is_redirect`]：归一化 URL 比较判 redirect（trailing-slash / 默认端口 /
//!   fragment 不算 redirect；query 保留）——**非裸 `!=`**。
//! - [`classify_lifecycle`] / [`NavSettleState`]：导航各生命周期事件 → 状态机推进的纯判定。
//!
//! ## 编排（[`crate::backend::cdp::CdpBackend::navigate`] 经 active tab 句柄调用）
//! settle 阶梯：等 `DOMContentLoaded` → 短 settle → 可交互探测（best-effort）→ 升级到 `Load`；
//! 之后**独立短 cap（3-5s）**等 networkidle，达到返 `NetworkIdle`，到 cap 降级回 `Load`（**绝不**
//! 并入 30s 导航超时——长轮询/SSE/WS 站永不 idle 也不会卡死整个 navigate）。SPA 软导航
//! （`Page.navigatedWithinDocument`，无 newDocument）→ 降级「等 URL 变 / 下一目标 actionable」，
//! 不重新等 load。
//!
//! 良性态不报错：networkidle cap 降级、SPA 软导航、302 重定向都是 `success=true` 的正常路径。

use std::time::Duration;

/// networkidle 的 inflight 请求计数器（**纯逻辑**，便于单测，不持任何浏览器状态）。
///
/// 按 CDP `Network.*` 事件维护「当前在途请求数」。networkidle 判定 = inflight 持续为 0 满
/// [`NETWORK_IDLE_QUIET`]（500ms）。
///
/// **加减语义**（已对照 chromiumoxide CDP `Network` 事件字段 + Playwright `network.ts` 的请求
/// 生命周期模型核对，见各方法 doc）：
/// - `Network.requestWillBeSent`（**无** `redirectResponse`）→ [`InflightCounter::on_request_will_be_sent`]
///   传 `is_redirect=false` → **+1**（一个全新请求开始在途）。
/// - `Network.requestWillBeSent`（**有** `redirectResponse`）→ 传 `is_redirect=true` → **不变**
///   （这是**已计数**请求被重定向后的续发，同一 requestId 复用；旧请求不会另发
///   loadingFinished/loadingFailed，故既不 +1 也不 -1，避免重复计数）。
/// - `Network.responseReceived` → **不变**（响应头到达 ≠ 请求完成；请求要到 loadingFinished/
///   loadingFailed 才离开在途。这与 PW 的「按 request 生命周期计数」一致——`responseReceived`
///   只是中途事件）。
/// - `Network.loadingFinished` → [`InflightCounter::on_loading_finished`] → **-1**（钳到 0 不下溢）。
/// - `Network.loadingFailed` → [`InflightCounter::on_loading_failed`] → **-1**（同上）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InflightCounter {
    inflight: u32,
}

impl InflightCounter {
    /// 全新计数器（inflight=0）。
    pub fn new() -> Self {
        Self { inflight: 0 }
    }

    /// 当前在途请求数。
    pub fn inflight(&self) -> u32 {
        self.inflight
    }

    /// 是否已空闲（无任何在途请求）。networkidle 判定的「瞬时空闲」条件——还需持续
    /// [`NETWORK_IDLE_QUIET`] 才算真正 idle（持续性由编排层计时，不在本纯逻辑里）。
    pub fn is_idle(&self) -> bool {
        self.inflight == 0
    }

    /// `Network.requestWillBeSent`：
    /// - `is_redirect == false`（params 无 `redirectResponse`）→ +1（新请求在途）。
    /// - `is_redirect == true`（params 有 `redirectResponse`）→ 不变（已计数请求的重定向续发，
    ///   同 requestId 复用，避免重复 +1）。
    pub fn on_request_will_be_sent(&mut self, is_redirect: bool) {
        if !is_redirect {
            self.inflight += 1;
        }
    }

    /// `Network.loadingFinished`：-1（钳到 0，绝不下溢——防御乱序/重复事件 / Network.enable 前
    /// 已在途的请求的 finish 事件）。
    pub fn on_loading_finished(&mut self) {
        self.inflight = self.inflight.saturating_sub(1);
    }

    /// `Network.loadingFailed`：-1（同 [`InflightCounter::on_loading_finished`]，钳到 0）。
    pub fn on_loading_failed(&mut self) {
        self.inflight = self.inflight.saturating_sub(1);
    }
}

/// networkidle 的「持续空闲」窗口：inflight 连续为 0 满此时长 → 判定 networkidle（DESIGN §12）。
pub const NETWORK_IDLE_QUIET: Duration = Duration::from_millis(500);

/// networkidle 等待的**独立短 cap**（DESIGN §12 + 裁决⑤：3-5s，取 4s 折中）。到此 cap 仍未达成
/// networkidle（长轮询/SSE/WS 站永不 idle）→ **降级回 `Load`**。这个 cap **绝不并入** 30s 导航总
/// 超时——networkidle 只是 `Load` 之上的「锦上添花」，达不到就退而求其次，不拖垮整个 navigate。
pub const NETWORK_IDLE_CAP: Duration = Duration::from_secs(4);

/// DOMContentLoaded 之后的「短 settle」窗口：给同步脚本 / 首批微任务一点喘息，再做可交互探测。
/// 不宜长（DESIGN：是「短 settle」），取 100ms。
pub const SETTLE_QUIET: Duration = Duration::from_millis(100);

/// 等 `DOMContentLoaded` 的上限。超过仍没等到 → 不致命（页面可能极慢或已是 SPA 软导航场景），
/// 编排层据已收到的事件降级返回当前能确定的 load_state（commit / load）。导航总预算另由
/// [`crate::transport::DEFAULT_COMMAND_TIMEOUT`]（30s，传输层每命令）与 settle 上限共同兜底。
pub const DOMCONTENTLOADED_TIMEOUT: Duration = Duration::from_secs(30);

/// SPA 软导航后等「URL 变化 / 下一目标 actionable」的上限（DESIGN §12：same-document 不重新等
/// load，只等软导航落地信号）。取一个短上限——软导航通常瞬时完成。
pub const SPA_SETTLE_TIMEOUT: Duration = Duration::from_secs(3);

// ── URL 归一化 + redirect 判定（纯函数，便于单测）──────────────────────────────────

/// **[纯逻辑] 归一化一个 URL 用于 redirect 等价比较**（DESIGN §12：redirected 用归一化比较，
/// **非裸 `!=`**）。归一化规则（只为「是否同一资源」的语义等价，不追求 RFC 3986 全规范化）：
///
/// 1. **去除 fragment**（`#...`）——fragment 是客户端锚点，从不构成「服务器把你重定向到别处」。
/// 2. **去除默认端口**（http→`:80`、https→`:443`、ws→`:80`、wss→`:443`）。
/// 3. **path 为空 → 视为 `/`**，且**去除末尾 trailing slash**后再比较（`a.com` ≡ `a.com/` ≡
///    `a.com//`）。注意：只规整「host 后紧跟的根 path」与「末尾斜杠」；中间路径不动。
/// 4. **scheme/host 转小写**（大小写不敏感部分）；**query 原样保留**（`?a=1` 与无 query 不等价——
///    query 改变即不同资源/不同导航结果）。
///
/// 解析失败（非 http(s)/ws(s) 或畸形）→ 原样返回（trim 末尾斜杠 + 去 fragment 的尽力而为），
/// 让上层退化为「宽松比较」而非 panic。
pub fn url_normalize(url: &str) -> String {
    // 1) 去 fragment（'#' 之后全部丢弃；fragment 永不影响导航等价）。
    let no_frag = match url.split_once('#') {
        Some((before, _)) => before,
        None => url,
    };

    // 切出 scheme://（用于判默认端口 + 小写 scheme/host）。
    let (scheme, rest) = match no_frag.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        // 非 scheme://host 形态（如 about:blank / data: / 畸形）→ 尽力而为：去末尾斜杠返回。
        None => return no_frag.trim_end_matches('/').to_string(),
    };

    // rest = authority[/path][?query]。先切出 authority（到首个 '/' 或 '?'）。
    let (authority, path_and_query) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };

    // authority 可能含 userinfo@host:port。只小写 host:port 部分（userinfo 大小写敏感，保守保留）。
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let hostport_lc = hostport.to_ascii_lowercase();

    // 去默认端口。
    let default_port = match scheme.as_str() {
        "http" | "ws" => Some(":80"),
        "https" | "wss" => Some(":443"),
        _ => None,
    };
    let hostport_norm = match default_port {
        Some(dp) if hostport_lc.ends_with(dp) => &hostport_lc[..hostport_lc.len() - dp.len()],
        _ => hostport_lc.as_str(),
    };

    // 切 path 与 query。
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };
    // path 归一：空 / 全是 '/' → 视为根（空）；否则去末尾 trailing slash。
    let path_norm = path.trim_end_matches('/');

    // 重组。
    let mut out = String::with_capacity(no_frag.len());
    out.push_str(&scheme);
    out.push_str("://");
    if let Some(u) = userinfo {
        out.push_str(u);
        out.push('@');
    }
    out.push_str(hostport_norm);
    out.push_str(path_norm);
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    out
}

/// **[纯逻辑] 判定一次导航是否「真的被重定向」**（DESIGN §12）：归一化 `from` 与 `to` 后比较，
/// 不等才算 redirect。trailing-slash / 默认端口 / fragment 差异**不算** redirect（裸 `!=` 会误报）；
/// query 变化 / host 变化 / path 变化 / scheme 变化（http→https 登录墙跳转）算 redirect。
pub fn is_redirect(from: &str, to: &str) -> bool {
    url_normalize(from) != url_normalize(to)
}

// ── 导航生命周期状态机（纯逻辑：事件分类 + 状态推进，不接事件源）────────────────────

/// 导航 settle 推进过程中已达成的最高里程碑（纯逻辑状态，与 [`crate::engine::LoadState`] 对应但
/// 内部用——编排层据它决定还要不要继续等、最终返回哪个 `LoadState`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NavSettleState {
    /// 还没等到 DOMContentLoaded（仅导航回包成功 = 文档已提交）。
    Commit,
    /// 已收到 `Page.domContentEventFired`。
    DomContentLoaded,
    /// 已收到 `Page.loadEventFired`。
    Load,
}

/// 一个对 settle 状态机有意义的 CDP 生命周期信号（编排层把订阅到的事件先分类成这个再喂状态机）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleSignal {
    /// `Page.domContentEventFired`。
    DomContentLoaded,
    /// `Page.loadEventFired`。
    Load,
    /// `Page.navigatedWithinDocument`（SPA 软导航，无 newDocument）。
    NavigatedWithinDocument,
}

/// **[纯逻辑] 把一个 CDP 事件 method 串分类成 [`LifecycleSignal`]**（无关事件 → None）。
/// 抽出便于单测「哪些 method 推进状态机」，且让编排层的订阅 → 分类边界清晰。
pub fn classify_lifecycle(method: &str) -> Option<LifecycleSignal> {
    match method {
        "Page.domContentEventFired" => Some(LifecycleSignal::DomContentLoaded),
        "Page.loadEventFired" => Some(LifecycleSignal::Load),
        "Page.navigatedWithinDocument" => Some(LifecycleSignal::NavigatedWithinDocument),
        _ => None,
    }
}

/// **[纯逻辑] 用一个生命周期信号推进 settle 状态**（取「更高里程碑」单调推进，不回退）。
/// 返回推进后的状态。`NavigatedWithinDocument` 不推进 load 阶梯（SPA 软导航走另一条降级路径，
/// 由编排层据它短路，不在此处把它当 Load）。
pub fn advance_settle(state: NavSettleState, signal: LifecycleSignal) -> NavSettleState {
    let next = match signal {
        LifecycleSignal::DomContentLoaded => NavSettleState::DomContentLoaded,
        LifecycleSignal::Load => NavSettleState::Load,
        // 软导航不属于 load 阶梯；保持当前状态（编排层据软导航信号短路）。
        LifecycleSignal::NavigatedWithinDocument => return state,
    };
    state.max(next)
}

/// **[纯逻辑] 从 `Network.requestWillBeSent` 的 params 判断是否「重定向续发」**（有 `redirectResponse`
/// 字段即是）。抽纯函数便于单测形状解析 + 让 [`InflightCounter::on_request_will_be_sent`] 的入参
/// 来源单点化。
pub fn request_is_redirect(params: &serde_json::Value) -> bool {
    params.get("redirectResponse").is_some()
}

/// **[纯逻辑] 从 `Network.responseReceived` 的 params 提取「主帧文档响应」的 HTTP status**。
///
/// 只认 `type == "Document"` 且 `frameId == main_frame_id` 的响应（主帧的文档响应才是「这次导航的
/// HTTP 状态」；子资源 / 子帧响应不算）。命中返回 `Some(status as u16)`；否则 None。CDP 的 status
/// 是 `i64`，钳进 `u16`（HTTP 状态码 100-599，必在范围内；越界 → None 保守）。
pub fn extract_main_doc_status(params: &serde_json::Value, main_frame_id: &str) -> Option<u16> {
    let rtype = params.get("type").and_then(|v| v.as_str())?;
    if rtype != "Document" {
        return None;
    }
    let frame_id = params.get("frameId").and_then(|v| v.as_str())?;
    if frame_id != main_frame_id {
        return None;
    }
    let status = params
        .get("response")
        .and_then(|r| r.get("status"))
        .and_then(|s| s.as_i64())?;
    u16::try_from(status).ok()
}

// ── D4：history 导航（back/forward）+ POST 页 reload 不可逆检测（纯逻辑）──────────────

/// 一次历史导航的方向（[`HistoryNav::Back`] = 后退一格 / [`HistoryNav::Forward`] = 前进一格）。
/// `Page.getNavigationHistory` 给出 `currentIndex` + `entries[]`；back/forward 据此算目标 entry 的
/// 索引（见 [`history_target_index`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryNav {
    /// 后退：目标 = `current_index - 1`（在首页则无更多历史）。
    Back,
    /// 前进：目标 = `current_index + 1`（在末页则无更多历史）。
    Forward,
}

/// **[纯逻辑] 算 back/forward 的目标历史 entry 索引，带边界钳制**（D4，DESIGN §12）。
///
/// `Page.getNavigationHistory` 返 `currentIndex`（当前 entry 在 `entries[]` 里的下标）+ `entries`
/// （`0..len`，按时间顺序，越早越靠前）。
/// - [`HistoryNav::Back`] → `current_index - 1`：已在**首页**（`current_index == 0`，或越界负数）
///   → `None`（无更多历史，**良性**，调用方返 success=true「无更多历史」**不报错、不 panic**）。
/// - [`HistoryNav::Forward`] → `current_index + 1`：已在**末页**（`current_index >= len-1`）→
///   `None`（同上，良性无更多历史）。
///
/// 入参 `current_index` 用 `i64`（CDP 原样给 i64；负数 / 越界都按「无目标」处理，绝不下溢 panic）；
/// `entries_len` 是 `entries.len()`。返回 `Some(target_usize)`（落在 `0..entries_len` 内）或 `None`
/// （边界/越界/空历史，良性）。
pub fn history_target_index(
    current_index: i64,
    entries_len: usize,
    direction: HistoryNav,
) -> Option<usize> {
    // 空历史 → 永远无目标（防 entries_len==0 时任何方向越界）。
    if entries_len == 0 {
        return None;
    }
    let target: i64 = match direction {
        HistoryNav::Back => current_index.checked_sub(1)?,
        HistoryNav::Forward => current_index.checked_add(1)?,
    };
    // 钳制到 [0, entries_len)：负数（首页 back）/ >=len（末页 forward）→ None（良性无更多历史）。
    if target < 0 {
        return None;
    }
    let target = usize::try_from(target).ok()?;
    if target >= entries_len {
        return None;
    }
    Some(target)
}

/// **[纯逻辑] 判一次历史 entry 的导航是否来自 POST 表单提交**（D4 reload→IRREVERSIBLE 检测的基础，
/// DESIGN §16 不可逆动作 + 裁决⑧「POST 页 reload → IRREVERSIBLE」）。
///
/// `Page.getNavigationHistory` 的每个 `NavigationEntry` 带 `transitionType`（chromium 的导航分类）。
/// **`"form_submit"`（CDP 串 `form_submit` / `FormSubmit`）是 POST 表单提交导航的稳定信号**——浏览器
/// 把「提交一个 `<form method=post>` 落到的页面」标为该 transition。reload 这样的页面会**重新提交
/// 表单**（浏览器弹「确认重新提交表单」），是不可逆副作用（重复下单/扣款/发消息）。
///
/// 判据：`transition_type` 规范化（小写 + 去 `_`）后 `== "formsubmit"`。CDP wire 串是 snake_case
/// `"form_submit"`；为防上游/序列化偶发给 CamelCase `"FormSubmit"`，归一化后比较（两形态都命中）。
/// 其它 transition（`link`/`typed`/`reload`/`auto_subframe`/…）→ `false`（GET 导航，reload 幂等，
/// 非不可逆）。
///
/// **可行性说明**：CDP **不直接**暴露导航的 HTTP method；`transitionType=="form_submit"` 是**最接近**
/// 的可观测信号，PW/browser-use 同此判据。GET 表单（`method=get`，`?a=1` query 字符串）也会标
/// `form_submit` 但 reload 幂等——这是**保守过判**（把个别幂等 GET 表单也判不可逆，宁可多确认一次也
/// 不漏判一个真 POST 重提交）；反之**不会漏判**真 POST。拿不到 transition（缺字段/形状陌生）→ `false`
/// （**保守默认：拿不准时不误判为 IRREVERSIBLE**，与 spec「拿不准时不误判」一致——避免给每个普通页
/// reload 都加确认门）。
pub fn entry_is_post_navigation(transition_type: &str) -> bool {
    // 归一化：小写 + 去 '_'，使 "form_submit" / "FormSubmit" / "FORM_SUBMIT" 都映射到 "formsubmit"。
    let normalized: String = transition_type
        .chars()
        .filter(|c| *c != '_')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    normalized == "formsubmit"
}

/// **[纯逻辑] 从 `Page.getNavigationHistory` 回包 + currentIndex 判当前页是否 POST 导航来的**
/// （D4 reload→IRREVERSIBLE 检测；[`entry_is_post_navigation`] 的 JSON 取数封装）。
///
/// 喂入 `entries`（`NavigationEntry[]` 的 JSON 数组）+ `current_index`：取 `entries[current_index]`
/// 的 `transitionType`，过 [`entry_is_post_navigation`]。取不到当前 entry / 缺 transitionType →
/// `false`（保守不误判，见 [`entry_is_post_navigation`] 的「拿不准默认」）。抽纯函数便于喂构造 JSON 单测。
pub fn current_entry_is_post(entries: &serde_json::Value, current_index: i64) -> bool {
    let Some(arr) = entries.as_array() else {
        return false;
    };
    let Ok(idx) = usize::try_from(current_index) else {
        return false;
    };
    let Some(entry) = arr.get(idx) else {
        return false;
    };
    entry
        .get("transitionType")
        .and_then(|v| v.as_str())
        .map(entry_is_post_navigation)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── InflightCounter：加减语义 + 不下溢 ──────────────────────────────────────

    #[test]
    fn inflight_starts_at_zero_idle() {
        let c = InflightCounter::new();
        assert_eq!(c.inflight(), 0);
        assert!(c.is_idle());
    }

    #[test]
    fn inflight_request_increments_finish_decrements() {
        let mut c = InflightCounter::new();
        c.on_request_will_be_sent(false); // 新请求 A
        assert_eq!(c.inflight(), 1);
        assert!(!c.is_idle());
        c.on_request_will_be_sent(false); // 新请求 B
        assert_eq!(c.inflight(), 2);
        c.on_loading_finished(); // A 完成
        assert_eq!(c.inflight(), 1);
        c.on_loading_failed(); // B 失败
        assert_eq!(c.inflight(), 0);
        assert!(c.is_idle());
    }

    #[test]
    fn inflight_redirect_continuation_does_not_increment() {
        // requestWillBeSent 带 redirectResponse（重定向续发，同 requestId）→ 不 +1。
        let mut c = InflightCounter::new();
        c.on_request_will_be_sent(false); // 初始请求 → 1
        assert_eq!(c.inflight(), 1);
        c.on_request_will_be_sent(true); // 302 续发（同 requestId）→ 仍 1，不重复计
        assert_eq!(c.inflight(), 1);
        c.on_loading_finished(); // 最终 finish → 0
        assert_eq!(c.inflight(), 0);
    }

    #[test]
    fn inflight_does_not_underflow() {
        // 乱序 / Network.enable 前已在途请求的 finish 事件 → -1 钳到 0，绝不下溢 panic/回绕。
        let mut c = InflightCounter::new();
        c.on_loading_finished(); // 0 - 1 → 钳 0
        assert_eq!(c.inflight(), 0);
        c.on_loading_failed(); // 0 - 1 → 钳 0
        assert_eq!(c.inflight(), 0);
        assert!(c.is_idle());
        // 再正常 +1 仍正确（钳零没破坏内部状态）。
        c.on_request_will_be_sent(false);
        assert_eq!(c.inflight(), 1);
    }

    #[test]
    fn inflight_response_received_is_noop() {
        // responseReceived 不改计数（响应头到达 ≠ 请求完成）——本计数器没有 on_response 方法，
        // 即「不处理」就是 no-op；此测试以「请求未因 responseReceived 离开在途」的形式钉死语义：
        // 只有 finished/failed 才 -1。
        let mut c = InflightCounter::new();
        c.on_request_will_be_sent(false);
        assert_eq!(c.inflight(), 1, "仍在途（responseReceived 不在此计数器的事件集里）");
        c.on_loading_finished();
        assert_eq!(c.inflight(), 0);
    }

    // ── request_is_redirect / extract_main_doc_status：params 形状解析 ──────────────

    #[test]
    fn request_is_redirect_detects_redirect_response() {
        let plain = serde_json::json!({"requestId": "1", "documentURL": "https://a.com"});
        assert!(!request_is_redirect(&plain));
        let redir = serde_json::json!({"requestId": "1", "redirectResponse": {"status": 302}});
        assert!(request_is_redirect(&redir));
    }

    #[test]
    fn extract_main_doc_status_only_for_main_frame_document() {
        let main = "FRAME_MAIN";
        // 主帧 Document 响应 → Some(status)。
        let p = serde_json::json!({
            "type": "Document",
            "frameId": "FRAME_MAIN",
            "response": {"status": 200, "url": "https://a.com"}
        });
        assert_eq!(extract_main_doc_status(&p, main), Some(200));
        // 302 也是合法 status。
        let p302 = serde_json::json!({
            "type": "Document", "frameId": "FRAME_MAIN", "response": {"status": 302}
        });
        assert_eq!(extract_main_doc_status(&p302, main), Some(302));
        // 子资源（type != Document）→ None。
        let sub = serde_json::json!({
            "type": "Script", "frameId": "FRAME_MAIN", "response": {"status": 200}
        });
        assert_eq!(extract_main_doc_status(&sub, main), None);
        // 别的帧的 Document（子 iframe 文档）→ None（不是这次主帧导航的状态）。
        let other = serde_json::json!({
            "type": "Document", "frameId": "FRAME_CHILD", "response": {"status": 404}
        });
        assert_eq!(extract_main_doc_status(&other, main), None);
        // 缺字段 → None（不 panic）。
        assert_eq!(extract_main_doc_status(&serde_json::json!({}), main), None);
    }

    // ── url_normalize：trailing-slash / 默认端口 / fragment / query 保留 ────────────

    #[test]
    fn url_normalize_removes_trailing_slash() {
        assert_eq!(url_normalize("https://a.com/"), url_normalize("https://a.com"));
        assert_eq!(url_normalize("https://a.com/path/"), url_normalize("https://a.com/path"));
        // 多重末尾斜杠也归一。
        assert_eq!(url_normalize("https://a.com///"), url_normalize("https://a.com"));
    }

    #[test]
    fn url_normalize_removes_default_port() {
        assert_eq!(url_normalize("http://a.com:80/x"), url_normalize("http://a.com/x"));
        assert_eq!(url_normalize("https://a.com:443/x"), url_normalize("https://a.com/x"));
        // 非默认端口保留（不归一）。
        assert_ne!(url_normalize("https://a.com:8443/x"), url_normalize("https://a.com/x"));
        // http 的 443 不是默认端口 → 保留。
        assert_ne!(url_normalize("http://a.com:443/x"), url_normalize("http://a.com/x"));
    }

    #[test]
    fn url_normalize_removes_fragment() {
        assert_eq!(url_normalize("https://a.com/p#sec"), url_normalize("https://a.com/p"));
        assert_eq!(url_normalize("https://a.com/#top"), url_normalize("https://a.com"));
    }

    #[test]
    fn url_normalize_preserves_query() {
        // query 改变即不同资源 → 归一后仍不等。
        assert_ne!(url_normalize("https://a.com/p?a=1"), url_normalize("https://a.com/p"));
        assert_ne!(url_normalize("https://a.com/p?a=1"), url_normalize("https://a.com/p?a=2"));
        // 同 query 相等。
        assert_eq!(url_normalize("https://a.com/p?a=1"), url_normalize("https://a.com/p?a=1"));
        // query 后的 fragment 仍被去除，query 保留。
        assert_eq!(
            url_normalize("https://a.com/p?a=1#x"),
            url_normalize("https://a.com/p?a=1")
        );
    }

    #[test]
    fn url_normalize_lowercases_scheme_and_host() {
        assert_eq!(url_normalize("HTTPS://A.COM/Path"), url_normalize("https://a.com/Path"));
        // path 大小写敏感 → 保留（不归一）。
        assert_ne!(url_normalize("https://a.com/Path"), url_normalize("https://a.com/path"));
    }

    #[test]
    fn url_normalize_non_http_best_effort() {
        // 非 scheme://host 形态：尽力而为去末尾斜杠 + 去 fragment，不 panic。
        assert_eq!(url_normalize("about:blank"), "about:blank");
        assert_eq!(url_normalize("data:text/html,<p>x</p>"), "data:text/html,<p>x</p>");
    }

    // ── is_redirect：同 URL 归一后 false、真跳转 true ──────────────────────────────

    #[test]
    fn is_redirect_false_for_normalized_equivalents() {
        // trailing-slash 不算 redirect（最常见误报源）。
        assert!(!is_redirect("https://a.com", "https://a.com/"));
        // 默认端口不算。
        assert!(!is_redirect("https://a.com/x", "https://a.com:443/x"));
        // fragment 不算。
        assert!(!is_redirect("https://a.com/x", "https://a.com/x#frag"));
        // 完全相同。
        assert!(!is_redirect("https://a.com/x", "https://a.com/x"));
    }

    #[test]
    fn is_redirect_true_for_real_navigation_change() {
        // host 变（典型登录墙跳转）。
        assert!(is_redirect("https://a.com/", "https://login.a.com/"));
        // path 变。
        assert!(is_redirect("https://a.com/x", "https://a.com/y"));
        // scheme 升级 http→https。
        assert!(is_redirect("http://a.com/", "https://a.com/"));
        // query 出现（带 ?next= 的登录跳转）。
        assert!(is_redirect("https://a.com/login", "https://a.com/login?next=/home"));
        // 非默认端口变化。
        assert!(is_redirect("https://a.com/x", "https://a.com:8443/x"));
    }

    // ── 生命周期分类 + settle 状态机推进 ──────────────────────────────────────────

    #[test]
    fn classify_lifecycle_maps_known_methods() {
        assert_eq!(
            classify_lifecycle("Page.domContentEventFired"),
            Some(LifecycleSignal::DomContentLoaded)
        );
        assert_eq!(classify_lifecycle("Page.loadEventFired"), Some(LifecycleSignal::Load));
        assert_eq!(
            classify_lifecycle("Page.navigatedWithinDocument"),
            Some(LifecycleSignal::NavigatedWithinDocument)
        );
        // 无关事件 → None。
        assert_eq!(classify_lifecycle("Network.requestWillBeSent"), None);
        assert_eq!(classify_lifecycle("Page.frameNavigated"), None);
    }

    #[test]
    fn advance_settle_monotonic_no_regress() {
        use LifecycleSignal as Sig;
        use NavSettleState as St;
        // commit → DCL → load 单调推进。
        assert_eq!(advance_settle(St::Commit, Sig::DomContentLoaded), St::DomContentLoaded);
        assert_eq!(advance_settle(St::DomContentLoaded, Sig::Load), St::Load);
        // 已 Load 收到迟到的 DCL → 不回退（取 max）。
        assert_eq!(advance_settle(St::Load, Sig::DomContentLoaded), St::Load);
        // 软导航不推进 load 阶梯（保持当前）。
        assert_eq!(advance_settle(St::Commit, Sig::NavigatedWithinDocument), St::Commit);
        assert_eq!(advance_settle(St::Load, Sig::NavigatedWithinDocument), St::Load);
    }

    #[test]
    fn nav_settle_state_ordering() {
        // Ord：Commit < DomContentLoaded < Load（max 推进的基础）。
        assert!(NavSettleState::Commit < NavSettleState::DomContentLoaded);
        assert!(NavSettleState::DomContentLoaded < NavSettleState::Load);
    }

    // ── D4：history 索引边界钳制（back/forward 不越界，良性无更多历史 → None）──────────

    #[test]
    fn history_back_from_middle_decrements() {
        // entries=[A,B,C]，当前在 B(idx 1) → back 目标 A(idx 0)。
        assert_eq!(history_target_index(1, 3, HistoryNav::Back), Some(0));
    }

    #[test]
    fn history_forward_from_middle_increments() {
        // entries=[A,B,C]，当前在 B(idx 1) → forward 目标 C(idx 2)。
        assert_eq!(history_target_index(1, 3, HistoryNav::Forward), Some(2));
    }

    #[test]
    fn history_back_at_first_page_is_none() {
        // 已在首页（idx 0）→ back 无更多历史 → None（良性，不越界、不 panic）。
        assert_eq!(history_target_index(0, 3, HistoryNav::Back), None);
    }

    #[test]
    fn history_forward_at_last_page_is_none() {
        // 已在末页（idx 2，len 3）→ forward 无更多历史 → None（良性钳制）。
        assert_eq!(history_target_index(2, 3, HistoryNav::Forward), None);
    }

    #[test]
    fn history_single_entry_both_directions_none() {
        // 只有一个 entry（idx 0，len 1）：back/forward 都无目标。
        assert_eq!(history_target_index(0, 1, HistoryNav::Back), None);
        assert_eq!(history_target_index(0, 1, HistoryNav::Forward), None);
    }

    #[test]
    fn history_empty_or_malformed_is_none_never_panics() {
        // 空历史（len 0）：任何方向 None（防越界）。
        assert_eq!(history_target_index(0, 0, HistoryNav::Back), None);
        assert_eq!(history_target_index(0, 0, HistoryNav::Forward), None);
        // 负 current_index（畸形）：back 不下溢 panic，返 None。
        assert_eq!(history_target_index(-1, 3, HistoryNav::Back), None);
        assert_eq!(history_target_index(i64::MIN, 3, HistoryNav::Back), None);
        // current_index 超出 entries（畸形）：forward 越界 → None。
        assert_eq!(history_target_index(5, 3, HistoryNav::Forward), None);
        assert_eq!(history_target_index(i64::MAX, 3, HistoryNav::Forward), None);
    }

    // ── D4：POST 页 reload → IRREVERSIBLE 检测（POST 页→true / GET 页→false）─────────────

    #[test]
    fn post_navigation_detected_for_form_submit_transition() {
        // form_submit transition = POST 表单提交页 → reload 会重提交 → IRREVERSIBLE。
        assert!(entry_is_post_navigation("form_submit"));
        // CDP 也可能给 CamelCase（兼容大小写）。
        assert!(entry_is_post_navigation("FormSubmit"));
        assert!(entry_is_post_navigation("FORM_SUBMIT"));
    }

    #[test]
    fn get_navigation_is_not_post() {
        // 普通 GET 导航（link/typed/reload/auto_subframe/…）→ reload 幂等，非不可逆。
        assert!(!entry_is_post_navigation("link"));
        assert!(!entry_is_post_navigation("typed"));
        assert!(!entry_is_post_navigation("reload"));
        assert!(!entry_is_post_navigation("auto_subframe"));
        assert!(!entry_is_post_navigation("address_bar"));
        // 空 / 未知 → 保守不误判（拿不准默认非 IRREVERSIBLE）。
        assert!(!entry_is_post_navigation(""));
        assert!(!entry_is_post_navigation("frobnicate"));
    }

    #[test]
    fn current_entry_is_post_reads_transition_at_current_index() {
        // entries=[GET A, POST B]，currentIndex=1（在 POST 页 B）→ true。
        let entries = serde_json::json!([
            {"id": 1, "url": "https://a.com", "transitionType": "link"},
            {"id": 2, "url": "https://a.com/submit", "transitionType": "form_submit"}
        ]);
        assert!(current_entry_is_post(&entries, 1), "current is the POST entry");
        // currentIndex=0（在 GET 页 A）→ false。
        assert!(!current_entry_is_post(&entries, 0), "current is the GET entry");
    }

    #[test]
    fn current_entry_is_post_conservative_on_bad_shapes() {
        let entries = serde_json::json!([
            {"id": 1, "url": "https://a.com", "transitionType": "form_submit"}
        ]);
        // 越界 currentIndex → false（保守，不 panic）。
        assert!(!current_entry_is_post(&entries, 5));
        assert!(!current_entry_is_post(&entries, -1));
        // 缺 transitionType 字段 → false（拿不准默认非 IRREVERSIBLE）。
        let no_transition = serde_json::json!([{"id": 1, "url": "https://a.com"}]);
        assert!(!current_entry_is_post(&no_transition, 0));
        // 非数组（畸形回包）→ false。
        assert!(!current_entry_is_post(&serde_json::json!({}), 0));
        assert!(!current_entry_is_post(&serde_json::Value::Null, 0));
    }
}
