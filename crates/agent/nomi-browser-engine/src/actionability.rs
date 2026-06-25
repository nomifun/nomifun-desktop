//! **act 反查链：`f<seq>e<n>` ref → DOM element objectId**（P2 命脉，DESIGN §7 裁决①）。
//!
//! observe（P1）跑 `incrementalAriaSnapshot` 时，注入侧把每个被分配 ref 的元素缓存进当帧
//! utility world 的 `_lastAriaSnapshotForQuery.elements: Map<ref, Element>`（injectedScript.ts:321），
//! 并给元素打 `_ariaRef = {role, name, ref}` expando（ariaSnapshot.ts:214）。act 要操作 LLM 给的
//! ref，必须把它反解回**当前活的** DOM 元素句柄（objectId）。
//!
//! ## 反查三层（陈旧检测分层；廉价 → 昂贵）
//! 1. **层①（廉价，纯 Rust，不进浏览器）**：[`crate::aria_ref::RefTable::resolve`]。LLM 给的 ref
//!    不在**当前代际**表里（旧代际遗留 / 拼错）→ `None` → 调用方报 [`BrowserError::NodeStale`]。
//!    本模块只在层① 通过（拿到 [`RefRecord`]）后才下钻。
//! 2. **层②（中等，进浏览器一次 call）**：在元素所属帧的 utility world，用注入侧
//!    `parseSelector("aria-ref="+ref)` + `querySelector(parsed, document, false)` 跑 vendored
//!    `aria-ref` selector engine（injectedScript.ts:709-715）。该 engine 仅当
//!    `elements.get(ref)?.isConnected` 才返回元素，否则空。空 = 元素已从活 DOM detach（代际内漂移）
//!    → [`BrowserError::NotConnected`]。
//! 3. **层③（精确，读元素 expando）**：在层② 拿到的元素 objectId 上读 `_ariaRef.{role,name}`，与
//!    [`RefRecord`] 的 role/name 二次比对。**role 不符** → [`BrowserError::NodeStale`]（防
//!    backendNodeId 被浏览器复用后 ref 映射到一个**不同角色**的新元素，静默点错）。name 宽松（动态
//!    文案常变，仅诊断记录，不据此判 stale —— P2 契约：「至少做 role 比对，name 可宽松」）。
//!
//! ## objectGroup 生命周期
//! 反查时给 `Runtime.callFunctionOn` 传 `objectGroup = "act-<seq>"`，返回的元素句柄归入该组。
//! 一次动作结束（成功/失败均然）调 [`crate::injected::InjectionManager::release_object_group`]
//! 一次释放该组全部句柄，防 CDP 端 RemoteObject 泄漏。
//!
//! 本模块**只**做 ref→objectId 反查 + objectGroup + role 校验；五检查 / hit-target / 输入 / act
//! 动作 / facade 是后续任务，不在此处。

use chromiumoxide::cdp::js_protocol::runtime::CallArgument;
use serde_json::Value;

use crate::aria_ref::RefRecord;
use crate::backend::cdp::{map_inject_err, CdpBackend};
use crate::engine::BrowserError;
use crate::injected::{InjectError, InjectionManager};
use crate::input::Point;

/// 反查到的一个活元素句柄：utility world 里的 `objectId` + 它归属的 `objectGroup`。
/// 动作结束后用 `group` 调 [`InjectionManager::release_object_group`] 整组释放。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectHandle {
    /// utility world 里的元素 RemoteObject id（喂后续五检查 / hit-target / 输入路径）。
    pub object_id: String,
    /// 该句柄归属的 objectGroup 名（`act-<seq>`）；动作收尾 releaseObjectGroup 用。
    pub group: String,
}

/// **actionability 五检查中 visible/stable/enabled/editable 四检查的三态结果**（DESIGN §11，
/// 设计裁决②）。点击/输入前由 vendored PW 的 `checkElementStates`（injectedScript.ts:640）**批量**
/// 判定，本枚举是 Rust 侧对其返回值（`undefined` | `{missingState}` | `'error:notconnected'`）的
/// 三态翻译。第⑤检查 receivesEvents（hit-target）不在 `checkElementStates`，由 B4 承担。
///
/// **谁消费**：B6 重试编排——`Pass` 放行执行动作；`Missing(state)` 是**可重试**的瞬态缺态
/// （元素当前不可见/未稳定/禁用/暂只读，等一拍可能就绪）；`NotConnected` 是元素已从活 DOM
/// detach（漂移），调用方报 [`BrowserError::NotConnected`]。
///
/// **不含 editable 的不可编辑特例**：元素**类型根本不支持编辑**（非 input/textarea/select/
/// contenteditable 且无允许 aria-readonly 的 role）→ 注入侧 `createStacklessError` 经
/// exceptionDetails 抛 → Rust 收 [`InjectError::JsException`] → [`check_states`] 直接返
/// `Err(BrowserError::Blocked)`（NonRecoverable，**禁重试**），**不**走本枚举。区别于
/// **暂时 readonly**（`received:'readOnly'`）—— 那是正常返回的 `Missing("editable")`（可重试）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckResult {
    /// 全部待检状态都满足（`checkElementStates` 返 `undefined`/null）。放行执行动作。
    Pass,
    /// 某个状态当前不满足（`{missingState:<state>}`）。**可重试**的瞬态缺态（B6 等一拍重判）。
    Missing(String),
    /// 元素已从活 DOM detach（`'error:notconnected'`）。漂移，调用方报 NotConnected。
    NotConnected,
}

/// **[纯逻辑] 解析 `checkElementStates` 的返回 RemoteObject `value`** 成 [`CheckResult`]。
///
/// `checkElementStates` 是 async 返 `'error:notconnected' | { missingState } | undefined`；经
/// `callFunctionOn(await_promise=true, return_by_value=true)` 回来后，`result.value` 就是该 promise
/// 的 resolved 值（`undefined` 在 by-value 下表现为**缺字段**或 JSON `null`）。本函数喂入那个
/// `value`（`None` = 缺 `value` 字段 = JS `undefined`）：
/// - `None` / `Value::Null` → [`CheckResult::Pass`]（全过）；
/// - `Value::String("error:notconnected")` → [`CheckResult::NotConnected`]；
/// - `{ "missingState": "<state>" }` → [`CheckResult::Missing("<state>")`]；
/// - 任何其它形状（不该发生：注入契约只产上述三态）→ 保守当 `NotConnected`（宁可让上层重新
///   observe，也不静默放行一个形状陌生的结果去点击）。
///
/// 抽成纯函数（不进浏览器）以便 `#[cfg(test)]` 喂构造的 `serde_json::Value` 单测三态解析。
pub fn parse_check_result(value: Option<&Value>) -> CheckResult {
    match value {
        // undefined（by-value 下缺字段）或显式 null → 全过。
        None | Some(Value::Null) => CheckResult::Pass,
        Some(Value::String(s)) if s == "error:notconnected" => CheckResult::NotConnected,
        Some(Value::Object(map)) => match map.get("missingState").and_then(|v| v.as_str()) {
            Some(state) => CheckResult::Missing(state.to_string()),
            // 对象但无 missingState：形状陌生，保守判 NotConnected（不放行点击）。
            None => CheckResult::NotConnected,
        },
        // 其它标量（数字/bool/非 notconnected 字符串/数组）：注入契约不产，保守 NotConnected。
        Some(_) => CheckResult::NotConnected,
    }
}

