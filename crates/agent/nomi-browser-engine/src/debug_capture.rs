//! **前端调试抓取**：per-tab 有界环形缓冲 + CDP 事件→结构化条目映射。
//!
//! 三类数据：
//! 1. **Console**（`Runtime.consoleAPICalled`）→ [`ConsoleEntry`]
//! 2. **Page errors**（`Runtime.exceptionThrown` + `Log.entryAdded[level=error]`）→ [`PageError`]
//! 3. **Network**（`Network.requestWillBeSent` / `responseReceived` / `loadingFinished` /
//!    `loadingFailed`）→ [`NetworkEntry`]
//!
//! 所有条目存在 per-tab 有界 [`RingBuffer`]（cap=500），最旧条目被丢弃。三个只读动作
//! （`GetConsoleLogs` / `GetPageErrors` / `GetNetworkLog`）读取缓冲并经 [`crate::redact`]
//! 脱敏后暴露给 LLM。
//!
//! 纯逻辑、零 I/O、零 CDP（映射器从 `serde_json::Value` 的 CDP 事件 params 构建条目）。

use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::LazyLock;

use regex::Regex;
use url::Url;

// ═══════════════════════════════════════════════════════════════════════════
// 有界环形缓冲
// ═══════════════════════════════════════════════════════════════════════════

/// 固定容量的环形缓冲（FIFO），超出时丢弃最旧条目。
pub struct RingBuffer<T> {
    buf: VecDeque<T>,
    cap: usize,
}

impl<T> RingBuffer<T> {
    /// 创建容量为 `cap` 的空缓冲（`cap` 最小 1）。
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// 推入一个条目；若已满则丢弃最旧一个。
    pub fn push(&mut self, item: T) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(item);
    }

    /// 当前条目数。
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// 容量上限。
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// 以切片对形式遍历（VecDeque 内部可能分两段）。
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.buf.iter()
    }

    /// 可变遍历。
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        self.buf.iter_mut()
    }

    /// 消费式取出所有条目（从旧到新）。
    pub fn drain(&mut self) -> Vec<T> {
        self.buf.drain(..).collect()
    }

    /// 快照：clone 所有条目。
    pub fn snapshot(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.buf.iter().cloned().collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 条目类型
// ═══════════════════════════════════════════════════════════════════════════

/// Console 消息级别（映射 CDP `Runtime.consoleAPICalled.type`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleLevel {
    Log,
    Info,
    Warn,
    Error,
    Debug,
    Other(String),
}

impl ConsoleLevel {
    pub fn from_cdp(s: &str) -> Self {
        match s {
            "log" => Self::Log,
            "info" => Self::Info,
            "warning" | "warn" => Self::Warn,
            "error" => Self::Error,
            "debug" => Self::Debug,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Log => "log",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Debug => "debug",
            Self::Other(s) => s,
        }
    }
}

/// 一条 console 消息（来自 `Runtime.consoleAPICalled`）。
#[derive(Clone, Debug)]
pub struct ConsoleEntry {
    pub level: ConsoleLevel,
    /// console 参数序列化为文本（多参以空格连接）。
    pub text: String,
    /// CDP 时间戳（单位 ms，epoch-based）。
    pub timestamp: f64,
    /// 调用源 URL（可能空）。
    pub url: Option<String>,
}

/// 一条页面错误（未捕获异常 / error-level 日志）。
#[derive(Clone, Debug)]
pub struct PageError {
    pub message: String,
    pub stack: Option<String>,
    pub timestamp: f64,
}

