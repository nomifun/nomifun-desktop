//! **storage_state（cookie + localStorage，W4b/W4c）**：默认（全局共享）浏览器登录态的
//! cookie + localStorage ↔ storage_state 序列化结构 + 捕获/恢复的**纯逻辑**转换
//! （DESIGN §17 + IMPLEMENTATION-PLAN-P3 裁决⑥）。
//!
//! ## 子范围
//! - **W4b（cookie）**：cookie 的捕获/恢复机制 + storage_state 的 cookie 部分结构（见
//!   [`StorageStateCookie`]）。
//! - **W4c（localStorage，本次）**：localStorage 的**origin-bound** 捕获/恢复——加
//!   [`OriginStorage`]（一个 origin 的 `Vec<LocalStorageItem>` 键值对）。localStorage **按 origin 分区**
//!   （`https://a.example` 与 `https://b.example` 的 localStorage 互不可见，也不能从一个 origin 全局
//!   set 另一 origin 的 localStorage）——故结构按 origin 聚合，捕获/恢复都是 origin-bound（捕获取
//!   **当前页面 origin** 的 `Object.entries(localStorage)`；恢复在**目标 origin 上下文**注入 `setItem`，
//!   见 [`crate::backend::CdpBackend::capture_local_storage`]/`restore_local_storage`）。
//!   **IndexedDB** 完整序列化（含二进制/结构化克隆值）复杂，W4c **best-effort = TODO 占位**
//!   （见 [`OriginStorage::index_db`] 文档），localStorage 是 W4c 必须项。
//! - **持久化到磁盘 vault**（keyring / AES-GCM machine-bound key）是 **W4d**——本模块只做**内存往返**
//!   （capture → [`StorageState`] → restore），不碰任何持久化 I/O。
//!
//! ## 为什么自定义 [`StorageStateCookie`] 而非直接存 chromiumoxide 的 `Cookie`/`CookieParam`
//! CDP 的捕获侧 [`network::Cookie`](chromiumoxide::cdp::browser_protocol::network::Cookie) 与恢复侧
//! [`network::CookieParam`](chromiumoxide::cdp::browser_protocol::network::CookieParam) 是**两个不同
//! 结构**（前者把 `priority`/`sourceScheme`/`sourcePort` 设为非 Option 且多了 `size`/`session`/
//! `partitionKeyOpaque`；后者全是 Option 且多了 `url`/`sameParty`）。直接来回转既丢字段又类型不匹配。
//! 故本模块定义一个**自有的、对 storage_state 稳定的** [`StorageStateCookie`]：
//! - **捕获**：从 `network::Cookie` 取**全部登录态相关字段**（裁决⑥点名的 partitionKey + sameSite +
//!   domain/path/expires/httpOnly/secure，外加 priority/sourceScheme/sourcePort 保真）。
//! - **恢复**：转成 `network::CookieParam` 原样灌（partitionKey/sameSite 原样），`Storage.setCookies`
//!   写回默认 browser context。
//! - **序列化**：`Serialize`/`Deserialize`（W4d vault 直接存它；本任务只验内存往返保真）。
//!
//! **绝不丢字段**：partitionKey（CHIPS 分区 cookie，丢了恢复后跨站登录态失效）+ sameSite（丢了
//! 浏览器按默认 Lax 处理，可能越界或登录态失效）+ httpOnly/secure（安全语义）+ expires（持久 vs
//! session）——任一丢失都会令恢复后的登录态不保真。本模块的纯逻辑测试就钉死「全字段往返不丢」。

use base64::Engine as _;
use chromiumoxide::cdp::browser_protocol::network::{
    Cookie, CookieParam, CookiePartitionKey, CookiePriority, CookieSameSite, CookieSourceScheme,
    TimeSinceEpoch,
};
use serde::{Deserialize, Serialize};

/// **storage_state（W4b cookie + W4c localStorage）**：默认（全局共享）browser context 的持久登录态快照。
///
/// `cookies`（W4b）+ `local_storage`（W4c，origin-bound）。**IndexedDB 是 best-effort/TODO**
/// （[`OriginStorage::index_db`]）。**W4d 把本结构存进磁盘 vault**（machine-bound key）。本任务（W4c）
/// 只做内存往返（capture/restore）。
///
/// 可塞进 `EngineConfig.storage_state`（G1 预留的 `Option<serde_json::Value>`）——
/// [`StorageState::to_json`] / [`StorageState::from_json`] 在 `Value` 与本结构间转换（引擎构造期
/// 上层灌 `Value`，W4d 接线时再决定是否把 `EngineConfig.storage_state` 直接换成本类型）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageState {
    /// 默认 browser context 的全部 cookie（`Storage.getCookies` 捕获，原样保真）。
    #[serde(default)]
    pub cookies: Vec<StorageStateCookie>,
    /// **W4c：per-origin localStorage 快照**（origin-bound，每 origin 一个 [`OriginStorage`]）。
    /// `#[serde(default)]` 向后兼容——W4b 已存的 cookie-only storage_state（无此键）反序列化为空
    /// `local_storage`（不破坏旧 vault 状态）。捕获/恢复见
    /// [`crate::backend::CdpBackend::capture_local_storage`] / `restore_local_storage`。
    #[serde(default)]
    pub local_storage: Vec<OriginStorage>,
}