/// **[纯逻辑] editable 异常分类**：判某条注入侧异常 message 是否是「元素类型根本不支持编辑」
/// （`elementState('editable')` 里 `getReadonly` 返 `'error'` → `createStacklessError`，
/// injectedScript.ts:744-745）。返回 `true` = NonRecoverable（[`check_states`] 据此返 `Blocked` 禁
/// 重试）；`false` = 其它注入异常（按普通 JsException → `Other` 处理，调用方可按需重试/上报）。
///
/// 判据：message 含 chromium 版 `createStacklessError('Element is not an <input>, …')` 的稳定特征
/// 子串。注入侧该文案是 vendored 常量（injectedScript.ts:745），匹配其稳定片段即可（含 `Error: `
/// 前缀的 `exception.description` 也命中，因子串匹配不锚首）。同理覆盖 checked/indeterminate 等
/// 「类型不支持该状态」异常（`Not a checkbox or radio button`），它们同样是 NonRecoverable 的
/// 「元素类型与待检状态不匹配」——但本任务只用到 editable，checked 类留作前瞻覆盖。
pub fn is_non_editable_error(message: &str) -> bool {
    message.contains("Element is not an <input>")
        || message.contains("does not have a role allowing [aria-readonly]")
        || message.contains("Not a checkbox or radio button")
}

/// **double hit-target 三步舞**（DESIGN §11，设计裁决③）的 setup 句柄：vendored PW
/// `setupHitTargetInterceptor` 在元素上**算点时**装的 capture-phase 拦截器对象的 RemoteObject id
/// （+ 它归属的 objectGroup）。这是 actionability 第⑤检查 receivesEvents（防误点遮挡元素）的
/// 承担者。
///
/// **为何全程 LIVE**：setup 经 `callFunctionOn(return_by_value=false)` 拿回**句柄**（不取值），让
/// interceptor 对象（含其 `stop()` 闭包，闭包捕获了 `result`/`listener`）在 CDP 端**存活**到点击
/// 之后；点击在 setup 与 [`CdpBackend::hit_stop`] 之间真分发，stop 时读「算点与事件分发间布局是否
/// 漂移 / 是否被遮挡」。interceptor 的 objectId 与传入动作同一 objectGroup（`act-<seq>`），随该组
/// 在动作收尾时一次 releaseObjectGroup 释放（生命周期收口，见 [`CdpBackend::release_act_group`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitInterceptor {
    /// setup 返回的 interceptor 对象的 RemoteObject id（by-handle 保活，喂 [`CdpBackend::hit_stop`]）。
    pub object_id: String,
    /// 该句柄归属的 objectGroup（与传入元素同组 `act-<seq>`）；动作收尾整组释放。
    pub group: String,
}

/// **[纯逻辑] setup 三态判定结果**：[`setup_result_outcome`] 把 `setupHitTargetInterceptor` 的
/// `callFunctionOn(return_by_value=false)` 回包 RemoteObject 翻成的三态（避免把 CDP 形状判定混进
/// async I/O，便于 `#[cfg(test)]` 喂构造的 `serde_json::Value` 单测）。
///
/// `setupHitTargetInterceptor`（injectedScript.ts:1067）返三种 JS 形态：
/// - **interceptor 对象**（`{ stop }`）—— 正常装好，by-handle 下回包有 `objectId`（subtype 非 null）；
/// - **`'error:notconnected'`** —— 元素已 detach，by-value 字符串（无 objectId，`value` 是该字串）；
/// - **bare `string`（hitTargetDescription）** —— `hitPoint` 给定时 setup 内的 `expectHitTarget` 预检
///   失败（已被遮挡），短路返描述字符串（无 objectId，`value` 是该描述）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupOutcome {
    /// interceptor 对象装好；带回它的 objectId（喂 [`CdpBackend::hit_stop`]）。
    Handle(String),
    /// 元素 detach（`'error:notconnected'`）。
    NotConnected,
    /// 预检短路：已被遮挡，带回 hitTargetDescription（→ `Blocked{reason}`，无需走点击）。
    Blocked(String),
}

/// **[纯逻辑] 解析 `setupHitTargetInterceptor` 的 `callFunctionOn(by_handle)` 回包**成 [`SetupOutcome`]。
///
/// 喂入整个 `result` 的 RemoteObject JSON（`call_on_injected_handle` 返回值）。判据：
/// - 有 `objectId`（subtype 非 null）→ interceptor 对象 → [`SetupOutcome::Handle`]；
/// - 无 objectId 但 `value == "error:notconnected"` → [`SetupOutcome::NotConnected`]；
/// - 无 objectId 且 `value` 是其它字符串 → 短路 hitTargetDescription → [`SetupOutcome::Blocked`]；
/// - 无 objectId 且 `value` 非字符串（不该发生：注入契约只产上述三态）→ 保守
///   [`SetupOutcome::NotConnected`]（宁可让上层重定位，也不静默放行一个形状陌生的结果）。
pub fn setup_result_outcome(result: &Value) -> SetupOutcome {
    // 有 objectId 即 interceptor 对象句柄（by-handle 保活）。
    if let Some(id) = result.get("objectId").and_then(|v| v.as_str()) {
        return SetupOutcome::Handle(id.to_string());
    }
    // 无 objectId：是字符串返回（'error:notconnected' 或 hitTargetDescription）。
    match result.get("value").and_then(|v| v.as_str()) {
        Some("error:notconnected") => SetupOutcome::NotConnected,
        Some(desc) => SetupOutcome::Blocked(desc.to_string()),
        // 形状陌生（无 objectId 也无字符串 value）：保守判 NotConnected。
        None => SetupOutcome::NotConnected,
    }
}

