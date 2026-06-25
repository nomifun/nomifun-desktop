//! **Tab 注册表 + active_target 指针**（P2 D1，DESIGN §13 裁决⑥）。
//!
//! P0/P1 的 [`crate::backend::cdp::CdpBackend`] 把「单 tab 的 per-tab 字段」（page session /
//! 注入管线 / OOPIF 管理表 / ref 表 / 主帧 id）**直挂**在 backend 上。P2 引入多 tab 路由：把这
//! 些 per-tab 字段下放进 [`TabRecord`]，建立 `tabs: HashMap<targetId, TabRecord>` 注册表 +
//! `active_target: targetId` 指针。observe/act/navigate 默认作用在 active tab 上。
//!
//! **D1 范围**：只建注册表骨架 + active 解引用 + last4 撞号去歧义。tab 发现循环 / switch /
//! close / open（多 tab 真填充）是 D3。**单 tab 场景**：`tabs` 恒只 1 项，`active_target` 指向
//! 它——行为与改造前完全一致。
//!
//! ## 锁设计（最关键，避免死锁 / 跨 await 持 `tabs` 锁）
//! observe/act/navigate **绝不**全程持 `tabs` 锁（会阻塞 D3 的 tab 发现循环，且 observe 内部还
//! 要锁 `ref_table` → 嵌套死锁）。模式：进操作时短暂锁 `tabs` + `active_target`，**克隆出**所需
//! 句柄（[`TabHandles`]），**立即释放 `tabs` 锁**，之后用克隆出的句柄操作。这就是 `ref_table`
//! 升级成 `Arc<AsyncMutex<..>>` 的原因——能 clone Arc 出来独立锁，不必持 `tabs` 锁跨 observe 的
//! 多 await。所有方法经 [`crate::backend::cdp::CdpBackend::active_tab_handles`] 拿句柄。

use std::collections::HashMap;

use crate::aria_ref::RefTable;
use crate::debug_capture::DebugBuffers;
use crate::injected::InjectionManager;

/// 一个被 backend 纳管的标签页：吸收原先直挂在 [`crate::backend::cdp::CdpBackend`] 上的全部
/// **per-tab** 状态。`tabs: HashMap<targetId, TabRecord>` 的值。
///
/// **保活字段**（`_inject_loop` / `_oopif_loop`）：注入管线 / OOPIF arm 循环的后台 `JoinHandle`，
/// 必须随 TabRecord 存活——否则 world 创建事件不再被收下、context 缓存停更。tab close（D3）即
/// 从 `tabs` 移除该 TabRecord，连带 drop 这俩 handle，后台循环自然退出。
///
/// **`ref_table` 为 `Arc<AsyncMutex<..>>`**（非裸 `AsyncMutex`）：observe/act 锁 `tabs` 时 clone
/// 出这个 Arc、释放 `tabs` 锁后再独立锁 ref_table 跨多 await——不必持 `tabs` 锁跨 observe（见模块
/// 级锁设计）。per-tab 隔离：每 tab 自有一张 ref 表（switch 无需作废逻辑，DESIGN 开放问题 4）。
pub struct TabRecord {
    /// 该 tab 的 page target 的 targetId（CDP 约定 == 主 frameId）。tabs 表的 key 即此。
    pub target_id: String,
    /// 该 tab page 的 CDP sessionId（原 `CdpBackend::page_session`；navigate/screenshot/输入发到它）。
    pub session_id: String,
    /// 该 tab 的注入管线（utility world 物化 + 逐帧 aria 注入；observe 的大脑）。
    /// `Clone` 友好：克隆共享 Arc 内部缓存，**不**复制后台循环（循环由 `_inject_loop` 保活）。
    pub injection: InjectionManager,
    /// 注入管线 arm 起的后台 context 登记循环句柄——**保活**。
    pub _inject_loop: tokio::task::JoinHandle<()>,
    /// 该 tab 的主帧 frameId（== page targetId，但以 frameTree 为权威）。observe 的根帧锚点。
    pub main_frame_id: String,
    /// 该 tab 的 OOPIF 子 session 注入管线表：sessionId → 已 arm 的 [`InjectionManager`]（+ loop 保活）。
    /// `Arc` 让 OOPIF arm 循环（`'static` 任务）与 observe 共享同一份真相（克隆 Arc 锁外用）。
    pub oopif_managers: std::sync::Arc<tokio::sync::Mutex<HashMap<String, OopifEntry>>>,
    /// OOPIF arm 后台循环句柄——保活。
    pub _oopif_loop: tokio::task::JoinHandle<()>,
    /// 该 tab 的代际 ref 表（per-tab 隔离）。`Arc<AsyncMutex<..>>`：clone Arc 出来独立锁，避免
    /// observe/act 跨 await 持 `tabs` 锁（见模块级锁设计）。
    pub ref_table: std::sync::Arc<tokio::sync::Mutex<Option<RefTable>>>,
    /// **per-tab 调试缓冲**（console/errors/network 有界环形缓冲）。`Arc<Mutex<..>>`：
    /// clone Arc 给读取动作（`GetConsoleLogs` 等），独立锁不持 `tabs` 锁。
    pub debug: std::sync::Arc<std::sync::Mutex<DebugBuffers>>,
    /// 调试事件收集后台循环句柄——**保活**。订阅 Runtime/Log/Network 事件写入 `debug` 缓冲。
    /// 关 tab 时 `.abort()`（同 `_inject_loop`/`_oopif_loop` 纪律）。
    pub _debug_loop: tokio::task::JoinHandle<()>,
}