/// **W4c：一个 origin 的 storage 快照**（localStorage 必须 + IndexedDB best-effort）。
///
/// localStorage **按 origin 分区**（同源策略）：`https://a.example` 与 `https://b.example` 的
/// localStorage 互不可见，也无法跨 origin 全局 set——故 storage_state 按 origin 聚合，捕获取**当前
/// 页面 origin**、恢复在**目标 origin 上下文**注入（见模块 doc）。`origin` 是 `location.origin` 形态
/// （scheme + host + 非默认端口，无尾斜杠，如 `https://example.com`、`http://localhost:8080`、
/// `file://`）——恢复时据它判定「当前页面是否就是该 origin」（origin-bound 注入）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginStorage {
    /// 该 storage 所属 origin（`location.origin`：scheme://host[:port]，无尾斜杠）。
    pub origin: String,
    /// 该 origin 的 localStorage 全部键值对（捕获自 `Object.entries(localStorage)`，恢复用
    /// `localStorage.setItem(name, value)`）。键值都是字符串（localStorage 规范：值恒字符串）。
    #[serde(default)]
    pub local_storage: Vec<LocalStorageItem>,
    /// **IndexedDB（origin-bound 完整序列化）**：本字段持有该 origin 下所有 IndexedDB 数据库的
    /// dump（object stores + records + 二进制哨兵编码）。`None` = 该 origin 无 IndexedDB 数据（或
    /// 捕获时跳过）。
    /// `#[serde(default, skip_serializing_if)]` 使其缺省不污染 JSON、向后兼容旧 vault。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_db: Option<IndexedDbDump>,
}

/// **W4c：一条 localStorage 键值对**（对 vault 稳定的自有结构）。localStorage 规范键值恒字符串。
/// 序列化字段名 `name`/`value`（与 Playwright storage_state JSON 的 localStorage 项形态一致，
/// W4d vault / 跨端可读）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStorageItem {
    /// localStorage 键。
    pub name: String,
    /// localStorage 值（字符串——localStorage 规范值恒字符串；JSON 等结构由页面自行编码进字符串）。
    pub value: String,
}

// ── IndexedDB 序列化类型（storage_state 持久化 IndexedDB）──────────────────────────

/// **IndexedDB 完整 dump**：一个 origin 下所有 IndexedDB 数据库的序列化快照。
///
/// 结构：`databases: Vec<IdbDatabase>`——每个 DB 含 name/version/stores（object store 列表）。
/// 二进制值（Blob/ArrayBuffer/typed arrays）编码为 `{"__b64__":"<base64>"}` 哨兵对象，
/// 恢复时解码回原始二进制。这使 JSON 序列化/vault 加密的通路不被非 UTF-8 内容打断。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedDbDump {
    /// 该 origin 下所有 IndexedDB 数据库。
    #[serde(default)]
    pub databases: Vec<IdbDatabase>,
}

/// **一个 IndexedDB 数据库**（name + version + object stores）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdbDatabase {
    /// 数据库名（`indexedDB.open(name)` 的那个 name）。
    pub name: String,
    /// 数据库版本号（`indexedDB.open(name, version)` 的 version；`IDBDatabase.version`）。
    pub version: u64,
    /// 该数据库的所有 object store 快照。
    #[serde(default)]
    pub stores: Vec<IdbStore>,
}

/// **一个 IndexedDB object store 的快照**（name + keyPath + records）。
///
/// `key_path`：如果 store 使用 in-line key（`createObjectStore("s", {keyPath:"id"})`），
/// 则为 `Some("id")`；out-of-line key（无 keyPath / autoIncrement only）则为 `None`。
/// `auto_increment`：store 是否设了 `autoIncrement: true`。
/// `records`：该 store 的全部记录（`getAll()` 的结果，经 JSON 序列化 + base64 二进制哨兵编码）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdbStore {
    /// object store 名。
    pub name: String,
    /// in-line keyPath（`None` = out-of-line key / 无 keyPath）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    /// 是否 autoIncrement。
    #[serde(default)]
    pub auto_increment: bool,
    /// object store 内全部记录（JSON `Value`；二进制值编码为 `{"__b64__":"..."}`）。
    #[serde(default)]
    pub records: Vec<serde_json::Value>,
}

/// **base64 二进制哨兵**的 JSON key。collector JS 将 ArrayBuffer/Blob/typed array 编码为
/// `{"__b64__":"<standard base64 encoding>"}` 对象，本常量是识别该哨兵的键名。
pub const IDB_BASE64_SENTINEL: &str = "__b64__";