/// **[纯逻辑] 解析 `stop()` / `expectHitTarget()` 的 by-value 返回值**成 hit-target 判定。
///
/// 二者 JS 返回同形（DESIGN §11）：`'done'`（命中目标或其后代，放行）| `{ hitTargetDescription }`
/// （命中了别的元素 = 布局漂移误点 / 遮挡，拦下）。喂入 `result.value`（`call_on_element`/
/// `call_on_injected_handle` 的 by-value 回包 `.value`；`None` = 缺 `value` 字段）：
/// - `"done"` → `Ok(())`（放行）；
/// - `{ "hitTargetDescription": "<desc>" }` → `Err(Blocked{reason:<desc>})`；
/// - 任何其它形状（不该发生）→ 保守 `Err(Blocked)`（不静默放行一个形状陌生的结果去点击）。
///
/// 抽成纯函数（不进浏览器）以便 `#[cfg(test)]` 喂构造的 `serde_json::Value` 单测两态解析。
pub fn parse_hit_target_value(value: Option<&Value>) -> Result<(), BrowserError> {
    match value {
        Some(Value::String(s)) if s == "done" => Ok(()),
        Some(Value::Object(map)) => match map.get("hitTargetDescription").and_then(|v| v.as_str()) {
            Some(desc) => Err(BrowserError::Blocked {
                reason: desc.to_string(),
            }),
            // 对象但无 hitTargetDescription：形状陌生，保守 Blocked（不放行点击）。
            None => Err(BrowserError::Blocked {
                reason: format!("unexpected hit-target result shape: {value:?}"),
            }),
        },
        // 其它（None / 非 'done' 字符串 / 标量）：注入契约不产，保守 Blocked。
        other => Err(BrowserError::Blocked {
            reason: format!("unexpected hit-target result shape: {other:?}"),
        }),
    }
}

/// 拼 vendored `aria-ref` selector engine 的选择器字符串：`"aria-ref="+full_ref`。
/// `full_ref` 是含帧前缀的完整 ref（如 `f3e7`）—— injected 的
/// `_lastAriaSnapshotForQuery.elements` 正是以**完整 ref**（`refPrefix + 'e' + n`）为键
/// （ariaSnapshot.ts:137 `snapshot.elements.set(childAriaNode.ref, element)`），故必须传完整 ref，
/// 不能只传 frame-local 的 `e<n>`。
pub fn aria_ref_selector(full_ref: &str) -> String {
    format!("aria-ref={full_ref}")
}

/// objectGroup 名：一次动作的全部元素句柄归入 `act-<seq>`。`seq` 由动作分配（act 自增计数）。
pub fn act_object_group(seq: u64) -> String {
    format!("act-{seq}")
}

/// **层③ 漂移判定**：把层② 拿回的元素实际 role/name 与 [`RefRecord`] 比对，判断 ref 是否漂移。
///
/// **role 严格**：不符即漂移（`true`）—— backendNodeId 复用后同一 ref 映射到不同角色的新元素，
/// 必须拦下（否则静默点错）。
/// **name 宽松**：动态文案常变（计数、时间、i18n），**不据 name 判漂移**（P2 契约）。name 仅供
/// 诊断 —— 调用方可记录差异但不应据此报 stale。
///
/// 返回 `true` = 漂移（调用方报 [`BrowserError::NodeStale`]）；`false` = role 一致，放行。
pub fn ref_drifted(rec: &RefRecord, actual_role: &str, _actual_name: &str) -> bool {
    rec.role != actual_role
}

impl CdpBackend {
    /// 选某 RefRecord 所属帧的注入管线：主 page session → `self.injection`；否则查 `oopif_managers`。
    /// 找不到 → `None`（调用方报 NodeStale / SessionLost，按语义；OOPIF 子 session 可能已 detach）。
    /// `pub(crate)`：C1 的 [`crate::actions`] fill 路径据 record 选注入管线跑 `fill(node,value)`。
    pub(crate) async fn manager_for_record(&self, rec: &RefRecord) -> Option<InjectionManager> {
        // D1：page_session_id / injection_manager 现为 async（经 active tab 解引用）。active tab 缺失
        // → 返 None（调用方报 NodeStale / SessionLost）。
        if let Ok(active_session) = self.page_session_id().await
            && rec.session_id == active_session
        {
            return self.injection_manager().await.ok();
        }
        self.oopif_manager_for(&rec.session_id).await
    }

    /// **层①-only：LLM ref → [`RefRecord`]**（不进浏览器）。在**当前代际** ref 表里 `resolve(llm_ref)`：
    /// 命中返记录（克隆出来，锁外用）；不在（旧代际遗留 / 拼错 / 还没 observe）→ [`BrowserError::NodeStale`]
    /// （带当前代际，文案引导重新 observe）。act 主循环（C1）用它**先于** arm_act_abort 拿 frame_id，并在
    /// 每次重试 attempt 内重解析 ref（外层自愈）。
    /// **D1**：`ref_table_lock()` 现为 async 返 active tab 的 ref_table Arc（clone Arc 后独立锁）。
    pub async fn resolve_ref_record(&self, llm_ref: &str) -> Result<RefRecord, BrowserError> {
        let ref_table = self.ref_table_lock().await?;
        let guard = ref_table.lock().await;
        match guard.as_ref().and_then(|t| t.resolve(llm_ref)) {
            Some(r) => Ok(r.clone()),
            None => Err(BrowserError::NodeStale {
                generation: guard.as_ref().map(|t| t.generation().0).unwrap_or(0),
            }),
        }
    }

    /// **act 反查链公共入口（含层①）**：LLM 给的 ref 字符串 → 活元素 [`ObjectHandle`]。
    ///
    /// 层①（廉价，纯 Rust）：在**当前代际** ref 表里 `resolve(llm_ref)`；不在（旧代际遗留 / 拼错）
    /// → [`BrowserError::NodeStale`]（带当前代际），**不进浏览器**。命中（拿到 [`RefRecord`]）后委托
    /// [`Self::resolve_ref_to_object`] 跑层②（aria-ref selector → objectId）+ 层③（role 校验）。
    ///
    /// `seq` 用于本次动作的 objectGroup（`act-<seq>`）。act facade（C1）以此为统一入口。
    pub async fn resolve_ref(&self, llm_ref: &str, seq: u64) -> Result<ObjectHandle, BrowserError> {
        let rec = self.resolve_ref_record(llm_ref).await?;
        self.resolve_ref_to_object(&rec, seq).await
    }