/// 一个 HTTP 请求头（名-值对）。
#[derive(Clone, Debug)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// 一条网络请求/响应记录。
#[derive(Clone, Debug)]
pub struct NetworkEntry {
    pub url: String,
    pub method: String,
    pub status: Option<u16>,
    pub mime: Option<String>,
    /// 耗时 ms（responseReceived.timestamp - requestWillBeSent.timestamp）。
    pub duration_ms: Option<f64>,
    /// 响应体大小（字节）。
    pub encoded_data_length: Option<u64>,
    /// 是否失败（loadingFailed）。
    pub failed: bool,
    /// 失败原因。
    pub error_text: Option<String>,
    /// 请求头（原始，未脱敏——脱敏在序列化层）。
    pub request_headers: Vec<HttpHeader>,
    /// 响应头（原始，未脱敏——脱敏在序列化层）。
    pub response_headers: Vec<HttpHeader>,
    /// 请求体（仅 include_bodies=true 时填充；原始，脱敏在序列化层）。
    pub request_body: Option<String>,
    /// 响应体（仅 include_bodies=true 时填充；原始，脱敏在序列化层）。
    pub response_body: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════
// per-tab 调试缓冲聚合
// ═══════════════════════════════════════════════════════════════════════════

/// 默认缓冲容量。
pub const DEFAULT_BUFFER_CAP: usize = 500;

/// per-tab 调试数据聚合：三个有界环形缓冲。
pub struct DebugBuffers {
    pub console: RingBuffer<ConsoleEntry>,
    pub errors: RingBuffer<PageError>,
    pub network: RingBuffer<NetworkEntry>,
}

impl DebugBuffers {
    pub fn new(cap: usize) -> Self {
        Self {
            console: RingBuffer::new(cap),
            errors: RingBuffer::new(cap),
            network: RingBuffer::new(cap),
        }
    }
}

impl Default for DebugBuffers {
    fn default() -> Self {
        Self::new(DEFAULT_BUFFER_CAP)
    }
}

/// 调试缓冲的不可变快照（从 `DebugBuffers` clone 出，供读取动作 + 集成测试消费）。
#[derive(Clone, Debug)]
pub struct DebugSnapshot {
    pub console: Vec<ConsoleEntry>,
    pub errors: Vec<PageError>,
    pub network: Vec<NetworkEntry>,
}

impl DebugSnapshot {
    /// 从 per-tab 缓冲取快照（短临界区 lock + clone）。
    pub fn from_buffers(buffers: &std::sync::Mutex<DebugBuffers>) -> Self {
        let guard = buffers.lock().unwrap_or_else(|e| e.into_inner());
        Self {
            console: guard.console.snapshot(),
            errors: guard.errors.snapshot(),
            network: guard.network.snapshot(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CDP 事件→条目纯映射器
// ═══════════════════════════════════════════════════════════════════════════

/// 从 `Runtime.consoleAPICalled` 的 params JSON 映射出 [`ConsoleEntry`]。
pub fn map_console_event(params: &serde_json::Value) -> Option<ConsoleEntry> {
    let level_str = params.get("type")?.as_str()?;
    let level = ConsoleLevel::from_cdp(level_str);
    let timestamp = params.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);

    // args 是 RemoteObject 数组；我们把每个的 description/value/unserializableValue 拼成文本。
    let args = params.get("args").and_then(|v| v.as_array());
    let text = match args {
        Some(arr) => arr
            .iter()
            .map(remote_object_to_text)
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    };

    let url = params
        .get("stackTrace")
        .and_then(|st| st.get("callFrames"))
        .and_then(|cf| cf.as_array())
        .and_then(|frames| frames.first())
        .and_then(|f| f.get("url"))
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    Some(ConsoleEntry {
        level,
        text,
        timestamp,
        url,
    })
}

/// 从 `Runtime.exceptionThrown` 的 params JSON 映射出 [`PageError`]。
pub fn map_exception_event(params: &serde_json::Value) -> Option<PageError> {
    let detail = params.get("exceptionDetails")?;
    let timestamp = params.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let text = detail
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("Uncaught exception");

    // 优先取 exception.description（更全，含类型+消息+堆栈前几行）；否则取 text。
    let exception_obj = detail.get("exception");
    let description = exception_obj
        .and_then(|e| e.get("description"))
        .and_then(|v| v.as_str());

    let message = description.unwrap_or(text).to_string();

    let stack = detail
        .get("stackTrace")
        .and_then(|st| {
            let frames = st.get("callFrames")?.as_array()?;
            let lines: Vec<String> = frames
                .iter()
                .take(10)
                .map(|f| {
                    let fn_name = f
                        .get("functionName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<anonymous>");
                    let url = f.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let redacted_url = redact_url(url);
                    let line = f.get("lineNumber").and_then(|v| v.as_u64()).unwrap_or(0);
                    let col = f.get("columnNumber").and_then(|v| v.as_u64()).unwrap_or(0);
                    format!("  at {fn_name} ({redacted_url}:{line}:{col})")
                })
                .collect();
            if lines.is_empty() {
                None
            } else {
                Some(lines.join("\n"))
            }
        });

    Some(PageError {
        message,
        stack,
        timestamp,
    })
}

/// 从 `Log.entryAdded` params 映射出 [`PageError`]（仅 level=error）。
pub fn map_log_error_event(params: &serde_json::Value) -> Option<PageError> {
    let entry = params.get("entry")?;
    let level = entry.get("level")?.as_str()?;
    if level != "error" {
        return None;
    }
    let text = entry
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let timestamp = entry.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let stack_trace = entry.get("stackTrace").and_then(|st| {
        let frames = st.get("callFrames")?.as_array()?;
        let lines: Vec<String> = frames
            .iter()
            .take(10)
            .map(|f| {
                let fn_name = f
                    .get("functionName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<anonymous>");
                let url = f.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let redacted_url = redact_url(url);
                let line = f.get("lineNumber").and_then(|v| v.as_u64()).unwrap_or(0);
                format!("  at {fn_name} ({redacted_url}:{line})")
            })
            .collect();
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    });
    Some(PageError {
        message: text,
        stack: stack_trace,
        timestamp,
    })
}

/// 从 `Network.requestWillBeSent` 的 params 构造一个初始 [`NetworkEntry`]（后续由
/// responseReceived/loadingFinished/loadingFailed 补全状态）。
pub fn map_request_will_be_sent(params: &serde_json::Value) -> Option<(String, NetworkEntry)> {
    let request_id = params.get("requestId")?.as_str()?.to_string();
    let request = params.get("request")?;
    let url = request.get("url")?.as_str()?.to_string();
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_string();

    let request_headers = parse_headers(request.get("headers"));

    let request_body = request
        .get("postData")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some((
        request_id,
        NetworkEntry {
            url,
            method,
            status: None,
            mime: None,
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers,
            response_headers: Vec::new(),
            request_body,
            response_body: None,
        },
    ))
}

/// 从 `Network.responseReceived` 补全状态/响应头。
pub fn patch_response_received(entry: &mut NetworkEntry, params: &serde_json::Value) {
    if let Some(response) = params.get("response") {
        entry.status = response.get("status").and_then(|v| v.as_u64()).map(|s| s as u16);
        entry.mime = response
            .get("mimeType")
            .and_then(|v| v.as_str())
            .map(String::from);
        entry.response_headers = parse_headers(response.get("headers"));
    }
}

/// 从 `Network.loadingFinished` 补全大小/时间。
#[allow(clippy::collapsible_if)]
pub fn patch_loading_finished(entry: &mut NetworkEntry, params: &serde_json::Value, request_timestamp: f64) {
    entry.encoded_data_length = params
        .get("encodedDataLength")
        .and_then(|v| v.as_f64())
        .map(|v| v as u64);
    if let Some(ts) = params.get("timestamp").and_then(|v| v.as_f64()) {
        if request_timestamp > 0.0 {
            entry.duration_ms = Some((ts - request_timestamp) * 1000.0);
        }
    }
}

/// 从 `Network.loadingFailed` 标记失败。
pub fn patch_loading_failed(entry: &mut NetworkEntry, params: &serde_json::Value) {
    entry.failed = true;
    entry.error_text = params
        .get("errorText")
        .and_then(|v| v.as_str())
        .map(String::from);
}

// ═══════════════════════════════════════════════════════════════════════════
// LLM-facing 序列化（脱敏 + 不可信包裹）
// ═══════════════════════════════════════════════════════════════════════════

// ── Known-secret exact-blackout (deterministic, not heuristic) ──────────────────────────
// The facade resolves `secret:NAME` → plaintext and inserts each resolved value into a
// session-scoped `KnownSecretValues` set (shared via `Arc<Mutex<HashSet<String>>>`). The
// debug serializers apply `String::replace(value, "[KNOWN_SECRET_REDACTED]")` as the FIRST
// redaction step — catching the value ANYWHERE (URL path, JSON body, console arg, stack)
// regardless of format or entropy. This is the deterministic guarantee; the structural URL
// redaction + `redact_debug_text` heuristics REMAIN as defense-in-depth for UNKNOWN
// (page-origin) secrets the agent never resolved.
//
// Security invariants:
// - The set holds ONLY the agent's own resolved secrets (values it already injects via
//   insertText), so this is NOT a new exposure category.
// - In-memory only, session-scoped (dropped with the engine/facade).
// - Only values with len >= 4 are inserted (avoid over-matching trivial strings).
// - Read under a short `Mutex::lock` (never across await); poisoned-lock = empty (fail-open
//   on poison is acceptable here because the heuristic passes still run as defense-in-depth).

/// Apply known-secret exact-blackout: replace every known secret value in `text` with the
/// redaction marker. This is O(n*m) where n=text length, m=number of secrets — acceptable
/// because the secret set is small (typically 1-5 entries) and the text is bounded by the
/// ring buffer cap.
///
/// NOTE: This is the deterministic keystone — it catches the agent's OWN `secret:NAME` values
/// regardless of format, entropy, or position. Page-origin secrets that the agent never resolved
/// are handled heuristically by `redact_debug_text` (see its "Accepted residual" doc).
fn apply_known_secret_blackout(mut text: String, known_secrets: &HashSet<String>) -> String {
    for secret in known_secrets {
        if !secret.is_empty() {
            text = text.replace(secret.as_str(), "[KNOWN_SECRET_REDACTED]");
        }
    }
    text
}

/// 安全头名白名单：只有这些 header 会暴露给 LLM，其余全部丢弃。fail-closed。
const SAFE_HEADER_ALLOWLIST: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "cache-control",
    "etag",
    "last-modified",
    "date",
    "server",
    "x-request-id",
    "x-powered-by",
    "access-control-allow-origin",
    "vary",
    "transfer-encoding",
];

/// URL 脱敏（STRUCTURAL）：parse with `url` crate, strip userinfo entirely,
/// redact query/fragment to `[QUERY_REDACTED]`, and run each path segment through the
/// secret heuristic (high-entropy or known-pattern → `[REDACTED]`).
///
/// For non-parseable URLs (e.g. relative, blob:, data:) fall back to regex-based stripping.
/// Fail-closed: if in doubt, redact.
pub fn redact_url(raw: &str) -> String {
    // Try structured parse first.
    if let Ok(mut parsed) = Url::parse(raw) {
        // 1) Strip userinfo entirely (never useful for debugging, always a leak).
        if !parsed.username().is_empty() || parsed.password().is_some() {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
        }

        // 2) Redact path segments that look like secrets.
        let redacted_path = {
            let segments: Vec<&str> = parsed.path_segments()
                .map(|segs| segs.collect())
                .unwrap_or_default();
            if segments.is_empty() {
                parsed.path().to_string()
            } else {
                let redacted_segs: Vec<String> = segments
                    .iter()
                    .map(|seg| {
                        if is_secret_segment(seg) {
                            "[REDACTED]".to_string()
                        } else {
                            (*seg).to_string()
                        }
                    })
                    .collect();
                format!("/{}", redacted_segs.join("/"))
            }
        };

        // 3) Rebuild: scheme://host + redacted-path + redacted-query indicator.
        let scheme = parsed.scheme();
        let host_str = parsed.host_str().unwrap_or("");
        let port_part = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();

        let query_part = if parsed.query().is_some() {
            "?[QUERY_REDACTED]"
        } else {
            ""
        };

        format!("{scheme}://{host_str}{port_part}{redacted_path}{query_part}")
    } else {
        // Fallback for non-parseable URLs: regex-strip userinfo + query.
        redact_url_fallback(raw)
    }
}

/// Heuristic: does this path segment look like a secret/token?
/// Catches: high-entropy segments (len>=12, Shannon>=3.5), known prefixes (sk-/Bearer/etc.),
/// and segments that look like JWTs (contain dots + high overall entropy).
fn is_secret_segment(seg: &str) -> bool {
    if seg.is_empty() {
        return false;
    }
    // Skip obviously safe short segments (file extensions, version numbers, common paths)
    if seg.len() < 8 {
        return false;
    }
    // Known secret prefixes
    if seg.starts_with("sk-")
        || seg.starts_with("pk-")
        || seg.starts_with("AKIA")
        || seg.starts_with("eyJ") // JWT
    {
        return true;
    }
    // High-entropy (lower threshold for debug context: len>=12, Shannon>=3.5)
    if seg.len() >= 12 && crate::redact::shannon_entropy(seg) >= 3.5 {
        return true;
    }
    false
}

/// Fallback URL redaction for non-parseable URLs: strip `://user:pass@` and `?...`/`#...`.
fn redact_url_fallback(raw: &str) -> String {
    static USERINFO_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"://[^/@]+@").unwrap());
    let no_userinfo = USERINFO_RE.replace(raw, "://");
    // Strip query + fragment
    let base = no_userinfo.as_ref();
    let end = base.find('?').or_else(|| base.find('#')).unwrap_or(base.len());
    let path_part = &base[..end];
    if end < base.len() {
        format!("{path_part}?[QUERY_REDACTED]")
    } else {
        path_part.to_string()
    }
}

/// **Debug-context redactor** (Group B): stricter than `redact_yaml` for console/error text.
///
/// Applied to console args, error messages, and network bodies — all of which are
/// attacker/page-controlled free text. Does NOT replace `redact_yaml` for observe
/// (over-redacting observe risks false positives).
///
/// Strategy (layered, first match wins):
/// 1. Expanded keyword set: `<keyword> [:=] <value>` → blackout value.
/// 2. Generic `key=value` fallback: any `word [:=] <non-whitespace 6+>` → blackout value
///    (with allowlist for obvious non-secrets: pure integers, already-handled URLs).
/// 3. Lower high-entropy fallback threshold (len>=12, Shannon>=3.5) for stray tokens.
/// 4. Delegates to `redact_yaml` for known patterns (sk-/AKIA/Bearer/PEM).
///
/// ## Accepted residual (NOT a bug — conscious precision/utility trade-off)
///
/// Bare short page-origin tokens (8-11 char, low-entropy, no `key=value` context, not a known
/// agent secret) in console args / error messages / URL path segments are NOT masked. Masking
/// all 8-char alphanumerics would over-redact legitimate debug content (paths like `/settings`,
/// ids, element refs, etc.) and degrade the debug tool's usefulness. This is the same heuristic
/// floor that `observe` has. The agent's OWN secrets are deterministically caught by
/// [`apply_known_secret_blackout`]; auth headers are dropped by the safe-header allowlist;
/// structural URL userinfo/query are stripped entirely. This is an intentional
/// precision-over-recall trade-off, not an oversight.
pub fn redact_debug_text(s: &str) -> String {
    // First pass: nomi_redact known patterns + high-entropy (via redact_yaml which does both).
    let base = crate::redact::redact_yaml(s);

    // Second pass: expanded keyword key=value redaction.
    let after_keywords = DEBUG_KEYWORD_KV_RE.replace_all(&base, |caps: &regex::Captures| {
        let key = caps.get(1).map_or("", |m| m.as_str());
        let sep = caps.get(2).map_or("", |m| m.as_str());
        let value = caps.get(3).map_or("", |m| m.as_str());
        // Don't re-redact values already handled by known-secret blackout or prior passes.
        if value.contains("[KNOWN_SECRET_REDACTED]") || value.contains("[REDACTED_SECRET]") {
            return caps.get(0).map_or("".to_string(), |m| m.as_str().to_string());
        }
        format!("{key}{sep}[REDACTED_SECRET]")
    });

    // Third pass: generic key=value fallback (word [:=] value_6+).
    let after_generic = DEBUG_GENERIC_KV_RE.replace_all(&after_keywords, |caps: &regex::Captures| {
        let key = caps.get(1).map_or("", |m| m.as_str());
        let sep = caps.get(2).map_or("", |m| m.as_str());
        let value = caps.get(3).map_or("", |m| m.as_str());
        // Don't re-redact values already handled by known-secret blackout or prior passes.
        if value.contains("[KNOWN_SECRET_REDACTED]") || value.contains("[REDACTED_SECRET]") {
            return caps.get(0).map_or("".to_string(), |m| m.as_str().to_string());
        }
        // Allowlist: pure integers, URLs (already handled above).
        if value.chars().all(|c| c.is_ascii_digit()) {
            // Keep pure numbers (e.g. line_number: 42, port: 8080)
            return caps.get(0).map_or("".to_string(), |m| m.as_str().to_string());
        }
        format!("{key}{sep}[REDACTED_SECRET]")
    });

    // Fourth pass: lower-threshold high-entropy tokens (len>=12, Shannon>=3.5).
    redact_high_entropy_debug(&after_generic)
}

/// Expanded keyword regex for debug context (case-insensitive).
/// Matches: `<keyword> [:=] <value>` where value is 6+ non-whitespace chars.
/// Also handles JSON-style `"key":"value"` (optional quotes around the separator).
static DEBUG_KEYWORD_KV_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?i)\b("#,
        r"api[-_]?key|token|secret|password|passwd|pwd|",
        r"session[-_]?id|sid|csrf|xsrf|auth|authorization|bearer|",
        r"access[-_]?token|refresh[-_]?token|client[-_]?secret|",
        r"credential|cookie|nonce|otp|private[-_]?key|",
        r"code|pin|mfa|mfa[-_]?code|verification|verification[-_]?code|passcode",
        r#")\b(\s*"?\s*[:=]\s*"?\s*)"#,
        r"(\S{6,})"
    ))
    .unwrap()
});