impl IndexedDbDump {
    /// 对 dump 中所有 record value 里的 `{"__b64__":"..."}` 哨兵执行解码验证（不改原值）。
    /// 返回 `true` 如果所有哨兵都是合法 base64（或无哨兵），`false` 如果存在非法编码。
    /// 用于测试断言。
    pub fn validate_base64_sentinels(&self) -> bool {
        for db in &self.databases {
            for store in &db.stores {
                for record in &store.records {
                    if !validate_value_sentinels(record) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// 递归验证一个 JSON value 内所有 base64 哨兵是否可解码。
fn validate_value_sentinels(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(map) => {
            if map.len() == 1
                && let Some(b64_val) = map.get(IDB_BASE64_SENTINEL)
            {
                if let Some(s) = b64_val.as_str() {
                    return base64::engine::general_purpose::STANDARD.decode(s).is_ok();
                }
                return false;
            }
            map.values().all(validate_value_sentinels)
        }
        serde_json::Value::Array(arr) => arr.iter().all(validate_value_sentinels),
        _ => true,
    }
}

/// 将原始字节编码为 base64 哨兵 JSON 对象 `{"__b64__":"<base64>"}`。
pub fn encode_binary_sentinel(bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({ IDB_BASE64_SENTINEL: base64::engine::general_purpose::STANDARD.encode(bytes) })
}

/// 如果 `v` 是 base64 哨兵对象 `{"__b64__":"..."}` 则解码出原始字节，否则返回 `None`。
pub fn decode_binary_sentinel(v: &serde_json::Value) -> Option<Vec<u8>> {
    let map = v.as_object()?;
    if map.len() != 1 {
        return None;
    }
    let b64_str = map.get(IDB_BASE64_SENTINEL)?.as_str()?;
    base64::engine::general_purpose::STANDARD.decode(b64_str).ok()
}

impl OriginStorage {
    /// 便捷构造：从 origin + (key,value) 对建一个 localStorage-only 的 [`OriginStorage`]（IndexedDB None）。
    pub fn new_local_storage(
        origin: impl Into<String>,
        items: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            origin: origin.into(),
            local_storage: items
                .into_iter()
                .map(|(name, value)| LocalStorageItem { name, value })
                .collect(),
            index_db: None,
        }
    }
}

/// **一条 cookie 的 storage_state 表示**（对 vault 稳定的自有结构）。字段是 cookie 登录态的**全集**
/// （capture 从 `network::Cookie` 取、restore 转 `network::CookieParam` 灌），命名与 CDP 对齐。
///
/// 序列化用 camelCase（与 CDP / storage_state JSON 习惯一致，W4d vault / 跨端可读）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStateCookie {
    /// cookie 名。
    pub name: String,
    /// cookie 值。**注**：W4b 不在此脱敏——storage_state 是登录态 vault 的内部表示（W4d machine-bound
    /// key 加密落盘），**绝不**进 LLM 上下文（LLM 永不见 storage_state；脱敏在 observe 序列化层，
    /// 见 [`crate::redact`]）。故此处保留真实值（恢复登录态必需）。
    pub value: String,
    /// cookie 域（如 `.example.com`）。
    pub domain: String,
    /// cookie 路径（如 `/`）。
    pub path: String,
    /// 过期时间（UNIX 秒）。`-1.0` = 未设过期（session cookie，配合 `session=true`）。
    pub expires: f64,
    /// 是否 http-only（JS 读不到；安全语义，勿丢）。
    pub http_only: bool,
    /// 是否 secure（仅 HTTPS 发送；安全语义，勿丢）。
    pub secure: bool,
    /// 是否 session cookie（无持久过期）。捕获自 `Cookie.session`；恢复时不直接进 `CookieParam`
    /// （CookieParam 无 `session` 字段——session 性由「`expires` 是否设」表达），仅作 storage_state 信息位。
    pub session: bool,
    /// SameSite 策略（`Strict`/`Lax`/`None`）。`None`（Rust Option None）= cookie 未显式设 SameSite
    /// （浏览器按其默认处理）。**勿丢**：丢了恢复后可能按默认 Lax，越界 / 跨站登录态失效。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub same_site: Option<SameSite>,
    /// cookie 优先级（`Low`/`Medium`/`High`）。CDP 捕获侧恒给值；保真灌回。
    pub priority: Priority,
    /// source scheme（`Unset`/`NonSecure`/`Secure`）。CDP 捕获侧恒给值；保真灌回。
    pub source_scheme: SourceScheme,
    /// source port（`-1` = 未指定）。保真灌回。
    pub source_port: i64,
    /// **CHIPS 分区 cookie 的 partition key**（`topLevelSite` + `hasCrossSiteAncestor`）。`None` =
    /// 非分区 cookie。**绝不丢**：分区 cookie 丢了 partitionKey，恢复后会被当非分区写入 / 写入失败，
    /// 跨站嵌入场景（CHIPS）的登录态失效。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub partition_key: Option<PartitionKey>,
}

/// SameSite（镜像 CDP [`CookieSameSite`]，对 storage_state 序列化稳定）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

/// cookie 优先级（镜像 CDP [`CookiePriority`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Medium,
    High,
}

/// source scheme（镜像 CDP [`CookieSourceScheme`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceScheme {
    Unset,
    NonSecure,
    Secure,
}

/// **CHIPS partition key**（镜像 CDP [`CookiePartitionKey`]）。`top_level_site` = 设置 cookie 时
/// 浏览器访问的顶层站点；`has_cross_site_ancestor` = 是否有跨站祖先帧。**两字段缺一恢复后分区错位**。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionKey {
    pub top_level_site: String,
    pub has_cross_site_ancestor: bool,
}