    /// **act 反查链（层② + 层③）**：[`RefRecord`]（层① 已通过）→ 活元素 [`ObjectHandle`]。
    ///
    /// 步骤（层② + 层③）：
    /// 1. 选 rec 所属帧的注入管线（page session / OOPIF 子 session）。
    /// 2. 层②：在该帧 utility world `call_on_injected_handle` 跑 vendored `aria-ref=` selector：
    ///    `parseSelector` + `querySelector` 取元素**句柄**（by-handle, objectGroup=`act-<seq>`）。
    ///    元素不存在（aria-ref engine 因不在表 / not connected 返空）→ [`BrowserError::NotConnected`]。
    /// 3. 层③：在拿到的元素句柄上读 `_ariaRef.{role,name}`，与 rec 二次比对；role 不符 →
    ///    [`BrowserError::NodeStale`]（带当前代际）。
    ///
    /// 返回的 [`ObjectHandle`] 的 `object_id` 是该 utility world 的活元素句柄，`group` 是
    /// `act-<seq>`，动作收尾用 [`Self::release_act_group`] 释放。**绝不 panic**。
    pub async fn resolve_ref_to_object(
        &self,
        rec: &RefRecord,
        seq: u64,
    ) -> Result<ObjectHandle, BrowserError> {
        let manager = self
            .manager_for_record(rec)
            .await
            .ok_or(BrowserError::NotConnected)?;
        let group = act_object_group(seq);

        // ── 层②：vendored aria-ref selector → 元素句柄（by-handle，归入 objectGroup） ──
        // 内联脚本：this 是 InjectedScript 实例；用它的 parseSelector + querySelector 跑
        // `aria-ref=<full_ref>`（aria-ref engine 仅当 elements.get(ref).isConnected 才返元素）。
        // 找不到 / 已 detach → 返 null（结果 RemoteObject subtype=null、无 objectId）。
        let selector = aria_ref_selector(&rec.full_ref);
        let lookup_fn = "function(sel) { \
             const parsed = this.parseSelector(sel); \
             const el = this.querySelector(parsed, document, false); \
             return el || null; \
         }";
        let arg = CallArgument {
            value: Some(serde_json::Value::String(selector)),
            ..Default::default()
        };
        let result = manager
            .call_on_injected_handle(&rec.frame_id, lookup_fn, vec![arg], Some(&group), false)
            .await
            .map_err(map_inject_err)?;

        // by-handle：找到 → result.objectId 是元素句柄；找不到 → 无 objectId（subtype=null）。
        let object_id = match result.get("objectId").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                // 元素在层① 的代际表里但活 DOM 中已不存在（detach / not connected）= 漂移层②。
                return Err(BrowserError::NotConnected);
            }
        };

        // ── 层③：读元素 _ariaRef.{role,name} 二次校验（role 严格，name 宽松） ──
        let aria_ref_fn =
            "function() { return this && this._ariaRef ? { role: this._ariaRef.role, name: this._ariaRef.name } : null; }";
        let aria = manager
            .call_on_element(&object_id, aria_ref_fn, true)
            .await
            .map_err(map_inject_err)?;
        // result.value = {role, name} | null。读不到 _ariaRef（极罕见：元素无 expando）→ 当作漂移。
        let (actual_role, actual_name) = match aria.get("value") {
            Some(v) if v.is_object() => (
                v.get("role").and_then(|r| r.as_str()).unwrap_or("").to_string(),
                v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
            ),
            _ => {
                // 元素存在但没有 _ariaRef expando：无法校验来源，保守判 stale 并释放本组句柄。
                let _ = manager.release_object_group(&group).await;
                return Err(BrowserError::NodeStale {
                    generation: self.current_generation().await,
                });
            }
        };
        if ref_drifted(rec, &actual_role, &actual_name) {
            tracing::warn!(
                target: "nomi_browser_engine::actionability",
                full_ref = %rec.full_ref,
                expected_role = %rec.role, actual_role = %actual_role,
                expected_name = %rec.name, actual_name = %actual_name,
                "ref role mismatch (backendNodeId reuse / drift) -> NodeStale"
            );
            // 漂移：释放本组句柄，报 stale（带当前代际，让模型重新 observe 取新 ref）。
            let _ = manager.release_object_group(&group).await;
            return Err(BrowserError::NodeStale {
                generation: self.current_generation().await,
            });
        }

        Ok(ObjectHandle { object_id, group })
    }

    /// **actionability 五检查的 visible/stable/enabled/editable 批量判定**（DESIGN §11，设计裁决
    /// ②）：在已反查到的元素句柄上经 vendored PW 的 `checkElementStates`（injectedScript.ts:640，
    /// async）一次性判完 `states` 列出的状态，返三态 [`CheckResult`]。第⑤检查 receivesEvents
    /// （hit-target）不在 `checkElementStates`，由 B4 承担。
    ///
    /// **调用编排**：`checkElementStates` 是 **InjectedScript 实例**的方法（元素作首参），故委托
    /// [`InjectionManager::check_element_states`]——它以该帧 utility world 的注入实例为 `this`、元素
    /// `object_id` 作首个 by-handle 参数、`states` 作 by-value 字符串数组次参。**实例与元素须同一
    /// world**：act 反查产出的元素句柄正在主 page session 的 utility world，故这里用主帧的注入实例
    /// （B3 范围只覆盖主 page session 的元素；OOPIF 跨 session 留 B4/收尾，届时 `ObjectHandle` 需带
    /// 帧路由信息）。
    ///
    /// 返回：
    /// - `Ok(CheckResult::Pass)` —— `checkElementStates` 返 `undefined`，全过，放行；
    /// - `Ok(CheckResult::Missing(state))` —— 某态瞬时缺失（含 **readonly→`Missing("editable")`**），**可重试**；
    /// - `Ok(CheckResult::NotConnected)` —— 元素 detach（`'error:notconnected'`）；
    /// - `Err(BrowserError::Blocked{reason})` —— **editable 不可编辑特例**：元素类型根本不支持编辑
    ///   （[`is_non_editable_error`] 命中注入侧 `createStacklessError`），NonRecoverable，**禁重试**；
    /// - 其它注入异常 → 经 [`map_inject_err`] 成 `Other`（调用方按需）。
    ///
    /// **绝不 panic**。
    pub async fn check_states(
        &self,
        h: &ObjectHandle,
        states: &[&str],
    ) -> Result<CheckResult, BrowserError> {
        let handles = self.active_tab_handles().await?;
        let manager = handles.injection;
        // B3 范围：元素在主 page session 的 utility world，用主帧注入实例当 `this`。
        let frame_id = handles.main_frame_id;
        let result = match manager
            .check_element_states(&frame_id, &h.object_id, states)
            .await
        {
            Ok(v) => v,
            Err(InjectError::JsException(msg)) => {
                // editable 不可编辑特例：元素类型根本不支持编辑 → NonRecoverable Blocked（禁重试）。
                // 区别于暂时 readonly —— 那走正常返回的 Missing("editable")（可重试）。
                if is_non_editable_error(&msg) {
                    return Err(BrowserError::Blocked { reason: msg });
                }
                // 其它注入异常：按普通 JsException 处理（→ Other）。
                return Err(map_inject_err(InjectError::JsException(msg)));
            }
            Err(e) => return Err(map_inject_err(e)),
        };
        Ok(parse_check_result(result.get("value")))
    }

    // ═══════════════════════════════════════════════════════════════════════
    // double hit-target 三步舞（DESIGN §11，设计裁决③）：算点→真点→stop 读「布局是否漂移 /
    // 是否被遮挡」→ block 误点。actionability 第⑤检查 receivesEvents 的承担者。点击（B5）夹在
    // hit_setup 与 hit_stop 之间；interceptor 句柄全程 LIVE（setup by_value=false 保活），到
    // hit_stop 完才随 act-<seq> objectGroup 一次释放。**绝不 panic**。
    // ═══════════════════════════════════════════════════════════════════════

    /// **三步舞第①步：装 hit-target 拦截器**（DESIGN §11，设计裁决③）。经 vendored PW
    /// `setupHitTargetInterceptor`（injectedScript.ts:1067）在元素上装一个 capture-phase 事件拦截器
    /// （`this._hitTargetInterceptor`），并预检该 `point` 是否已被别的元素（如全屏 modal）遮挡。
    ///
    /// **调用编排**：`setupHitTargetInterceptor` 是 **InjectedScript 实例**方法（首参是 node），故经
    /// [`InjectionManager::call_on_injected_handle`] 以该帧 utility world 的注入实例为 `this`、元素
    /// `h.object_id` 作首个 by-handle 参数、`action`/`{x,y}`/`block_all` 作 by-value 次参传入。
    /// **`return_by_value=false`**：interceptor 是含 `stop()` 闭包的对象，必须取**句柄**保活到点击
    /// 之后（取值会丢掉闭包，stop 就读不到点击期间分发的事件）。元素与注入实例须**同一 world**
    /// （act 反查产出的句柄正在主 page session 的 utility world，见 §7；B4 只覆盖主帧，OOPIF 跨
    /// session 留收尾）。
    ///
    /// 返回（[`setup_result_outcome`] 三态翻译）：
    /// - `Ok(HitInterceptor)` —— 拦截器装好（回包有 objectId）；句柄归入 `h.group`（`act-<seq>`），
    ///   点击后调 [`Self::hit_stop`] 读判定，动作收尾随该组一次 releaseObjectGroup 释放；
    /// - `Err(BrowserError::Blocked{reason})` —— **预检短路**：`point` 已被遮挡，setup 内的
    ///   `expectHitTarget` 预检失败、短路返 hitTargetDescription 字符串（无需走点击，reason 指向
    ///   遮挡元素，如 `DIV#overlay`）；
    /// - `Err(BrowserError::NotConnected)` —— 元素已 detach（`'error:notconnected'`）。
    ///
    /// `action` 取 `'mouse'`/`'hover'`/`'tap'`/`'drag'`（injectedScript.ts:197-200 决定拦哪些事件；点击走
    /// `'mouse'`）。`block_all` 为 `true` 时拦截器**阻断**所有命中事件（PW 在 hit 检查失败后用来防止
    /// 误点传播；正常 setup 传 `false`）。
    pub async fn hit_setup(
        &self,
        h: &ObjectHandle,
        action: &str,
        point: Point,
        block_all: bool,
    ) -> Result<HitInterceptor, BrowserError> {
        let handles = self.active_tab_handles().await?;
        let manager = handles.injection;
        // B4 范围：元素在主 page session 的 utility world，用主帧注入实例当 `this`。
        let frame_id = handles.main_frame_id;
        // function(node, action, hitPoint, blockAll){ return this.setupHitTargetInterceptor(...) }
        let setup_fn = "function(node, action, hitPoint, blockAll) { \
             return this.setupHitTargetInterceptor(node, action, hitPoint, blockAll); \
         }";
        let args = vec![
            // 元素 node（by-handle，同 world）。
            CallArgument {
                object_id: Some(
                    chromiumoxide::cdp::js_protocol::runtime::RemoteObjectId::new(h.object_id.clone()),
                ),
                ..Default::default()
            },
            // action（by-value 字符串）。
            CallArgument {
                value: Some(Value::String(action.to_string())),
                ..Default::default()
            },
            // hitPoint {x,y}（by-value）。
            CallArgument {
                value: Some(serde_json::json!({ "x": point.x, "y": point.y })),
                ..Default::default()
            },
            // blockAllEvents（by-value bool）。
            CallArgument {
                value: Some(Value::Bool(block_all)),
                ..Default::default()
            },
        ];
        // return_by_value=false：保住 interceptor 对象（含 stop 闭包）的句柄到点击之后。
        let result = manager
            .call_on_injected_handle(&frame_id, setup_fn, args, Some(&h.group), false)
            .await
            .map_err(map_inject_err)?;

        match setup_result_outcome(&result) {
            SetupOutcome::Handle(object_id) => Ok(HitInterceptor {
                object_id,
                group: h.group.clone(),
            }),
            // 预检短路：已被遮挡 → Blocked（reason 指向遮挡元素）。无需走点击。
            SetupOutcome::Blocked(reason) => Err(BrowserError::Blocked { reason }),
            // 元素 detach → NotConnected。
            SetupOutcome::NotConnected => Err(BrowserError::NotConnected),
        }
    }

    /// **三步舞第③步：读 hit-target 判定**（点击已在 setup 与此之间分发完）。在 setup 装的
    /// interceptor 句柄上调其 `stop()`（[`crate::injected::InjectionManager::call_on_element`]，
    /// `function(){ return this.stop() }`，by-value）。`stop()` 返「点击期间首个命中事件落在目标
    /// 元素（或其后代）上 = `'done'`」| 「落在了别的元素 = `{ hitTargetDescription }` = 算点与事件
    /// 分发间布局漂移 / 被遮挡的误点」（injectedScript.ts:1119-1127）。
    ///
    /// **没收到任何事件也算成功**（stop 内 `return result || 'done'`：JS disabled / 跨帧 iframe 遮挡
    /// 等导致无事件触发时保守放行）—— 故无事件 → `Ok(())`。
    ///
    /// 返回（[`parse_hit_target_value`] 两态翻译）：
    /// - `Ok(())` —— `'done'`（命中目标，放行）；
    /// - `Err(BrowserError::Blocked{reason})` —— `{ hitTargetDescription }`（布局漂移误点，reason 指向
    ///   实际命中的元素）。
    pub async fn hit_stop(&self, i: &HitInterceptor) -> Result<(), BrowserError> {
        let manager = self.injection_manager().await?;
        let stop_fn = "function() { return this.stop(); }";
        let result = manager
            .call_on_element(&i.object_id, stop_fn, true)
            .await
            .map_err(map_inject_err)?;
        parse_hit_target_value(result.get("value"))
    }

    /// **遮挡权威判定（无拦截器，一次性）**：在元素句柄上直接跑 vendored PW
    /// `expectHitTarget`（injectedScript.ts:955）—— 用 `elementsFromPoint`/`elementFromPoint` 沿
    /// （含 shadow root 的）composed 树判 `point` 命中的最内元素是否为目标元素或其后代。是 →
    /// `'done'`；否 → `{ hitTargetDescription }`（命中了遮挡它的元素）。
    ///
    /// **用途**：①点击前的**廉价预检**（不装拦截器、不真点，直接问「此刻这个点会不会点到别人」）；
    /// ②CSS transform / 跨帧 iframe 等场景下，setup 的拦截器路径可能拿不到事件，此函数作**遮挡判定
    /// 的权威退路**。
    ///
    /// **调用编排**：`expectHitTarget` 是 InjectedScript 实例方法，签名 `(hitPoint, targetElement)`，故经
    /// [`InjectionManager::call_on_injected_handle`] 以注入实例为 `this`、`{x,y}` 作 by-value 首参、元素
    /// `h.object_id` 作 by-handle 次参传入（by-value 取结果）。元素须与注入实例同一 world（B4 主帧）。
    ///
    /// 返回（[`parse_hit_target_value`] 两态翻译）：`Ok(())`（`'done'`，未被遮挡）|
    /// `Err(BrowserError::Blocked{reason})`（`{ hitTargetDescription }`，reason 指向遮挡元素）。
    pub async fn expect_hit_target(
        &self,
        h: &ObjectHandle,
        point: Point,
    ) -> Result<(), BrowserError> {
        let handles = self.active_tab_handles().await?;
        let manager = handles.injection;
        let frame_id = handles.main_frame_id;
        // function(hitPoint, node){ return this.expectHitTarget(hitPoint, node) }
        let expect_fn = "function(hitPoint, node) { return this.expectHitTarget(hitPoint, node); }";
        let args = vec![
            // hitPoint {x,y}（by-value 首参）。
            CallArgument {
                value: Some(serde_json::json!({ "x": point.x, "y": point.y })),
                ..Default::default()
            },
            // 目标元素 node（by-handle 次参）。
            CallArgument {
                object_id: Some(
                    chromiumoxide::cdp::js_protocol::runtime::RemoteObjectId::new(h.object_id.clone()),
                ),
                ..Default::default()
            },
        ];
        // by-value：结果是 'done' 或 {hitTargetDescription}，皆可序列化。归入元素同组（无新句柄，
        // 但传 group 保持一致；by-value 不产句柄故释放与否无害）。
        let result = manager
            .call_on_injected_handle(&frame_id, expect_fn, args, Some(&h.group), true)
            .await
            .map_err(map_inject_err)?;
        parse_hit_target_value(result.get("value"))
    }

    /// 释放一次动作的 objectGroup（`act-<seq>`）—— 动作收尾（成功/失败均然）调一次，释放该次
    /// 反查产生的全部元素句柄。需 rec 定位所属帧的注入管线（句柄归属该帧 utility world）。
    /// best-effort：管线找不到 / 释放失败均不致命（句柄随导航/GC 自然回收），只 warn。
    pub async fn release_act_group(&self, rec: &RefRecord, seq: u64) {
        let group = act_object_group(seq);
        let Some(manager) = self.manager_for_record(rec).await else {
            tracing::warn!(
                target: "nomi_browser_engine::actionability",
                session_id = %rec.session_id, group = %group,
                "release_act_group: manager not found (skip)"
            );
            return;
        };
        if let Err(e) = manager.release_object_group(&group).await {
            tracing::warn!(
                target: "nomi_browser_engine::actionability",
                group = %group, error = ?e,
                "release_act_group: releaseObjectGroup failed (non-fatal)"
            );
        }
    }

    /// **按 ref 字符串释放动作组**（[`Self::release_act_group`] 的 facade 友好封装）：act 持有 ref
    /// 字符串 + seq，用它定位 RefRecord（取所属帧）再整组释放。ref 已不在表（导航翻新代际）→
    /// 静默跳过（句柄随导航自然回收）。绝不 panic。
    pub async fn release_act_group_by_ref(&self, llm_ref: &str, seq: u64) {
        // D1：ref_table_lock 现 async（active tab 的 ref_table Arc）。active tab 缺失 → 视作 ref 已失效。
        let rec = match self.ref_table_lock().await {
            Ok(ref_table) => {
                let guard = ref_table.lock().await;
                guard.as_ref().and_then(|t| t.resolve(llm_ref)).cloned()
            }
            Err(_) => None,
        };
        match rec {
            Some(r) => self.release_act_group(&r, seq).await,
            None => {
                // ref 已随导航失效：其句柄所在 world 已销毁，无需显式释放。
                tracing::debug!(
                    target: "nomi_browser_engine::actionability",
                    llm_ref = %llm_ref, seq,
                    "release_act_group_by_ref: ref not in current table (skip)"
                );
            }
        }
    }

    /// **arm 一个 objectGroup 释放的 RAII drop-guard**（动作骨架收口；类型保证非约定）。
    ///
    /// [`crate::actions::CdpBackend::act_with_skeleton`] 在反查所属帧的注入管线后调本方法 arm 一个
    /// [`ActGroupReleaseGuard`]，把它绑在整个动作生命周期上（`let _g = arm_act_group_release(...);
    /// run_act_with_retry(...).await`）。**无论动作正常返回 / `?` 早返 / await 点 panic**，guard 离开
    /// 作用域即 Drop，释放本动作 objectGroup（`act-<seq>`）的全部句柄——把原「手动 finally」升级成
    /// 类型保证（C2/C3 后续若在骨架与 release 间插 `?` 或 panic 也不泄漏；keystone 被多动作复用，风险
    /// 放大，故收口）。
    ///
    /// `rec` 用来定位所属帧的注入管线（句柄归该帧 utility world）。管线找不到（OOPIF 子 session 已
    /// detach）→ guard 为 no-op（句柄随 world 销毁自然回收，无需释放）。release 是 async 而 `Drop`
    /// 不能 await：guard 的 Drop 用 `tokio::spawn` fire-and-forget 一次 `release_object_group`（参照
    /// [`crate::backend::cdp::ActAbortGuard`] 的 Drop 范式——它同样在 Drop 里 `JoinHandle::abort`）。
    /// 正常路径**只释放一次**（guard 是唯一释放点，无手动双释放）。**绝不 panic**。
    pub async fn arm_act_group_release(&self, rec: &RefRecord, seq: u64) -> ActGroupReleaseGuard {
        let group = act_object_group(seq);
        // 反查所属帧注入管线（InjectionManager 克隆友好：Connection 共享传输、shared 是 Arc<Mutex>）。
        // 找不到（OOPIF 子 session detach）→ no-op guard：句柄随 world 销毁自然回收。
        let manager = self.manager_for_record(rec).await;
        ActGroupReleaseGuard {
            release: manager.map(|m| (m, group)),
        }
    }
}