/// 一个已 arm 的 OOPIF 子 session 注入管线 + 其后台 loop 句柄（保活）。
/// （从 [`crate::backend::cdp`] 下放至此，与 [`TabRecord`] 同模块——per-tab OOPIF 表的值。）
pub struct OopifEntry {
    pub manager: InjectionManager,
    pub _loop: tokio::task::JoinHandle<()>,
}

/// **active tab 的句柄快照**（[`crate::backend::cdp::CdpBackend::active_tab_handles`] 返回）：
/// 从 active [`TabRecord`] **克隆出**的全部可独立持有的句柄。observe/act/navigate 进操作时短暂
/// 锁 `tabs`+`active_target` 拿到它后**立即释放 `tabs` 锁**，之后全程用本快照操作——兑现「不跨
/// await 持 `tabs` 锁」（避免阻塞 D3 tab 发现循环 + observe 内嵌套锁 ref_table 死锁）。
///
/// 克隆成本低且语义正确：`session_id`/`target_id`/`main_frame_id` 是 `String`；`injection` 是
/// `InjectionManager`（Clone 共享 Arc 缓存，不复制后台循环）；`oopif_managers`/`ref_table` 是 `Arc`
/// （clone Arc 共享同一真相，可锁外独立锁）。
#[derive(Clone)]
pub struct TabHandles {
    /// active tab 的 targetId。
    pub target_id: String,
    /// active tab 的 page sessionId。
    pub session_id: String,
    /// active tab 的注入管线（克隆，共享 Arc 缓存）。
    pub injection: InjectionManager,
    /// active tab 的主帧 frameId。
    pub main_frame_id: String,
    /// active tab 的 OOPIF 管理表（克隆 Arc，锁外独立锁）。
    pub oopif_managers: std::sync::Arc<tokio::sync::Mutex<HashMap<String, OopifEntry>>>,
    /// active tab 的代际 ref 表（克隆 Arc，锁外独立锁——observe 跨多 await 不持 tabs 锁）。
    pub ref_table: std::sync::Arc<tokio::sync::Mutex<Option<RefTable>>>,
    /// active tab 的调试缓冲（克隆 Arc，读取动作用）。
    pub debug: std::sync::Arc<std::sync::Mutex<DebugBuffers>>,
}

/// **[纯逻辑] 取 targetId 的末 4 字符**（DESIGN §13：tab_id 用 targetId 末 4 位，对 LLM 友好）。
///
/// 不足 4 字符则全取（短 targetId 极罕见——真 CDP targetId 是 32 hex，但稳健起见兜底）。按
/// **字符**（非字节）取末 4 个 `char`，对全 ASCII 的 targetId 与字节切片等价，但对非 ASCII 不 panic。
pub fn last4(target_id: &str) -> String {
    let chars: Vec<char> = target_id.chars().collect();
    let start = chars.len().saturating_sub(4);
    chars[start..].iter().collect()
}