// ── CDP ↔ storage_state 枚举互转（纯逻辑，便于单测）───────────────────────────────

impl From<CookieSameSite> for SameSite {
    fn from(v: CookieSameSite) -> Self {
        match v {
            CookieSameSite::Strict => SameSite::Strict,
            CookieSameSite::Lax => SameSite::Lax,
            CookieSameSite::None => SameSite::None,
        }
    }
}
impl From<SameSite> for CookieSameSite {
    fn from(v: SameSite) -> Self {
        match v {
            SameSite::Strict => CookieSameSite::Strict,
            SameSite::Lax => CookieSameSite::Lax,
            SameSite::None => CookieSameSite::None,
        }
    }
}

impl From<CookiePriority> for Priority {
    fn from(v: CookiePriority) -> Self {
        match v {
            CookiePriority::Low => Priority::Low,
            CookiePriority::Medium => Priority::Medium,
            CookiePriority::High => Priority::High,
        }
    }
}
impl From<Priority> for CookiePriority {
    fn from(v: Priority) -> Self {
        match v {
            Priority::Low => CookiePriority::Low,
            Priority::Medium => CookiePriority::Medium,
            Priority::High => CookiePriority::High,
        }
    }
}

impl From<CookieSourceScheme> for SourceScheme {
    fn from(v: CookieSourceScheme) -> Self {
        match v {
            CookieSourceScheme::Unset => SourceScheme::Unset,
            CookieSourceScheme::NonSecure => SourceScheme::NonSecure,
            CookieSourceScheme::Secure => SourceScheme::Secure,
        }
    }
}
impl From<SourceScheme> for CookieSourceScheme {
    fn from(v: SourceScheme) -> Self {
        match v {
            SourceScheme::Unset => CookieSourceScheme::Unset,
            SourceScheme::NonSecure => CookieSourceScheme::NonSecure,
            SourceScheme::Secure => CookieSourceScheme::Secure,
        }
    }
}

impl From<CookiePartitionKey> for PartitionKey {
    fn from(v: CookiePartitionKey) -> Self {
        PartitionKey {
            top_level_site: v.top_level_site,
            has_cross_site_ancestor: v.has_cross_site_ancestor,
        }
    }
}
impl From<PartitionKey> for CookiePartitionKey {
    fn from(v: PartitionKey) -> Self {
        CookiePartitionKey {
            top_level_site: v.top_level_site,
            has_cross_site_ancestor: v.has_cross_site_ancestor,
        }
    }
}

// ── cookie 捕获/恢复的核心纯逻辑转换（CDP Cookie → StorageStateCookie → CookieParam）─────

impl StorageStateCookie {
    /// **[纯逻辑] 捕获**：把 CDP `Network.Cookie`（`Storage.getCookies` 的元素）取成 storage_state
    /// cookie——**全字段保真**（partitionKey / sameSite / domain/path/expires/httpOnly/secure /
    /// priority/sourceScheme/sourcePort / session）。不进浏览器，便于单测。
    pub fn from_cdp_cookie(c: Cookie) -> Self {
        Self {
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            expires: c.expires,
            http_only: c.http_only,
            secure: c.secure,
            session: c.session,
            same_site: c.same_site.map(SameSite::from),
            priority: Priority::from(c.priority),
            source_scheme: SourceScheme::from(c.source_scheme),
            source_port: c.source_port,
            partition_key: c.partition_key.map(PartitionKey::from),
        }
    }

    /// **[纯逻辑] 恢复**：转成 CDP `Network.CookieParam`（`Storage.setCookies` 的元素）——**原样灌**
    /// partitionKey / sameSite / 全字段。不进浏览器，便于单测。
    ///
    /// 转换要点：
    /// - **不设 `url`**（设了 url 会让 CDP 据 url 推 domain/path/source_scheme，覆盖我们显式给的
    ///   domain/path）——我们显式给 `domain`+`path`，让 cookie 原样落回原域，不被 url 推断扰动。
    /// - **`expires`**：`-1.0`（session cookie）→ `None`（CookieParam 无 expires = session cookie，
    ///   语义等价）；其它值原样进 `TimeSinceEpoch`（持久 cookie）。
    /// - **`session` 字段无对应**：CookieParam 无 `session`——session 性由「expires 是否设」表达（上一条
    ///   已据 `expires==-1` 映射），故 `session` 不单独传（仅作 storage_state 信息位）。
    /// - partitionKey / sameSite / priority / sourceScheme / sourcePort / httpOnly / secure 原样灌。
    pub fn to_cookie_param(&self) -> CookieParam {
        // session cookie（expires == -1）→ 不设 expires；否则原样持久过期时间。
        let expires = if self.expires < 0.0 {
            None
        } else {
            Some(TimeSinceEpoch::new(self.expires))
        };
        CookieParam {
            name: self.name.clone(),
            value: self.value.clone(),
            // 显式 domain/path，不设 url（见方法 doc：避免 url 推断覆盖）。
            url: None,
            domain: Some(self.domain.clone()),
            path: Some(self.path.clone()),
            secure: Some(self.secure),
            http_only: Some(self.http_only),
            same_site: self.same_site.map(CookieSameSite::from),
            expires,
            priority: Some(CookiePriority::from(self.priority)),
            // sameParty 已被 Chrome 弃用（First-Party Sets 改用 CHIPS partitionKey）；捕获侧 Cookie 无此
            // 字段，恢复不设（None）。
            same_party: None,
            source_scheme: Some(CookieSourceScheme::from(self.source_scheme)),
            source_port: Some(self.source_port),
            partition_key: self.partition_key.clone().map(CookiePartitionKey::from),
        }
    }
}