/// **objectGroup 释放的 RAII drop-guard**（[`CdpBackend::arm_act_group_release`] 返回）：持所属帧的
/// [`InjectionManager`] 句柄 + 组名（`act-<seq>`）；Drop 时 `tokio::spawn` fire-and-forget 一次
/// `Runtime.releaseObjectGroup`，释放本动作 objectGroup 的全部元素句柄。
///
/// **为何 `tokio::spawn`**：`release_object_group` 是 async，而 `Drop` 是同步上下文（不能 await）。
/// release 是幂等且 best-effort（组名不存在/已释放 CDP 不报错；句柄也随导航/GC 自然回收），故 detach
/// 一个释放任务、不等其完成是安全的——失败只 warn，不影响动作语义。这与
/// [`crate::backend::cdp::ActAbortGuard`] 在 Drop 里 `JoinHandle::abort()`（同样 fire-and-forget）
/// 同源。`release` 为 `None` 时（管线找不到）Drop 是 no-op。**Drop 绝不 panic**。
pub struct ActGroupReleaseGuard {
    /// `Some((manager, group))` = arm 成功，Drop 释放该组；`None` = no-op（管线找不到 / 已 take）。
    release: Option<(InjectionManager, String)>,
}

impl Drop for ActGroupReleaseGuard {
    fn drop(&mut self) {
        // take()：保证只释放一次（无手动 + RAII 双释放；guard 是唯一释放点）。
        if let Some((manager, group)) = self.release.take() {
            // release_object_group 是 async；Drop 不能 await → fire-and-forget 一个释放任务。
            // best-effort：失败只 warn（组随导航/GC 自然回收，不影响动作语义）。绝不 panic。
            tokio::spawn(async move {
                if let Err(e) = manager.release_object_group(&group).await {
                    tracing::warn!(
                        target: "nomi_browser_engine::actionability",
                        group = %group, error = ?e,
                        "ActGroupReleaseGuard: releaseObjectGroup failed (non-fatal)"
                    );
                }
            });
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 纯逻辑单测（无浏览器）：selector 拼接、objectGroup 命名、层③ role/name 漂移判定。
// 真实反查（aria-ref selector → objectId、stale → NodeStale、release）见 #[ignore] 集成。
// ═══════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;

    fn rec(role: &str, name: &str, full_ref: &str) -> RefRecord {
        RefRecord {
            session_id: "S".into(),
            frame_id: "F".into(),
            full_ref: full_ref.into(),
            role: role.into(),
            name: name.into(),
        }
    }

    #[test]
    fn aria_ref_selector_prepends_engine_name_with_full_ref() {
        // 必须用**完整** ref（含帧前缀 f<seq>），因 injected 的 elements Map 以完整 ref 为键。
        assert_eq!(aria_ref_selector("f3e7"), "aria-ref=f3e7");
        assert_eq!(aria_ref_selector("f0e1"), "aria-ref=f0e1");
        assert_eq!(aria_ref_selector("f12e345"), "aria-ref=f12e345");
    }

    #[test]
    fn act_object_group_names_per_seq() {
        assert_eq!(act_object_group(0), "act-0");
        assert_eq!(act_object_group(7), "act-7");
        // 不同 seq 名不同（动作隔离释放）。
        assert_ne!(act_object_group(1), act_object_group(2));
    }

    #[test]
    fn ref_drifted_role_mismatch_is_drift() {
        let r = rec("button", "Submit order", "f0e1");
        // role 一致 + name 一致 → 不漂移。
        assert!(!ref_drifted(&r, "button", "Submit order"));
        // role 不符 → 漂移（即便 name 巧合相同）。
        assert!(ref_drifted(&r, "link", "Submit order"));
        assert!(ref_drifted(&r, "textbox", "Submit order"));
    }

    #[test]
    fn ref_drifted_name_is_lenient() {
        let r = rec("button", "Submit order", "f0e1");
        // role 一致、name 变了（动态文案）→ **不**判漂移（P2 契约：name 宽松）。
        assert!(!ref_drifted(&r, "button", "Submit order (3)"));
        assert!(!ref_drifted(&r, "button", ""));
        assert!(!ref_drifted(&r, "button", "完全不同的名字"));
    }

    #[test]
    fn ref_drifted_empty_role_record_matches_empty_actual() {
        // 退化：rec.role 为空（极罕见的无 role 元素），actual 也空 → 不漂移。
        let r = rec("", "", "f0e9");
        assert!(!ref_drifted(&r, "", ""));
        // rec 无 role 但 actual 有 role → 漂移（严格）。
        assert!(ref_drifted(&r, "button", ""));
    }

    // ── check_states 三态解析（[纯逻辑]，喂构造 Value，不进浏览器）────────────────────

    #[test]
    fn parse_check_result_undefined_is_pass() {
        // `checkElementStates` 全过返 `undefined` → by-value 下缺 `value` 字段（None）→ Pass。
        assert_eq!(parse_check_result(None), CheckResult::Pass);
        // 显式 JSON null（极个别 CDP 形态）同样视为全过。
        assert_eq!(parse_check_result(Some(&Value::Null)), CheckResult::Pass);
    }

    #[test]
    fn parse_check_result_missing_state_is_missing() {
        // `{missingState:"visible"}` → Missing("visible")（hidden 元素：visible 缺态）。
        let v = serde_json::json!({"missingState": "visible"});
        assert_eq!(parse_check_result(Some(&v)), CheckResult::Missing("visible".into()));
        // disabled 元素：enabled 缺态。
        let v = serde_json::json!({"missingState": "enabled"});
        assert_eq!(parse_check_result(Some(&v)), CheckResult::Missing("enabled".into()));
        // readonly input：editable 缺态（**可重试**，区别于不可编辑特例的 Blocked）。
        let v = serde_json::json!({"missingState": "editable"});
        assert_eq!(parse_check_result(Some(&v)), CheckResult::Missing("editable".into()));
        // stable 缺态（元素仍在动）。
        let v = serde_json::json!({"missingState": "stable"});
        assert_eq!(parse_check_result(Some(&v)), CheckResult::Missing("stable".into()));
    }

    #[test]
    fn parse_check_result_notconnected_is_notconnected() {
        // `'error:notconnected'` → NotConnected（元素已 detach）。
        let v = Value::String("error:notconnected".into());
        assert_eq!(parse_check_result(Some(&v)), CheckResult::NotConnected);
    }

    #[test]
    fn parse_check_result_unknown_shapes_are_conservatively_notconnected() {
        // 注入契约只产上述三态；任何陌生形状保守判 NotConnected（不静默放行点击）。
        assert_eq!(
            parse_check_result(Some(&Value::String("weird".into()))),
            CheckResult::NotConnected
        );
        // 对象但无 missingState 字段。
        let v = serde_json::json!({"foo": "bar"});
        assert_eq!(parse_check_result(Some(&v)), CheckResult::NotConnected);
        // missingState 非字符串（坏形状）。
        let v = serde_json::json!({"missingState": 7});
        assert_eq!(parse_check_result(Some(&v)), CheckResult::NotConnected);
        // 标量数字 / bool / 数组。
        assert_eq!(parse_check_result(Some(&serde_json::json!(1))), CheckResult::NotConnected);
        assert_eq!(parse_check_result(Some(&serde_json::json!(true))), CheckResult::NotConnected);
        assert_eq!(parse_check_result(Some(&serde_json::json!([]))), CheckResult::NotConnected);
    }

    // ── editable 异常分类（[纯逻辑]：不可编辑特例 → NonRecoverable，区别于其它注入异常）──

    #[test]
    fn is_non_editable_error_matches_stackless_error_text() {
        // 注入侧 `createStacklessError('Element is not an <input>, … [aria-readonly]')`
        // （injectedScript.ts:745）的原文 → NonRecoverable（→ Blocked，禁重试）。
        let msg = "Element is not an <input>, <textarea>, <select> or [contenteditable] \
                   and does not have a role allowing [aria-readonly]";
        assert!(is_non_editable_error(msg));
        // chromium 端 `new Error(msg)` 的 `exception.description` 带 `Error: ` 前缀（describe_exception
        // 优先取它）——子串匹配不锚首，仍命中。
        let with_prefix = format!("Error: {msg}");
        assert!(is_non_editable_error(&with_prefix));
        // checked/indeterminate 类「类型不支持该状态」同属 NonRecoverable（前瞻覆盖）。
        assert!(is_non_editable_error("Not a checkbox or radio button"));
    }

    #[test]
    fn is_non_editable_error_rejects_unrelated_exceptions() {
        // 其它注入异常（瞬态 / 我方 bug）**不**判 NonRecoverable —— 走普通 JsException→Other。
        assert!(!is_non_editable_error("TypeError: x is not a function"));
        assert!(!is_non_editable_error("Node is not queryable."));
        assert!(!is_non_editable_error(""));
        assert!(!is_non_editable_error("Unexpected element state \"frobnicate\""));
    }

    // ── double hit-target 三步舞解析（[纯逻辑]，喂构造 Value，不进浏览器）──────────────

    #[test]
    fn setup_result_outcome_objectid_is_handle() {
        // setup 返 interceptor 对象 → by-handle 回包有 objectId（subtype 'object'）→ Handle。
        let v = serde_json::json!({"type": "object", "objectId": "OBJ-7"});
        assert_eq!(setup_result_outcome(&v), SetupOutcome::Handle("OBJ-7".into()));
        // 即使同时带 value/subtype，只要有 objectId 就是句柄（保活路径）。
        let v = serde_json::json!({"type": "object", "subtype": null, "objectId": "OBJ-8"});
        assert_eq!(setup_result_outcome(&v), SetupOutcome::Handle("OBJ-8".into()));
    }

    #[test]
    fn setup_result_outcome_notconnected_string() {
        // setup 短路返 'error:notconnected'（元素 detach）→ by-value 字符串，无 objectId → NotConnected。
        let v = serde_json::json!({"type": "string", "value": "error:notconnected"});
        assert_eq!(setup_result_outcome(&v), SetupOutcome::NotConnected);
    }

    #[test]
    fn setup_result_outcome_hit_target_description_string_is_blocked() {
        // setup 预检短路返 bare hitTargetDescription（已被遮挡）→ by-value 字符串，无 objectId → Blocked。
        let v = serde_json::json!({"type": "string", "value": "DIV#overlay"});
        assert_eq!(
            setup_result_outcome(&v),
            SetupOutcome::Blocked("DIV#overlay".into())
        );
        // 形如 “X from Y subtree” 的根遮挡描述同样作 Blocked 透传。
        let v = serde_json::json!({"type": "string", "value": "BUTTON from DIALOG#modal subtree"});
        assert_eq!(
            setup_result_outcome(&v),
            SetupOutcome::Blocked("BUTTON from DIALOG#modal subtree".into())
        );
    }

    #[test]
    fn setup_result_outcome_unknown_shape_is_conservatively_notconnected() {
        // 无 objectId 也无字符串 value（不该发生）→ 保守 NotConnected（不放行点击）。
        let v = serde_json::json!({"type": "undefined"});
        assert_eq!(setup_result_outcome(&v), SetupOutcome::NotConnected);
        // value 非字符串（坏形状）。
        let v = serde_json::json!({"value": 7});
        assert_eq!(setup_result_outcome(&v), SetupOutcome::NotConnected);
        let v = serde_json::json!({});
        assert_eq!(setup_result_outcome(&v), SetupOutcome::NotConnected);
    }

    #[test]
    fn parse_hit_target_value_done_is_ok() {
        // stop()/expectHitTarget 返 'done' → 命中目标 → Ok。
        let v = Value::String("done".into());
        assert!(parse_hit_target_value(Some(&v)).is_ok());
    }

    #[test]
    fn parse_hit_target_value_description_is_blocked_with_reason() {
        // {hitTargetDescription:"DIV#overlay"} → 误点遮挡 → Blocked，reason == 描述。
        let v = serde_json::json!({"hitTargetDescription": "DIV#overlay"});
        match parse_hit_target_value(Some(&v)) {
            Err(BrowserError::Blocked { reason }) => assert_eq!(reason, "DIV#overlay"),
            other => panic!("expected Blocked(DIV#overlay), got {other:?}"),
        }
        // 根遮挡 “X from Y subtree” 描述同样透传到 reason。
        let v = serde_json::json!({"hitTargetDescription": "SPAN from DIALOG#m subtree"});
        match parse_hit_target_value(Some(&v)) {
            Err(BrowserError::Blocked { reason }) => {
                assert_eq!(reason, "SPAN from DIALOG#m subtree")
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn parse_hit_target_value_unknown_shapes_are_conservatively_blocked() {
        // 注入契约只产 'done' | {hitTargetDescription}；任何陌生形状保守 Blocked（不静默放行）。
        assert!(matches!(
            parse_hit_target_value(None),
            Err(BrowserError::Blocked { .. })
        ));
        assert!(matches!(
            parse_hit_target_value(Some(&Value::String("weird".into()))),
            Err(BrowserError::Blocked { .. })
        ));
        // 对象但无 hitTargetDescription。
        let v = serde_json::json!({"foo": "bar"});
        assert!(matches!(
            parse_hit_target_value(Some(&v)),
            Err(BrowserError::Blocked { .. })
        ));
        // 标量数字 / bool / 数组。
        assert!(matches!(
            parse_hit_target_value(Some(&serde_json::json!(1))),
            Err(BrowserError::Blocked { .. })
        ));
        assert!(matches!(
            parse_hit_target_value(Some(&serde_json::json!(true))),
            Err(BrowserError::Blocked { .. })
        ));
        assert!(matches!(
            parse_hit_target_value(Some(&serde_json::json!([]))),
            Err(BrowserError::Blocked { .. })
        ));
    }
}