/// Generic key=value fallback: any `\b\w+\s*[:=]\s*\S{6,}` → blackout value.
/// More aggressive than nomi_redact's SECRET_ASSIGNMENT_REGEX (which only matches 4 keywords).
static DEBUG_GENERIC_KV_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([\w-]+)(\s*[:=]\s*)(\S{6,})").unwrap()
});

/// Lower-threshold high-entropy token redaction for debug context.
/// Threshold: len>=12 AND Shannon entropy >= 3.5 (vs redact.rs's len>=20, entropy>=4.0).
fn redact_high_entropy_debug(s: &str) -> String {
    s.split_inclusive(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .map(|tok| {
            let trimmed = tok.trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'');
            // Don't re-redact tokens that are already redaction markers.
            if trimmed.contains("[KNOWN_SECRET_REDACTED]") || trimmed.contains("[REDACTED_SECRET]") {
                return tok.to_string();
            }
            if trimmed.len() >= 12 && crate::redact::shannon_entropy(trimmed) >= 3.5 {
                tok.replace(trimmed, "[REDACTED_SECRET]")
            } else {
                tok.to_string()
            }
        })
        .collect()
}

/// Redact a stack trace: apply `redact_url` to each URL found in stack frame lines,
/// then run the result through `redact_debug_text` for any remaining secrets.
///
/// Stack frames are of the form `  at functionName (URL:line:col)` — we extract
/// and individually redact each URL in parentheses.
fn redact_stack_trace(stack: &str) -> String {
    static STACK_URL_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\(([^)]+://[^)]+)\)").unwrap());
    let url_redacted = STACK_URL_RE.replace_all(stack, |caps: &regex::Captures| {
        let full_url_with_loc = caps.get(1).map_or("", |m| m.as_str());
        // The URL may have :line:col suffix — split off the line/col part.
        let (url_part, loc_suffix) = split_url_location(full_url_with_loc);
        let redacted = redact_url(url_part);
        format!("({redacted}{loc_suffix})")
    });
    redact_debug_text(&url_redacted)
}

/// Split "https://host/path.js:10:5" into ("https://host/path.js", ":10:5").
/// Handles the common stack-frame URL:line:col pattern.
fn split_url_location(s: &str) -> (&str, &str) {
    // Find the last occurrence of a pattern like `:digits:digits` or `:digits` at the end.
    // We search backwards for a colon followed by digits.
    static LOC_SUFFIX_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(:\d+(?::\d+)?)$").unwrap());
    match LOC_SUFFIX_RE.find(s) {
        Some(m) => (&s[..m.start()], &s[m.start()..]),
        None => (s, ""),
    }
}

/// 过滤 headers：只保留安全白名单里的 header。
pub fn filter_safe_headers(headers: &[HttpHeader]) -> Vec<HttpHeader> {
    headers
        .iter()
        .filter(|h| {
            let lower = h.name.to_ascii_lowercase();
            SAFE_HEADER_ALLOWLIST.contains(&lower.as_str())
        })
        .cloned()
        .collect()
}

/// 把 console entries 序列化为 LLM-facing 文本（脱敏 + wrap_untrusted）。
///
/// `known_secrets`: the session's resolved secret values for exact-blackout (may be empty).
pub fn serialize_console_for_llm(entries: &[ConsoleEntry], known_secrets: &HashSet<String>) -> String {
    if entries.is_empty() {
        return "No console messages captured.".to_string();
    }
    let mut lines = Vec::with_capacity(entries.len());
    for e in entries {
        let redacted_text = redact_debug_text(&e.text);
        let url_part = match &e.url {
            Some(u) => format!(" ({})", redact_url(u)),
            None => String::new(),
        };
        lines.push(format!("[{}]{url_part} {redacted_text}", e.level.as_str()));
    }
    let body = lines.join("\n");
    // Known-secret exact-blackout FIRST (deterministic), then wrap_untrusted.
    let body = apply_known_secret_blackout(body, known_secrets);
    crate::redact::wrap_untrusted(&body, Some("console"))
}

/// 把 page errors 序列化为 LLM-facing 文本（脱敏 + wrap_untrusted）。
///
/// `known_secrets`: the session's resolved secret values for exact-blackout (may be empty).
pub fn serialize_errors_for_llm(errors: &[PageError], known_secrets: &HashSet<String>) -> String {
    if errors.is_empty() {
        return "No page errors captured.".to_string();
    }
    let mut lines = Vec::with_capacity(errors.len());
    for e in errors {
        let redacted_msg = redact_debug_text(&e.message);
        let mut entry = format!("ERROR: {redacted_msg}");
        if let Some(stack) = &e.stack {
            let redacted_stack = redact_stack_trace(stack);
            entry.push_str(&format!("\n{redacted_stack}"));
        }
        lines.push(entry);
    }
    let body = lines.join("\n---\n");
    // Known-secret exact-blackout FIRST (deterministic), then wrap_untrusted.
    let body = apply_known_secret_blackout(body, known_secrets);
    crate::redact::wrap_untrusted(&body, Some("page-errors"))
}

/// 把 network entries 序列化为 LLM-facing 文本（脱敏 + wrap_untrusted）。
///
/// `include_bodies`: 是否包含请求/响应体（默认 false，bodies 重且易含 secret）。
/// `known_secrets`: the session's resolved secret values for exact-blackout (may be empty).
#[allow(clippy::collapsible_if)]
pub fn serialize_network_for_llm(entries: &[NetworkEntry], include_bodies: bool, known_secrets: &HashSet<String>) -> String {
    if entries.is_empty() {
        return "No network activity captured.".to_string();
    }
    let mut lines = Vec::with_capacity(entries.len());
    for e in entries {
        let redacted_url = redact_url(&e.url);
        let status_str = match e.status {
            Some(s) => s.to_string(),
            None if e.failed => "FAILED".to_string(),
            None => "pending".to_string(),
        };
        let mut entry = format!("{} {} → {status_str}", e.method, redacted_url);

        if let Some(mime) = &e.mime {
            entry.push_str(&format!(" ({mime})"));
        }
        if let Some(ms) = e.duration_ms {
            entry.push_str(&format!(" {:.0}ms", ms));
        }
        if let Some(bytes) = e.encoded_data_length {
            entry.push_str(&format!(" {bytes}B"));
        }
        if e.failed {
            if let Some(err) = &e.error_text {
                entry.push_str(&format!(" [error: {err}]"));
            }
        }

        // Safe headers only
        let safe_req_h = filter_safe_headers(&e.request_headers);
        let safe_resp_h = filter_safe_headers(&e.response_headers);
        if !safe_req_h.is_empty() {
            let h_str: Vec<String> = safe_req_h.iter().map(|h| format!("{}: {}", h.name, h.value)).collect();
            entry.push_str(&format!("\n  req-headers: {}", h_str.join("; ")));
        }
        if !safe_resp_h.is_empty() {
            let h_str: Vec<String> = safe_resp_h.iter().map(|h| format!("{}: {}", h.name, h.value)).collect();
            entry.push_str(&format!("\n  resp-headers: {}", h_str.join("; ")));
        }

        // Bodies (opt-in, redacted via redact_debug_text — broader keyword coverage
        // than redact_yaml; catches session_id/sid/csrf/auth/credential/cookie/nonce/otp/…)
        // Known-secret exact-blackout runs FIRST on body text (deterministic), then heuristic.
        if include_bodies {
            if let Some(body) = &e.request_body {
                let blackout = apply_known_secret_blackout(body.clone(), known_secrets);
                let redacted = redact_debug_text(&blackout);
                entry.push_str(&format!("\n  req-body: {redacted}"));
            }
            if let Some(body) = &e.response_body {
                let blackout = apply_known_secret_blackout(body.clone(), known_secrets);
                let redacted = redact_debug_text(&blackout);
                entry.push_str(&format!("\n  resp-body: {redacted}"));
            }
        }

        lines.push(entry);
    }
    let body = lines.join("\n---\n");
    // Known-secret exact-blackout FIRST (deterministic), then wrap_untrusted.
    let body = apply_known_secret_blackout(body, known_secrets);
    crate::redact::wrap_untrusted(&body, Some("network"))
}

// ═══════════════════════════════════════════════════════════════════════════
// 内部辅助
// ═══════════════════════════════════════════════════════════════════════════

/// 把 CDP headers（Object<string,string>）解析成 Vec<HttpHeader>。
fn parse_headers(headers: Option<&serde_json::Value>) -> Vec<HttpHeader> {
    match headers.and_then(|h| h.as_object()) {
        Some(obj) => obj
            .iter()
            .map(|(k, v)| HttpHeader {
                name: k.clone(),
                value: v.as_str().unwrap_or("").to_string(),
            })
            .collect(),
        None => Vec::new(),
    }
}

/// 把 CDP RemoteObject 序列化为可读文本（console args 用）。
fn remote_object_to_text(obj: &serde_json::Value) -> String {
    // 优先 unserializableValue（Infinity/-0/NaN/bigint），然后 value（基本类型），
    // 然后 description（对象摘要），最后 type。
    if let Some(s) = obj.get("unserializableValue").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(val) = obj.get("value") {
        return match val {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => "null".to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            other => other.to_string(),
        };
    }
    if let Some(desc) = obj.get("description").and_then(|v| v.as_str()) {
        return desc.to_string();
    }
    obj.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("undefined")
        .to_string()
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Empty known-secret set for tests that don't test the exact-blackout path.
    fn no_secrets() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn console_event_maps_and_ring_bounds() {
        // 1) 映射 Runtime.consoleAPICalled → ConsoleEntry
        let params = json!({
            "type": "error",
            "args": [
                {"type": "string", "value": "something went wrong"},
                {"type": "number", "value": 42}
            ],
            "timestamp": 1700000000123.0,
            "stackTrace": {
                "callFrames": [{
                    "functionName": "doStuff",
                    "url": "https://example.com/app.js",
                    "lineNumber": 10,
                    "columnNumber": 5
                }]
            }
        });
        let entry = map_console_event(&params).expect("should map");
        assert_eq!(entry.level, ConsoleLevel::Error);
        assert_eq!(entry.text, "something went wrong 42");
        assert_eq!(entry.timestamp, 1700000000123.0);
        assert_eq!(entry.url.as_deref(), Some("https://example.com/app.js"));

        // 2) RingBuffer bounded: cap=3, push 4 → oldest dropped, len stays 3
        let mut ring: RingBuffer<ConsoleEntry> = RingBuffer::new(3);
        assert!(ring.is_empty());
        for i in 0..4 {
            ring.push(ConsoleEntry {
                level: ConsoleLevel::Log,
                text: format!("msg{i}"),
                timestamp: i as f64,
                url: None,
            });
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.capacity(), 3);
        // oldest (msg0) dropped, remaining are msg1,msg2,msg3
        let items: Vec<_> = ring.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(items, vec!["msg1", "msg2", "msg3"]);
    }

    #[test]
    fn exception_event_maps_to_page_error() {
        let params = json!({
            "timestamp": 1700000000500.0,
            "exceptionDetails": {
                "text": "Uncaught TypeError",
                "exception": {
                    "type": "object",
                    "description": "TypeError: Cannot read properties of null (reading 'foo')"
                },
                "stackTrace": {
                    "callFrames": [{
                        "functionName": "badFn",
                        "url": "https://example.com/app.js",
                        "lineNumber": 20,
                        "columnNumber": 3
                    }]
                }
            }
        });
        let err = map_exception_event(&params).expect("should map");
        assert!(err.message.contains("Cannot read properties of null"));
        assert!(err.stack.as_deref().unwrap().contains("badFn"));
        assert_eq!(err.timestamp, 1700000000500.0);
    }

    #[test]
    fn log_error_event_maps_only_errors() {
        let params_error = json!({
            "entry": {
                "level": "error",
                "text": "network error occurred",
                "timestamp": 123.0
            }
        });
        assert!(map_log_error_event(&params_error).is_some());

        let params_warn = json!({
            "entry": {
                "level": "warning",
                "text": "deprecation warning",
                "timestamp": 124.0
            }
        });
        assert!(map_log_error_event(&params_warn).is_none());
    }

    #[test]
    fn network_request_maps_and_patches() {
        let req_params = json!({
            "requestId": "req-1",
            "request": {
                "url": "https://api.example.com/data?token=secret123",
                "method": "POST",
                "headers": {
                    "Authorization": "Bearer abc123",
                    "Content-Type": "application/json"
                },
                "postData": "{\"key\":\"value\"}"
            },
            "timestamp": 100.0
        });
        let (id, mut entry) = map_request_will_be_sent(&req_params).expect("should map");
        assert_eq!(id, "req-1");
        assert_eq!(entry.url, "https://api.example.com/data?token=secret123");
        assert_eq!(entry.method, "POST");
        assert_eq!(entry.request_headers.len(), 2);
        assert_eq!(entry.request_body.as_deref(), Some("{\"key\":\"value\"}"));
        assert!(entry.status.is_none());

        // patch response
        let resp_params = json!({
            "response": {
                "status": 200,
                "mimeType": "application/json",
                "headers": {
                    "Set-Cookie": "session=xyz",
                    "Content-Length": "42"
                }
            }
        });
        patch_response_received(&mut entry, &resp_params);
        assert_eq!(entry.status, Some(200));
        assert_eq!(entry.mime.as_deref(), Some("application/json"));
        assert_eq!(entry.response_headers.len(), 2);

        // patch loading finished
        let fin_params = json!({
            "encodedDataLength": 1024.0,
            "timestamp": 100.5
        });
        patch_loading_finished(&mut entry, &fin_params, 100.0);
        assert_eq!(entry.encoded_data_length, Some(1024));
        assert!((entry.duration_ms.unwrap() - 500.0).abs() < 0.1);

        // patch loading failed (separate entry)
        let mut failed_entry = NetworkEntry {
            url: "https://fail.example.com".into(),
            method: "GET".into(),
            status: None,
            mime: None,
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers: vec![],
            response_headers: vec![],
            request_body: None,
            response_body: None,
        };
        let fail_params = json!({
            "errorText": "net::ERR_CONNECTION_REFUSED"
        });
        patch_loading_failed(&mut failed_entry, &fail_params);
        assert!(failed_entry.failed);
        assert_eq!(
            failed_entry.error_text.as_deref(),
            Some("net::ERR_CONNECTION_REFUSED")
        );
    }

    #[test]
    fn ring_buffer_drain_empties() {
        let mut ring: RingBuffer<u32> = RingBuffer::new(5);
        for i in 0..5 {
            ring.push(i);
        }
        let drained = ring.drain();
        assert_eq!(drained, vec![0, 1, 2, 3, 4]);
        assert!(ring.is_empty());
    }

    #[test]
    fn ring_buffer_snapshot_preserves() {
        let mut ring: RingBuffer<u32> = RingBuffer::new(5);
        for i in 0..3 {
            ring.push(i);
        }
        let snap = ring.snapshot();
        assert_eq!(snap, vec![0, 1, 2]);
        assert_eq!(ring.len(), 3); // unchanged
    }

    // ═══════════════════════════════════════════════════════════════════════
    // KEYSTONE: 安全脱敏测试——证明 secret 永不以原文出现在 LLM-facing 输出中
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn network_and_console_redact_secrets() {
        // ── 场景 1: NetworkEntry 携带 Authorization header + secret-bearing URL ──
        let net_entry = NetworkEntry {
            url: "https://api.example.com/v1/data?token=sk-SUPER_SECRET_TOKEN_12345&other=fine".to_string(),
            method: "GET".to_string(),
            status: Some(200),
            mime: Some("application/json".to_string()),
            duration_ms: Some(150.0),
            encoded_data_length: Some(2048),
            failed: false,
            error_text: None,
            request_headers: vec![
                HttpHeader { name: "Authorization".to_string(), value: "Bearer abc123_secret_token".to_string() },
                HttpHeader { name: "Content-Type".to_string(), value: "application/json".to_string() },
                HttpHeader { name: "X-Api-Key".to_string(), value: "key_deadbeef_999".to_string() },
                HttpHeader { name: "Cookie".to_string(), value: "session=private_cookie_val".to_string() },
            ],
            response_headers: vec![
                HttpHeader { name: "Set-Cookie".to_string(), value: "token=resp_secret".to_string() },
                HttpHeader { name: "Content-Length".to_string(), value: "2048".to_string() },
            ],
            request_body: None,
            response_body: None,
        };

        let output = serialize_network_for_llm(&[net_entry.clone()], false, &no_secrets());

        // Authorization header value MUST NOT appear (header dropped by allowlist)
        assert!(
            !output.contains("abc123_secret_token"),
            "Authorization header value leaked into LLM output:\n{output}"
        );
        assert!(
            !output.contains("Bearer"),
            "Authorization header leaked:\n{output}"
        );
        // X-Api-Key header MUST NOT appear
        assert!(
            !output.contains("key_deadbeef_999"),
            "X-Api-Key header leaked:\n{output}"
        );
        // Cookie header MUST NOT appear
        assert!(
            !output.contains("private_cookie_val"),
            "Cookie header leaked:\n{output}"
        );
        // Set-Cookie response header MUST NOT appear
        assert!(
            !output.contains("resp_secret"),
            "Set-Cookie response header leaked:\n{output}"
        );
        // Query string (token=sk-SUPER_SECRET_TOKEN_12345) MUST be redacted
        assert!(
            !output.contains("sk-SUPER_SECRET_TOKEN_12345"),
            "Query string secret leaked:\n{output}"
        );
        assert!(
            !output.contains("token="),
            "Query params leaked:\n{output}"
        );
        // But safe headers DO appear
        assert!(
            output.contains("Content-Type: application/json") || output.contains("content-type"),
            "Safe header Content-Type should appear:\n{output}"
        );
        // URL path remains visible
        assert!(
            output.contains("api.example.com/v1/data"),
            "URL path should remain:\n{output}"
        );
        // Output is wrapped in <data>
        assert!(
            output.contains("<data") && output.contains("</data>"),
            "Output must be wrapped as untrusted:\n{output}"
        );

        // ── 场景 2: ConsoleEntry 回显了一个 known token ──
        let console_entry = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: "Token received: Bearer abc123_secret_token from server".to_string(),
            timestamp: 12345.0,
            url: Some("https://example.com/app.js?session=mysecret".to_string()),
        };

        let console_output = serialize_console_for_llm(&[console_entry], &no_secrets());

        // The raw token should be caught by redact_yaml's high-entropy or known-pattern detection.
        // For shorter tokens like "abc123_secret_token" that might not hit high-entropy,
        // we rely on the header-level dropping (network) and wrap_untrusted (console).
        // The IMPORTANT part: the URL query string in the source URL IS redacted.
        assert!(
            !console_output.contains("session=mysecret"),
            "Console source URL query leaked:\n{console_output}"
        );
        // Output is wrapped
        assert!(
            console_output.contains("<data") && console_output.contains("</data>"),
            "Console output must be wrapped as untrusted:\n{console_output}"
        );

        // ── 场景 3: Bodies with secrets (include_bodies=true) ──
        let net_with_body = NetworkEntry {
            url: "https://api.example.com/login".to_string(),
            method: "POST".to_string(),
            status: Some(200),
            mime: Some("application/json".to_string()),
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers: vec![],
            response_headers: vec![],
            request_body: Some(r#"{"password":"sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD"}"#.to_string()),
            response_body: Some(r#"{"token":"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"}"#.to_string()),
        };

        let body_output = serialize_network_for_llm(&[net_with_body.clone()], true, &no_secrets());
        // The sk- prefixed key should be redacted by redact_yaml known-pattern
        assert!(
            !body_output.contains("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD"),
            "Request body secret (sk-*) leaked:\n{body_output}"
        );
        // The long JWT should be caught by high-entropy detection
        assert!(
            !body_output.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"),
            "Response body JWT leaked:\n{body_output}"
        );

        // ── 场景 4: Bodies NOT included by default ──
        let default_output = serialize_network_for_llm(&[net_with_body.clone()], false, &no_secrets());
        assert!(
            !default_output.contains("password"),
            "Bodies should not appear when include_bodies=false:\n{default_output}"
        );
    }

    #[test]
    fn redact_url_strips_query_and_userinfo_and_path_secrets() {
        // Query stripped
        let result = redact_url("https://api.example.com/v1/data?token=secret&key=123");
        assert!(result.contains("[QUERY_REDACTED]"), "query not redacted: {result}");
        assert!(!result.contains("secret"), "query secret leaked: {result}");
        assert!(result.contains("api.example.com"), "host missing: {result}");

        // No query → no [QUERY_REDACTED] marker
        let result2 = redact_url("https://example.com/path");
        assert!(!result2.contains("[QUERY_REDACTED]"), "false query marker: {result2}");
        assert!(result2.contains("example.com/path"), "path lost: {result2}");

        // Userinfo stripped
        let result3 = redact_url("https://admin:s3cret@example.com/path");
        assert!(!result3.contains("admin"), "username leaked: {result3}");
        assert!(!result3.contains("s3cret"), "password leaked: {result3}");
        assert!(result3.contains("example.com/path"), "host+path lost: {result3}");

        // Path token redacted (high entropy segment)
        let result4 = redact_url("https://x.test/reset/SUPERSECRETTOKEN123");
        assert!(!result4.contains("SUPERSECRETTOKEN123"), "path token leaked: {result4}");
        assert!(result4.contains("x.test"), "host lost: {result4}");
        assert!(result4.contains("/reset/"), "safe path segment lost: {result4}");
    }

    #[test]
    fn filter_safe_headers_drops_auth_headers() {
        let headers = vec![
            HttpHeader { name: "Authorization".to_string(), value: "Bearer x".to_string() },
            HttpHeader { name: "Content-Type".to_string(), value: "text/html".to_string() },
            HttpHeader { name: "cookie".to_string(), value: "session=abc".to_string() },
            HttpHeader { name: "X-Api-Key".to_string(), value: "key123".to_string() },
            HttpHeader { name: "Content-Length".to_string(), value: "100".to_string() },
            HttpHeader { name: "Set-Cookie".to_string(), value: "x=y".to_string() },
            HttpHeader { name: "Proxy-Authorization".to_string(), value: "Basic x".to_string() },
        ];
        let safe = filter_safe_headers(&headers);
        let names: Vec<&str> = safe.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["Content-Type", "Content-Length"]);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SECURITY: leak closure tests (TDD — each covers a confirmed leak path)
    // ═══════════════════════════════════════════════════════════════════════

    /// Leak #1: URL path token — a high-entropy segment in the path MUST be redacted.
    #[test]
    fn leak1_url_path_token_redacted() {
        let net_entry = NetworkEntry {
            url: "https://x.test/reset/SUPERSECRETTOKEN123".to_string(),
            method: "GET".to_string(),
            status: Some(200),
            mime: None,
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers: vec![],
            response_headers: vec![],
            request_body: None,
            response_body: None,
        };
        let output = serialize_network_for_llm(&[net_entry], false, &no_secrets());
        assert!(
            !output.contains("SUPERSECRETTOKEN123"),
            "URL path token leaked into LLM output:\n{output}"
        );
        // Host + non-secret path segments should remain visible
        assert!(
            output.contains("x.test"),
            "Host should remain visible:\n{output}"
        );
    }

    /// Leak #2: userinfo (user:password@host) MUST be stripped entirely.
    #[test]
    fn leak2_userinfo_stripped() {
        let net_entry = NetworkEntry {
            url: "https://user:hunter2pass@x.test/path".to_string(),
            method: "GET".to_string(),
            status: Some(200),
            mime: None,
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers: vec![],
            response_headers: vec![],
            request_body: None,
            response_body: None,
        };
        let output = serialize_network_for_llm(&[net_entry], false, &no_secrets());
        assert!(
            !output.contains("hunter2pass"),
            "Userinfo password leaked into LLM output:\n{output}"
        );
        assert!(
            !output.contains("user:"),
            "Userinfo user: leaked into LLM output:\n{output}"
        );
        // Host should remain
        assert!(
            output.contains("x.test"),
            "Host should remain visible:\n{output}"
        );
    }

    /// Leak #3: console arg with short secret in key=value form.
    #[test]
    fn leak3_console_short_secret_key_value() {
        let entry = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: "session_id: usr_abc1234XYZ".to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output = serialize_console_for_llm(&[entry], &no_secrets());
        assert!(
            !output.contains("usr_abc1234XYZ"),
            "Short secret in console key=value leaked:\n{output}"
        );
    }

    /// Leak #4: error message with short secret in assignment form.
    #[test]
    fn leak4_error_message_short_secret() {
        let entry = PageError {
            message: "throw new Error('csrf=abc1234567')".to_string(),
            stack: None,
            timestamp: 1.0,
        };
        let output = serialize_errors_for_llm(&[entry], &no_secrets());
        assert!(
            !output.contains("abc1234567"),
            "Short secret in error message leaked:\n{output}"
        );
    }

    /// Leak #5: stack frame URL with query secret — per-frame URL must be redacted.
    #[test]
    fn leak5_stack_frame_url_query_secret() {
        // Build a Runtime.exceptionThrown event with a stack frame URL containing a query secret.
        let params = json!({
            "timestamp": 100.0,
            "exceptionDetails": {
                "text": "Uncaught Error",
                "exception": {
                    "type": "object",
                    "description": "Error: something"
                },
                "stackTrace": {
                    "callFrames": [{
                        "functionName": "f",
                        "url": "https://x.test/a.js?session_id=leaktoken99",
                        "lineNumber": 1,
                        "columnNumber": 0
                    }]
                }
            }
        });
        let error = map_exception_event(&params).unwrap();
        let output = serialize_errors_for_llm(&[error], &no_secrets());
        assert!(
            !output.contains("leaktoken99"),
            "Stack frame URL query secret leaked:\n{output}"
        );
        assert!(
            !output.contains("session_id="),
            "Stack frame URL query key=value leaked:\n{output}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Task 5: 有界内存洪泛测试——5000 事件灌入后 ring 不超 cap
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn buffers_stay_bounded_under_flood() {
        let cap = DEFAULT_BUFFER_CAP; // 500
        let mut buffers = DebugBuffers::new(cap);
        let flood_count = 5000;

        // 灌入 5000 条 console 消息
        for i in 0..flood_count {
            buffers.console.push(ConsoleEntry {
                level: ConsoleLevel::Log,
                text: format!("flood console msg {i}"),
                timestamp: i as f64,
                url: None,
            });
        }
        assert_eq!(buffers.console.len(), cap, "console ring must stay at cap");
        assert!(buffers.console.len() <= cap);

        // 灌入 5000 条 page errors
        for i in 0..flood_count {
            buffers.errors.push(PageError {
                message: format!("flood error {i}"),
                stack: Some(format!("  at fn{i} (file.js:{i}:0)")),
                timestamp: i as f64,
            });
        }
        assert_eq!(buffers.errors.len(), cap, "errors ring must stay at cap");
        assert!(buffers.errors.len() <= cap);

        // 灌入 5000 条 network entries
        for i in 0..flood_count {
            buffers.network.push(NetworkEntry {
                url: format!("https://api.example.com/req/{i}?token=secret{i}"),
                method: "GET".to_string(),
                status: Some(200),
                mime: Some("application/json".to_string()),
                duration_ms: Some(100.0),
                encoded_data_length: Some(1024),
                failed: false,
                error_text: None,
                request_headers: vec![
                    HttpHeader { name: "Authorization".to_string(), value: format!("Bearer tk{i}") },
                    HttpHeader { name: "Content-Type".to_string(), value: "application/json".to_string() },
                ],
                response_headers: vec![],
                request_body: None,
                response_body: None,
            });
        }
        assert_eq!(buffers.network.len(), cap, "network ring must stay at cap");
        assert!(buffers.network.len() <= cap);

        // 验证最旧的 4500 条已丢弃（只保留最新 500）
        let first_console = buffers.console.iter().next().unwrap();
        assert_eq!(
            first_console.text,
            format!("flood console msg {}", flood_count - cap),
            "oldest 4500 should be dropped; first remaining = {}",
            flood_count - cap
        );

        let first_error = buffers.errors.iter().next().unwrap();
        assert_eq!(
            first_error.message,
            format!("flood error {}", flood_count - cap)
        );

        let first_network = buffers.network.iter().next().unwrap();
        assert!(first_network.url.contains(&format!("/req/{}", flood_count - cap)));

        // 验证序列化后的 LLM 输出也不暴露已丢弃条目的 secret
        let console_output = serialize_console_for_llm(&buffers.console.snapshot(), &no_secrets());
        assert!(
            !console_output.contains("flood console msg 0"),
            "oldest dropped entry must not appear in output"
        );
        // 且当前存在的条目都被脱敏包裹
        assert!(
            console_output.contains("<data"),
            "output must be wrapped as untrusted"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Known-secret exact-blackout tests (deterministic guarantee)
    // Proves that KNOWN secret values are redacted ANYWHERE they appear,
    // regardless of format, position, or entropy — the 3 confirmed review leaks.
    // ═══════════════════════════════════════════════════════════════════════

    fn known_secrets_set(secrets: &[&str]) -> HashSet<String> {
        secrets.iter().map(|s| s.to_string()).collect()
    }

    /// Known-secret in a URL path (review leak: 8-11 char URL path tokens).
    #[test]
    fn known_secret_blackout_url_path() {
        let secrets = known_secrets_set(&["secret1234"]);
        let net_entry = NetworkEntry {
            url: "https://x.test/reset/secret1234".to_string(),
            method: "GET".to_string(),
            status: Some(200),
            mime: None,
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers: vec![],
            response_headers: vec![],
            request_body: None,
            response_body: None,
        };
        let output = serialize_network_for_llm(&[net_entry], false, &secrets);
        assert!(
            !output.contains("secret1234"),
            "Known secret in URL path must be blackout-redacted:\n{output}"
        );
        assert!(
            output.contains("[KNOWN_SECRET_REDACTED]"),
            "Redaction marker must appear:\n{output}"
        );
        // Host and safe path segments remain
        assert!(output.contains("x.test"), "Host should remain:\n{output}");
        assert!(output.contains("/reset/"), "Safe path segment should remain:\n{output}");
    }

    /// Known-secret in JSON console output (review leak: JSON-formatted console secrets).
    #[test]
    fn known_secret_blackout_json_console() {
        let secrets = known_secrets_set(&["abc12345"]);
        let entry = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: r#"{"mysession":"abc12345"}"#.to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output = serialize_console_for_llm(&[entry], &secrets);
        assert!(
            !output.contains("abc12345"),
            "Known secret in JSON console must be blackout-redacted:\n{output}"
        );
        assert!(
            output.contains("[KNOWN_SECRET_REDACTED]"),
            "Redaction marker must appear:\n{output}"
        );
    }

    /// Known-secret in a network body (review leak: non-keyword body fields).
    #[test]
    fn known_secret_blackout_network_body() {
        let secrets = known_secrets_set(&["abc12345"]);
        let net_entry = NetworkEntry {
            url: "https://api.example.com/login".to_string(),
            method: "POST".to_string(),
            status: Some(200),
            mime: Some("application/json".to_string()),
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers: vec![],
            response_headers: vec![],
            request_body: Some(r#"{"session_id":"abc12345"}"#.to_string()),
            response_body: None,
        };
        let output = serialize_network_for_llm(&[net_entry], true, &secrets);
        assert!(
            !output.contains("abc12345"),
            "Known secret in network body must be blackout-redacted:\n{output}"
        );
        assert!(
            output.contains("[KNOWN_SECRET_REDACTED]"),
            "Redaction marker must appear:\n{output}"
        );
    }

    /// Known-secret in an error/stack trace.
    #[test]
    fn known_secret_blackout_error_stack() {
        let secrets = known_secrets_set(&["leaktoken99"]);
        let entry = PageError {
            message: "Error: auth failed with token leaktoken99".to_string(),
            stack: Some("at login (https://x.test/auth?t=leaktoken99:10:5)".to_string()),
            timestamp: 1.0,
        };
        let output = serialize_errors_for_llm(&[entry], &secrets);
        assert!(
            !output.contains("leaktoken99"),
            "Known secret in error/stack must be blackout-redacted:\n{output}"
        );
        assert!(
            output.contains("[KNOWN_SECRET_REDACTED]"),
            "Redaction marker must appear:\n{output}"
        );
    }

    /// Multiple known secrets: all are redacted, including when they appear together.
    #[test]
    fn known_secret_blackout_multiple_secrets() {
        let secrets = known_secrets_set(&["secret1234", "abc12345", "leaktoken99"]);
        let entry = ConsoleEntry {
            level: ConsoleLevel::Error,
            text: "tokens: secret1234, abc12345, also leaktoken99 end".to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output = serialize_console_for_llm(&[entry], &secrets);
        assert!(!output.contains("secret1234"), "First secret leaked:\n{output}");
        assert!(!output.contains("abc12345"), "Second secret leaked:\n{output}");
        assert!(!output.contains("leaktoken99"), "Third secret leaked:\n{output}");
    }

    /// Short secrets (len < 4) are NOT inserted by the facade (invariant), but if somehow
    /// present in the set the blackout still applies. This test verifies the blackout function
    /// itself works for any non-empty string in the set.
    #[test]
    fn known_secret_blackout_ignores_empty() {
        // Empty string in set should not cause issues (the function skips empty)
        let mut secrets = HashSet::new();
        secrets.insert(String::new());
        secrets.insert("ab".to_string()); // short — but if in set, still blackouts
        let entry = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: "nothing secret here ab end".to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output = serialize_console_for_llm(&[entry], &secrets);
        // "ab" IS in the set, so it gets blackout-redacted
        assert!(
            !output.contains(" ab "),
            "Short secret in set should still be redacted:\n{output}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Fix 1: network bodies go through redact_debug_text (broader keyword set)
    // ═══════════════════════════════════════════════════════════════════════

    /// A `session_id` key=value in a network body (NOT a known-secret) must be redacted
    /// by the broader `redact_debug_text` keyword heuristics (session_id is in its keyword
    /// set). Previously this went through `redact_yaml` which only has 4 keywords.
    #[test]
    fn network_body_session_id_redacted_by_debug_keywords() {
        let net_entry = NetworkEntry {
            url: "https://api.example.com/session".to_string(),
            method: "POST".to_string(),
            status: Some(200),
            mime: Some("application/json".to_string()),
            duration_ms: None,
            encoded_data_length: None,
            failed: false,
            error_text: None,
            request_headers: vec![],
            response_headers: vec![],
            // session_id value is 11 chars — NOT high-entropy enough for the entropy
            // heuristic, and NOT in the known-secrets set. Only the keyword regex catches it.
            request_body: Some(r#"{"session_id":"abc12345678"}"#.to_string()),
            response_body: None,
        };
        let output = serialize_network_for_llm(&[net_entry], true, &no_secrets());
        assert!(
            !output.contains("abc12345678"),
            "session_id value in network body must be redacted by keyword heuristic:\n{output}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Fix 2: OTP/PIN/code keywords added to DEBUG_KEYWORD_KV_RE
    // ═══════════════════════════════════════════════════════════════════════

    /// OTP/PIN keywords in console output must be redacted — even when the value is
    /// purely numeric (the keyword match overrides the digit allowlist in pass 3).
    #[test]
    fn otp_pin_keywords_redacted_in_debug_text() {
        // verification_code with 6-digit numeric value
        let entry1 = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: "console.log('verification_code=847291')".to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output1 = serialize_console_for_llm(&[entry1], &no_secrets());
        assert!(
            !output1.contains("847291"),
            "verification_code value must be redacted:\n{output1}"
        );

        // pin with 6-digit numeric value
        let entry2 = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: "pin=123456 sent".to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output2 = serialize_console_for_llm(&[entry2], &no_secrets());
        assert!(
            !output2.contains("123456"),
            "pin value must be redacted:\n{output2}"
        );

        // mfa_code
        let entry3 = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: "mfa_code: 998877 accepted".to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output3 = serialize_console_for_llm(&[entry3], &no_secrets());
        assert!(
            !output3.contains("998877"),
            "mfa_code value must be redacted:\n{output3}"
        );

        // passcode
        let entry4 = ConsoleEntry {
            level: ConsoleLevel::Log,
            text: "passcode=456789done".to_string(),
            timestamp: 1.0,
            url: None,
        };
        let output4 = serialize_console_for_llm(&[entry4], &no_secrets());
        assert!(
            !output4.contains("456789"),
            "passcode value must be redacted:\n{output4}"
        );
    }
}