/// **[纯逻辑] last4 撞号去歧义判定结果**（[`resolve_last4_among`] 返回；不进锁/不进浏览器，便于单测）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Last4Match {
    /// 唯一命中：返完整 targetId。
    Unique(String),
    /// 撞号（多个 targetId 末 4 位相同）：歧义，需更长前缀。带上命中的完整 id 列表供诊断。
    Ambiguous(Vec<String>),
    /// 零命中：没有 targetId 的末 4 位匹配。
    NotFound,
}

/// **[纯逻辑] 在一组 targetId 里按末 4 位匹配 `last4`，判唯一 / 撞号 / 零命中**（不进锁，便于单测）。
///
/// 匹配规则：对每个候选 `tid`，若 `tid` 以 `last4` **结尾**（即 `last4(tid) == last4`，等价于
/// `tid.ends_with(last4)` 当 `last4` 恰好 4 字符时；为稳健直接比对 `last4(tid)`）→ 命中。
/// - 唯一命中 → [`Last4Match::Unique`]（完整 targetId）；
/// - 多个命中（撞号）→ [`Last4Match::Ambiguous`]（让上层报「用更长前缀」）；
/// - 零命中 → [`Last4Match::NotFound`]。
///
/// **也接受用户直接给完整 targetId**：若某候选 `tid == last4`（整串相等，即 LLM 给了全 id 而非末 4）
/// 直接唯一命中——这让 `resolve_last4` 对「末 4 位」与「完整 id」两种输入都健壮（D3 switch_tab 友好）。
pub fn resolve_last4_among<'a, I>(needle: &str, candidates: I) -> Last4Match
where
    I: IntoIterator<Item = &'a str>,
{
    let mut hits: Vec<String> = Vec::new();
    for tid in candidates {
        // 完整 id 精确相等 → 直接唯一命中（LLM 给了全 id）。
        if tid == needle {
            return Last4Match::Unique(tid.to_string());
        }
        // 否则按末 4 位匹配。
        if last4(tid) == needle {
            hits.push(tid.to_string());
        }
    }
    match hits.len() {
        0 => Last4Match::NotFound,
        1 => Last4Match::Unique(hits.into_iter().next().unwrap()),
        _ => Last4Match::Ambiguous(hits),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// D3：tab 发现循环 / switch / close / open / tabs 的**纯逻辑**判定（不进锁/不进浏览器，
// 便于单测）。OOPIF arm 循环（spawn_oopif_arm_loop）与 tab 发现循环靠 type 分流防重复 arm：
// **发现循环只收 type=="page" 且不在 tabs 的顶层标签**；**OOPIF 循环只收 type=="iframe"
// 子 session**。两路对 attachedToTarget 各自筛 type，互不重叠（DESIGN §13 + 裁决⑥/不变量⑮）。
// ═══════════════════════════════════════════════════════════════════════════

/// **[纯逻辑] tab 发现循环是否应把某 attach 事件 arm 成一个新顶层标签**（不进锁/不进浏览器）。
///
/// 判据（与 [`crate::backend::cdp::spawn_oopif_arm_loop`] 的 type 分流互补，防重复 arm）：
/// 1. `target_type == "page"`（顶层标签）——**iframe / service_worker / 其它一律不收**（iframe 归
///    OOPIF arm 循环；二者各自筛 type 故互不重叠）。
/// 2. 该 page session **不是**主 page session（主 page 在 `from_launched` 已单独 arm，勿重 arm）。
/// 3. 该 targetId **不在** `tabs` 注册表（CDP 可能对同 target 多次 attach；已纳管的不重 arm）。
///
/// `is_main_page_session`：该 attach 的 sessionId 是否等于初始主 page session（发现循环建时捕获）。
/// `already_in_tabs`：该 targetId 是否已是 `tabs` 的 key。两者均由调用方在锁内查后传入（本函数纯判定）。
///
/// **opener/context 说明**：DESIGN §13 提到 window.open/target=_blank 的新 target 有 openerId；但
/// flatten setAutoAttach 下，**所有**新顶层 page（含用户 window.open、`open_link_new_tab` 的 createTarget）
/// 都以 `type=="page"` 的 attachedToTarget 到来。我们**按 type 收编所有新顶层 page**（不依赖 openerId 在场——
/// `open_link_new_tab` 经 createTarget 建的 background tab 可能无 openerId）。opener/parentFrameId 的真正
/// 用途是与 OOPIF 区分：OOPIF 是 `type=="iframe"` 且带 `parentFrameId`，**永不**命中本判定（type 已挡）。
pub fn should_arm_as_page(
    target_type: &str,
    is_main_page_session: bool,
    already_in_tabs: bool,
) -> bool {
    target_type == "page" && !is_main_page_session && !already_in_tabs
}

/// **[纯逻辑] OOPIF arm 循环是否应把某 attach 事件 arm 成一个跨进程子帧**（不进锁/不进浏览器）。
///
/// 判据（与 [`should_arm_as_page`] 的 type 分流**严格互补**，防一条 attach 被两路同时 arm）：
/// 1. `target_type == "iframe"`（**且仅 iframe**）——**`page`（兄弟顶层标签）/ service_worker / 其它
///    一律不收**。这是裁决⑥的核心：每个 per-tab OOPIF 循环订阅的是**全局** `Target.attachedToTarget`，
///    若放行 `type=="page"`，看到**兄弟顶层 tab**（另一个 page，sid≠自己）就会把它 arm 进自己的
///    `oopif_managers`，导致 observe 活动 tab 时把兄弟 tab 整页内容当 OOPIF 子帧拼进来（**跨标签
///    污染**）。顶层 page 归 [`should_arm_as_page`] 的 tab 发现循环管，OOPIF 循环只管 iframe 子帧。
/// 2. 该子 session **不是**本 tab 的主 page session（主 page 已单独 arm 注入管线，勿重 arm）。
/// 3. 该子 session **未**已在本 tab 的 `oopif_managers`（CDP 可能对同 target 多次 attach；已纳管不重 arm）。
///
/// `is_own_page_session`：该 attach 的 sessionId 是否等于本 OOPIF 循环所属 tab 的主 page session。
/// `already_armed`：该 sessionId 是否已是本 tab `oopif_managers` 的 key。两者由调用方查后传入（本函数纯判定）。
///
/// 真跨源 OOPIF 子帧（`type=="iframe"` 且跨进程另起子 session）仍照常命中本判定 → arm（不误伤
/// `TODO(verify-oopif)` 接线）；同进程 iframe（`file://` srcdoc / 同源）不另起子 session，本就不到这条路径。
pub fn should_arm_as_oopif(
    target_type: &str,
    is_own_page_session: bool,
    already_armed: bool,
) -> bool {
    target_type == "iframe" && !is_own_page_session && !already_armed
}

/// **一个标签页的对 LLM 摘要项**（[tabs 列表动作] 返回；`is_active` 标记当前 active tab）。
/// `last4` 是 targetId 末 4 位（对 LLM 友好的 tab_id）；url/title 取自 `Target.getTargets`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabListItem {
    /// targetId 末 4 位（LLM 用它 switch/close）。
    pub last4: String,
    /// 完整 targetId（诊断 / 撞号时给全 id）。
    pub target_id: String,
    /// 标签页当前 URL。
    pub url: String,
    /// 标签页标题（可能空）。
    pub title: String,
    /// 是否是当前 active tab（observe/act 默认作用其上）。
    pub is_active: bool,
}

/// **[纯逻辑] 把 tab 列表渲染成对 LLM 的多行文案**（不进浏览器，便于单测）。每行形如
/// `- [<last4>]<*> "<title>" <url>`（`*` 标记 active tab）。空列表 → 提示语。文案引导 LLM 用 last4
/// 做 switch_tab/close_tab。
pub fn render_tab_list(items: &[TabListItem]) -> String {
    if items.is_empty() {
        return "no tabs open".to_string();
    }
    let mut lines = Vec::with_capacity(items.len() + 1);
    lines.push(format!("{} tab(s) open (use the [id] with switch_tab/close_tab):", items.len()));
    for it in items {
        let active = if it.is_active { " (active)" } else { "" };
        let title = if it.title.is_empty() {
            String::new()
        } else {
            format!(" {:?}", it.title)
        };
        lines.push(format!("- [{}]{active}{title} {}", it.last4, it.url));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last4_extracts_last_four_chars() {
        // 标准 32-hex CDP targetId：取末 4 位。
        assert_eq!(last4("ABCDEF0123456789ABCDEF0123456789"), "6789");
        assert_eq!(last4("0000000000000000000000000000ABCD"), "ABCD");
        // 恰好 4 字符：全取。
        assert_eq!(last4("WXYZ"), "WXYZ");
    }

    #[test]
    fn last4_handles_shorter_than_four() {
        // 不足 4 字符 → 全取（不 panic）。
        assert_eq!(last4("AB"), "AB");
        assert_eq!(last4("X"), "X");
        assert_eq!(last4(""), "");
        // 恰 3 字符。
        assert_eq!(last4("abc"), "abc");
    }

    #[test]
    fn last4_non_ascii_does_not_panic() {
        // 按 char 取末 4（非字节）：非 ASCII 不在字符边界 panic。
        assert_eq!(last4("页一二三四五"), "二三四五");
        assert_eq!(last4("ab页"), "ab页");
    }

    #[test]
    fn resolve_last4_unique_hit() {
        // 唯一末 4 命中 → Unique(完整 id)。
        let cands = ["AAAA1111", "BBBB2222", "CCCC3333"];
        assert_eq!(
            resolve_last4_among("2222", cands),
            Last4Match::Unique("BBBB2222".into())
        );
    }

    #[test]
    fn resolve_last4_ambiguous_collision() {
        // 两个 targetId 末 4 相同 → Ambiguous（撞号歧义，需更长前缀）。
        let cands = ["AAAA9999", "BBBB9999", "CCCC3333"];
        match resolve_last4_among("9999", cands) {
            Last4Match::Ambiguous(ids) => {
                assert_eq!(ids.len(), 2);
                assert!(ids.contains(&"AAAA9999".to_string()));
                assert!(ids.contains(&"BBBB9999".to_string()));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn resolve_last4_not_found() {
        // 零命中 → NotFound。
        let cands = ["AAAA1111", "BBBB2222"];
        assert_eq!(resolve_last4_among("zzzz", cands), Last4Match::NotFound);
        // 空候选集 → NotFound。
        assert_eq!(resolve_last4_among("1111", std::iter::empty()), Last4Match::NotFound);
    }

    #[test]
    fn resolve_last4_accepts_full_target_id() {
        // LLM 给完整 targetId（非末 4）→ 精确相等 → Unique。
        let cands = ["AAAA1111", "BBBB2222"];
        assert_eq!(
            resolve_last4_among("AAAA1111", cands),
            Last4Match::Unique("AAAA1111".into())
        );
    }

    #[test]
    fn resolve_last4_single_tab_is_unique() {
        // 单 tab 场景（D1 恒态）：tabs 恒 1 项，按其末 4 解析必唯一命中。
        let cands = ["ABCDEF0123456789ABCDEF0123456789"];
        assert_eq!(
            resolve_last4_among("6789", cands),
            Last4Match::Unique("ABCDEF0123456789ABCDEF0123456789".into())
        );
    }

    // ── D3：tab 发现循环 type 分流 + tab 列表渲染（[纯逻辑]，喂构造值，不进浏览器）──

    #[test]
    fn should_arm_as_page_only_new_top_level_page() {
        // 真新顶层 page（type=page，非主 session，不在 tabs）→ 应 arm。
        assert!(should_arm_as_page("page", false, false));
    }

    #[test]
    fn should_arm_as_page_skips_iframe_oopif() {
        // iframe 子 session（OOPIF）→ type 已挡（归 spawn_oopif_arm_loop），发现循环不收（防重复 arm）。
        assert!(!should_arm_as_page("iframe", false, false));
        // service_worker / 其它非 page 类型同样不收。
        assert!(!should_arm_as_page("service_worker", false, false));
        assert!(!should_arm_as_page("worker", false, false));
        assert!(!should_arm_as_page("other", false, false));
    }

    #[test]
    fn should_arm_as_page_skips_main_page_session() {
        // 主 page session（from_launched 已单独 arm）→ 即便 type=page 也不重 arm。
        assert!(!should_arm_as_page("page", true, false));
    }

    #[test]
    fn should_arm_as_page_skips_already_armed_tab() {
        // 已在 tabs（CDP 对同 target 多次 attach）→ 不重复 arm（tabs map 无重复 key）。
        assert!(!should_arm_as_page("page", false, true));
        // 同时是主 session 且已纳管：两层守卫都该挡。
        assert!(!should_arm_as_page("page", true, true));
    }

    // ── D3 fix（裁决⑥）：OOPIF arm 循环 type 分流——**只收 iframe，绝不收 page**（防兄弟顶层 tab
    //    误 arm 致跨标签 observe 污染）。与 should_arm_as_page 严格互补，同一 attach 不被两路同时收。

    #[test]
    fn should_arm_as_oopif_only_iframe() {
        // 真跨进程 OOPIF 子帧（type=iframe，非本 page session，未 armed）→ 应 arm。
        assert!(should_arm_as_oopif("iframe", false, false));
    }

    #[test]
    fn should_arm_as_oopif_rejects_sibling_top_level_page() {
        // **核心回归（裁决⑥）**：兄弟顶层 tab（type=page）即便不是本 page session、未 armed，OOPIF
        // 循环也**绝不收**——否则会把兄弟整页当 OOPIF 子帧拼进活动 tab 的 observe（跨标签污染）。
        assert!(
            !should_arm_as_oopif("page", false, false),
            "OOPIF loop must reject type=page (sibling top-level tab) to avoid cross-tab pollution"
        );
        // service_worker / worker / 其它同样不收（只 iframe）。
        assert!(!should_arm_as_oopif("service_worker", false, false));
        assert!(!should_arm_as_oopif("worker", false, false));
        assert!(!should_arm_as_oopif("other", false, false));
    }

    #[test]
    fn should_arm_as_oopif_skips_own_and_already_armed() {
        // 本 tab 的主 page session（已单独 arm 注入管线）→ 即便 type=iframe 也不收（理论上主 session
        // 不会以 iframe 来，守卫为防御）。
        assert!(!should_arm_as_oopif("iframe", true, false));
        // 已 armed（同 target 多次 attach）→ 不重复 arm。
        assert!(!should_arm_as_oopif("iframe", false, true));
    }

    #[test]
    fn should_arm_as_page_and_oopif_are_mutually_exclusive() {
        // 互补不变量：对任一 type，两路不可能同时返 true（防同一 attach 被两路 arm）。
        for ttype in ["page", "iframe", "service_worker", "worker", "browser", "other", ""] {
            let as_page = should_arm_as_page(ttype, false, false);
            let as_oopif = should_arm_as_oopif(ttype, false, false);
            assert!(
                !(as_page && as_oopif),
                "type {ttype:?} must not arm as both page and oopif"
            );
        }
    }

    #[test]
    fn render_tab_list_marks_active_and_uses_last4() {
        let items = vec![
            TabListItem {
                last4: "1111".into(),
                target_id: "AAAA1111".into(),
                url: "https://a.example/".into(),
                title: "Page A".into(),
                is_active: true,
            },
            TabListItem {
                last4: "2222".into(),
                target_id: "BBBB2222".into(),
                url: "https://b.example/".into(),
                title: String::new(),
                is_active: false,
            },
        ];
        let out = render_tab_list(&items);
        assert!(out.contains("2 tab(s) open"), "out:\n{out}");
        // active tab 标 (active)，并带 last4 + 标题 + url。
        assert!(out.contains("[1111] (active) \"Page A\" https://a.example/"), "out:\n{out}");
        // 非 active tab 无 (active) 标记；空标题不渲染引号串。
        assert!(out.contains("[2222] https://b.example/"), "out:\n{out}");
        assert!(!out.contains("[2222] (active)"), "non-active must not be marked active:\n{out}");
    }

    #[test]
    fn render_tab_list_empty_is_friendly() {
        assert_eq!(render_tab_list(&[]), "no tabs open");
    }
}