impl StorageState {
    /// **[纯逻辑] 从 CDP `Storage.getCookies` 的 cookie 数组建 storage_state**（全字段保真）。
    /// `local_storage` 留空（cookie 与 localStorage 是两条独立捕获路径——cookie 走 `Storage.getCookies`，
    /// localStorage 走注入 `Object.entries`，见 [`crate::backend::CdpBackend`]）。
    pub fn from_cdp_cookies(cookies: Vec<Cookie>) -> Self {
        Self {
            cookies: cookies.into_iter().map(StorageStateCookie::from_cdp_cookie).collect(),
            local_storage: Vec::new(),
        }
    }

    /// **[纯逻辑] 转成 CDP `Storage.setCookies` 的 `CookieParam` 数组**（原样灌）。
    pub fn to_cookie_params(&self) -> Vec<CookieParam> {
        self.cookies.iter().map(StorageStateCookie::to_cookie_param).collect()
    }

    /// **[纯逻辑] 转 `serde_json::Value`**（塞进 `EngineConfig.storage_state` 的 `Option<Value>`）。
    /// 失败（理论上不会——本结构全可序列化）→ `Err`，绝不 panic。
    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }

    /// **[纯逻辑] 从 `serde_json::Value` 解析**（`EngineConfig.storage_state` 灌入侧）。缺 `cookies`
    /// 键 → 空（`#[serde(default)]`，向后兼容 W4c 加字段）。
    pub fn from_json(v: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一条**全字段非默认**的 CDP `Cookie`（持久 + partitionKey + sameSite=None + httpOnly +
    /// secure），覆盖所有保真字段。
    fn full_cdp_cookie() -> Cookie {
        Cookie {
            name: "session_id".into(),
            value: "deadbeef-secret-token".into(),
            domain: ".example.com".into(),
            path: "/app".into(),
            expires: 1_900_000_000.0,
            size: 33,
            http_only: true,
            secure: true,
            session: false,
            same_site: Some(CookieSameSite::None),
            priority: CookiePriority::High,
            source_scheme: CookieSourceScheme::Secure,
            source_port: 443,
            partition_key: Some(CookiePartitionKey {
                top_level_site: "https://embedder.example".into(),
                has_cross_site_ancestor: true,
            }),
            partition_key_opaque: Some(false),
        }
    }

    /// 造一条 **session cookie**（expires=-1，session=true，无 partitionKey，无 sameSite）。
    fn session_cdp_cookie() -> Cookie {
        Cookie {
            name: "csrf".into(),
            value: "tok".into(),
            domain: "example.com".into(),
            path: "/".into(),
            expires: -1.0,
            size: 7,
            http_only: false,
            secure: false,
            session: true,
            same_site: None,
            priority: CookiePriority::Medium,
            source_scheme: CookieSourceScheme::NonSecure,
            source_port: 80,
            partition_key: None,
            partition_key_opaque: None,
        }
    }

    #[test]
    fn capture_preserves_all_fields_including_chips_and_samesite() {
        // 捕获：CDP Cookie → StorageStateCookie，全字段保真（裁决⑥点名的 partitionKey + sameSite 等）。
        let ss = StorageStateCookie::from_cdp_cookie(full_cdp_cookie());
        assert_eq!(ss.name, "session_id");
        assert_eq!(ss.value, "deadbeef-secret-token");
        assert_eq!(ss.domain, ".example.com");
        assert_eq!(ss.path, "/app");
        assert_eq!(ss.expires, 1_900_000_000.0);
        assert!(ss.http_only);
        assert!(ss.secure);
        assert!(!ss.session);
        assert_eq!(ss.same_site, Some(SameSite::None));
        assert_eq!(ss.priority, Priority::High);
        assert_eq!(ss.source_scheme, SourceScheme::Secure);
        assert_eq!(ss.source_port, 443);
        // **CHIPS partitionKey 保真**（丢了恢复后跨站登录态失效）。
        let pk = ss.partition_key.as_ref().expect("partitionKey must be captured");
        assert_eq!(pk.top_level_site, "https://embedder.example");
        assert!(pk.has_cross_site_ancestor);
    }

    #[test]
    fn restore_param_carries_chips_and_samesite_and_explicit_domain() {
        // 恢复：StorageStateCookie → CookieParam，partitionKey/sameSite/全字段原样灌，显式 domain（不设 url）。
        let ss = StorageStateCookie::from_cdp_cookie(full_cdp_cookie());
        let p = ss.to_cookie_param();
        assert_eq!(p.name, "session_id");
        assert_eq!(p.value, "deadbeef-secret-token");
        // 显式 domain/path，绝不设 url（避免 url 推断覆盖 domain/source_scheme）。
        assert_eq!(p.url, None, "must NOT set url (would override explicit domain/path)");
        assert_eq!(p.domain.as_deref(), Some(".example.com"));
        assert_eq!(p.path.as_deref(), Some("/app"));
        assert_eq!(p.secure, Some(true));
        assert_eq!(p.http_only, Some(true));
        assert_eq!(p.same_site, Some(CookieSameSite::None));
        assert_eq!(p.priority, Some(CookiePriority::High));
        assert_eq!(p.source_scheme, Some(CookieSourceScheme::Secure));
        assert_eq!(p.source_port, Some(443));
        // 持久 cookie：expires 原样进 TimeSinceEpoch（非 session）。
        let exp = p.expires.expect("persistent cookie must carry expires");
        assert_eq!(*exp.inner(), 1_900_000_000.0);
        // **CHIPS partitionKey 原样灌回**。
        let pk = p.partition_key.as_ref().expect("partitionKey must be restored");
        assert_eq!(pk.top_level_site, "https://embedder.example");
        assert!(pk.has_cross_site_ancestor);
    }

    #[test]
    fn session_cookie_maps_expires_to_none() {
        // session cookie（expires==-1）→ CookieParam 不设 expires（session 性由「无 expires」表达）。
        let ss = StorageStateCookie::from_cdp_cookie(session_cdp_cookie());
        assert!(ss.session);
        assert_eq!(ss.expires, -1.0);
        let p = ss.to_cookie_param();
        assert_eq!(p.expires, None, "session cookie must NOT carry expires");
        // 无 sameSite / 无 partitionKey 原样传 None（不臆造默认）。
        assert_eq!(p.same_site, None);
        assert_eq!(p.partition_key, None);
    }

    #[test]
    fn json_round_trip_preserves_all_fields() {
        // **核心：往返保真**——StorageState → JSON → StorageState 全字段不丢（W4d vault 存的就是这个 JSON）。
        let original = StorageState::from_cdp_cookies(vec![full_cdp_cookie(), session_cdp_cookie()]);
        let json = original.to_json().expect("to_json");
        let back = StorageState::from_json(json).expect("from_json");
        assert_eq!(original, back, "storage_state must round-trip through JSON without loss");
        // 显式再钉死 CHIPS partitionKey 经 JSON 往返不丢（最易被丢的字段）。
        let pk = back.cookies[0].partition_key.as_ref().expect("partitionKey survives JSON");
        assert_eq!(pk.top_level_site, "https://embedder.example");
        assert!(pk.has_cross_site_ancestor);
    }

    #[test]
    fn json_uses_camel_case_and_carries_partition_key() {
        // 序列化 camelCase（与 CDP / storage_state JSON 习惯一致；W4d vault / 跨端可读）。
        let ss = StorageState::from_cdp_cookies(vec![full_cdp_cookie()]);
        let s = serde_json::to_string(&ss).expect("serialize");
        assert!(s.contains("\"httpOnly\""), "expected camelCase httpOnly: {s}");
        assert!(s.contains("\"sameSite\""), "expected camelCase sameSite: {s}");
        assert!(s.contains("\"partitionKey\""), "expected partitionKey serialized: {s}");
        assert!(s.contains("\"topLevelSite\""), "expected camelCase topLevelSite: {s}");
        assert!(s.contains("\"hasCrossSiteAncestor\""), "expected hasCrossSiteAncestor: {s}");
    }

    #[test]
    fn empty_and_missing_cookies_key_are_backward_compatible() {
        // 缺 cookies 键（W4c 之前 / 空状态）→ 空 cookies（#[serde(default)] 向后兼容）。
        let empty = StorageState::from_json(serde_json::json!({})).expect("from empty obj");
        assert!(empty.cookies.is_empty());
        // 空 cookies 往返保真。
        let none_state = StorageState::default();
        assert_eq!(none_state.cookies.len(), 0);
        let back = StorageState::from_json(none_state.to_json().unwrap()).unwrap();
        assert_eq!(back, none_state);
        // 空 storage_state → 空 CookieParam 数组（restore 灌空 = no-op）。
        assert!(none_state.to_cookie_params().is_empty());
    }

    #[test]
    fn capture_then_restore_then_recapture_is_stable() {
        // 模拟「capture → restore → 再 capture」的稳定性（CookieParam 不含 size/session/partitionKeyOpaque，
        // 但 storage_state 关心的登录态字段在两次 capture 间应一致）。这里用 CookieParam→（伪）Cookie 验
        // 不现实（CookieParam 字段更少），故改为验：同一 StorageStateCookie 经 to_cookie_param 再手工核对
        // 关键字段与原 StorageStateCookie 一致（恢复链路不丢登录态字段）。
        let ss = StorageStateCookie::from_cdp_cookie(full_cdp_cookie());
        let p = ss.to_cookie_param();
        // 恢复参数携带的登录态关键字段 == 捕获时的字段（保真链）。
        assert_eq!(p.name, ss.name);
        assert_eq!(p.value, ss.value);
        assert_eq!(p.domain.as_deref(), Some(ss.domain.as_str()));
        assert_eq!(p.path.as_deref(), Some(ss.path.as_str()));
        assert_eq!(p.http_only, Some(ss.http_only));
        assert_eq!(p.secure, Some(ss.secure));
        assert_eq!(p.same_site.map(SameSite::from), ss.same_site);
        assert_eq!(
            p.partition_key.map(PartitionKey::from),
            ss.partition_key
        );
    }

    // ── W4c：localStorage（origin-bound）纯逻辑往返 ──────────────────────────────

    /// 造一个含两 origin localStorage 的 storage_state（覆盖多 origin + 空值/特殊字符键值）。
    fn local_storage_state() -> StorageState {
        StorageState {
            cookies: vec![],
            local_storage: vec![
                OriginStorage::new_local_storage(
                    "https://app.example.com",
                    [
                        ("auth_token".to_string(), "jwt.eyJ.signature".to_string()),
                        ("theme".to_string(), "dark".to_string()),
                        // 特殊字符 / 空值（往返必须原样保真，不被吞）。
                        ("note".to_string(), "a=b&c={\"x\":1}".to_string()),
                        ("empty".to_string(), String::new()),
                    ],
                ),
                OriginStorage::new_local_storage(
                    "http://localhost:8080",
                    [("dev_flag".to_string(), "on".to_string())],
                ),
            ],
        }
    }

    #[test]
    fn local_storage_json_round_trip_preserves_origin_and_items() {
        // **核心：localStorage 往返保真（origin + 键值）**——StorageState → JSON → StorageState 不丢。
        let original = local_storage_state();
        let json = original.to_json().expect("to_json");
        let back = StorageState::from_json(json).expect("from_json");
        assert_eq!(original, back, "localStorage must round-trip through JSON without loss");
        // 显式钉死：origin 绑定 + 键值（含特殊字符/空值）原样。
        let app = back
            .local_storage
            .iter()
            .find(|o| o.origin == "https://app.example.com")
            .expect("app origin survives round-trip");
        let find = |k: &str| app.local_storage.iter().find(|i| i.name == k).map(|i| i.value.as_str());
        assert_eq!(find("auth_token"), Some("jwt.eyJ.signature"));
        assert_eq!(find("note"), Some("a=b&c={\"x\":1}"), "special chars must survive");
        assert_eq!(find("empty"), Some(""), "empty value must survive (not dropped)");
        // 第二个 origin（含非默认端口）不被混入第一个 origin（origin-bound 分区）。
        let local = back
            .local_storage
            .iter()
            .find(|o| o.origin == "http://localhost:8080")
            .expect("second origin survives");
        assert_eq!(local.local_storage.len(), 1, "origins must stay partitioned");
    }

    #[test]
    fn local_storage_json_field_names_are_storage_state_compatible() {
        // 序列化字段名 `localStorage`/`origin`/`name`/`value`（与 Playwright storage_state JSON 习惯一致）。
        let s = serde_json::to_string(&local_storage_state()).expect("serialize");
        assert!(s.contains("\"localStorage\""), "expected localStorage key: {s}");
        assert!(s.contains("\"origin\""), "expected origin key: {s}");
        assert!(s.contains("\"name\""), "expected localStorage item name key: {s}");
        assert!(s.contains("\"value\""), "expected localStorage item value key: {s}");
        // IndexedDB best-effort = None → skip_serializing_if 不出现在 JSON（不污染、向后兼容）。
        assert!(!s.contains("\"indexDb\""), "indexDb None must be omitted: {s}");
    }

    #[test]
    fn missing_local_storage_key_is_backward_compatible_with_w4b() {
        // **向后兼容铁律**：W4b 已存的 cookie-only storage_state（无 `localStorage` 键）反序列化为
        // 空 local_storage（#[serde(default)]）——不破坏旧 vault 状态。
        let w4b_json = serde_json::json!({ "cookies": [] });
        let back = StorageState::from_json(w4b_json).expect("W4b cookie-only state still parses");
        assert!(
            back.local_storage.is_empty(),
            "missing localStorage key must default to empty (W4b backward compat)"
        );
        // 反向：纯 localStorage（无 cookies 键）也解析（两半正交，各自 #[serde(default)]）。
        let ls_only = serde_json::json!({
            "localStorage": [{ "origin": "https://x.test", "localStorage": [] }]
        });
        let back2 = StorageState::from_json(ls_only).expect("localStorage-only state parses");
        assert!(back2.cookies.is_empty());
        assert_eq!(back2.local_storage.len(), 1);
        assert_eq!(back2.local_storage[0].origin, "https://x.test");
    }

    #[test]
    fn origin_storage_new_local_storage_builder_maps_pairs_in_order() {
        // 便捷构造器：origin + (k,v) 对 → OriginStorage（顺序保持，IndexedDB None）。
        let o = OriginStorage::new_local_storage(
            "https://example.com",
            [("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())],
        );
        assert_eq!(o.origin, "https://example.com");
        assert_eq!(o.local_storage.len(), 2);
        assert_eq!(o.local_storage[0], LocalStorageItem { name: "a".into(), value: "1".into() });
        assert_eq!(o.local_storage[1], LocalStorageItem { name: "b".into(), value: "2".into() });
        assert!(o.index_db.is_none(), "new_local_storage builder leaves IndexedDB as None");
    }

    // ── IndexedDB dump 序列化/反序列化 + base64 二进制往返 ─────────────────────────

    #[test]
    fn indexeddb_dump_roundtrips_binary() {
        // 核心：IndexedDbDump 含 base64 二进制哨兵的 record，经 JSON 序列化/反序列化往返保真。
        use super::{
            IdbDatabase, IdbStore, IndexedDbDump, encode_binary_sentinel, decode_binary_sentinel,
            IDB_BASE64_SENTINEL,
        };

        let binary_data: Vec<u8> = vec![0x00, 0x01, 0xFF, 0xFE, 0x42, 0x43];
        let sentinel = encode_binary_sentinel(&binary_data);

        let dump = IndexedDbDump {
            databases: vec![IdbDatabase {
                name: "mydb".into(),
                version: 3,
                stores: vec![IdbStore {
                    name: "objects".into(),
                    key_path: Some("id".into()),
                    auto_increment: false,
                    records: vec![
                        // 普通 JSON 记录
                        serde_json::json!({"id": 1, "name": "hello"}),
                        // 含 base64 二进制哨兵的记录
                        serde_json::json!({"id": 2, "payload": sentinel}),
                    ],
                }],
            }],
        };

        // 序列化→反序列化往返保真。
        let json_str = serde_json::to_string(&dump).expect("serialize IndexedDbDump");
        let back: IndexedDbDump = serde_json::from_str(&json_str).expect("deserialize IndexedDbDump");
        assert_eq!(dump, back, "IndexedDbDump must round-trip through JSON without loss");

        // base64 哨兵验证通过。
        assert!(dump.validate_base64_sentinels(), "all base64 sentinels must be valid");

        // 解码 sentinel 还原出原始字节。
        let record2 = &back.databases[0].stores[0].records[1];
        let payload = record2.get("payload").expect("payload field");
        let decoded = decode_binary_sentinel(payload).expect("decode base64 sentinel");
        assert_eq!(decoded, binary_data, "binary must round-trip through base64 sentinel");

        // 验证 sentinel 格式正确（单键 __b64__）。
        assert!(payload.is_object());
        let map = payload.as_object().unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(IDB_BASE64_SENTINEL));
    }

    #[test]
    fn indexeddb_dump_empty_databases_serialize_clean() {
        // 空 IndexedDbDump（无数据库）序列化/反序列化正确。
        use super::IndexedDbDump;

        let dump = IndexedDbDump::default();
        let json = serde_json::to_string(&dump).expect("serialize empty dump");
        assert!(json.contains("\"databases\":[]"), "empty databases array: {json}");
        let back: IndexedDbDump = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dump, back);
    }

    #[test]
    fn indexeddb_dump_out_of_line_key_store() {
        // out-of-line key（key_path=None, autoIncrement=true）的 store 正确往返。
        use super::{IdbDatabase, IdbStore, IndexedDbDump};

        let dump = IndexedDbDump {
            databases: vec![IdbDatabase {
                name: "ool_db".into(),
                version: 1,
                stores: vec![IdbStore {
                    name: "queue".into(),
                    key_path: None,
                    auto_increment: true,
                    records: vec![serde_json::json!("item1"), serde_json::json!(42)],
                }],
            }],
        };
        let json = serde_json::to_string(&dump).expect("serialize");
        // key_path=None 不出现在 JSON（skip_serializing_if）。
        assert!(!json.contains("\"keyPath\""), "null keyPath must be omitted: {json}");
        let back: IndexedDbDump = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dump, back);
        assert!(back.databases[0].stores[0].key_path.is_none());
        assert!(back.databases[0].stores[0].auto_increment);
    }

    #[test]
    fn origin_storage_with_indexeddb_json_round_trip() {
        // OriginStorage 含 IndexedDB dump 时的完整往返（localStorage + IndexedDB 共存）。
        use super::{IdbDatabase, IdbStore, IndexedDbDump, encode_binary_sentinel};

        let origin_store = OriginStorage {
            origin: "https://app.example.com".into(),
            local_storage: vec![LocalStorageItem {
                name: "token".into(),
                value: "abc".into(),
            }],
            index_db: Some(IndexedDbDump {
                databases: vec![IdbDatabase {
                    name: "appdb".into(),
                    version: 2,
                    stores: vec![IdbStore {
                        name: "cache".into(),
                        key_path: Some("url".into()),
                        auto_increment: false,
                        records: vec![
                            serde_json::json!({"url": "/api/data", "body": "cached"}),
                            serde_json::json!({"url": "/api/img", "body": encode_binary_sentinel(&[0xCA, 0xFE])}),
                        ],
                    }],
                }],
            }),
        };

        let state = StorageState {
            cookies: vec![],
            local_storage: vec![origin_store.clone()],
        };
        let json = state.to_json().expect("to_json");
        let back = StorageState::from_json(json).expect("from_json");
        assert_eq!(state, back, "StorageState with IndexedDB must round-trip");

        // 确认 IndexedDB 在 JSON 里出现（不再 skip）。
        let s = serde_json::to_string(&state).expect("serialize");
        assert!(s.contains("\"indexDb\""), "indexDb must appear when Some: {s}");
        assert!(s.contains("\"appdb\""), "db name must appear: {s}");
    }
}
