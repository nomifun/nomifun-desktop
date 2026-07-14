//! **E5：出口数据流防火墙**（`Fetch.enable` 全流量拦截 + IP 封禁 + 跨域 POST-body 门控）。
//!
//! DESIGN §16「出口数据流防火墙」/「跨域 POST-body 门控」+ P2 裁决⑪。三条职责：
//!
//! 1. **`Fetch.enable` 全流量拦截**（接线在 [`crate::backend::cdp`]）：经
//!    `Target.setAutoAttach{flatten}` 收编的**每个** session（根 browser / page / OOPIF /
//!    **service_worker**）都挂 `Fetch.enable`。**SW 必须也拦**（裁决⑪/不变量⑬）——否则页面把出口
//!    请求塞进 service worker 即可整体绕过防火墙。P0 已保证「不 detach SW、保持 attach」（见
//!    `transport.rs` `handle_attached` 的 spike 坑），E5 在其上对 SW session 也 `Fetch.enable`。
//!    每条被拦的请求经 `Fetch.requestPaused` 抵达，由 [`decide`] 判定后 `Fetch.continueRequest`
//!    放行或 `Fetch.failRequest{BlockedByClient}` 阻断。
//!
//! 2. **IP 封禁（纯逻辑，本模块重点）**：[`is_blocked_ip`] 封 RFC1918 私网 / loopback /
//!    link-local（**含 `169.254.169.254` 云元数据端点**——经典 SSRF 支点）/ CGNAT / IPv6 ULA /
//!    其它非公网，**公网放行**。全用 std `IpAddr`/`Ipv4Addr`/`Ipv6Addr` 的方法（`is_private` /
//!    `is_loopback` / `is_link_local` …），不手搓位运算（IPv6 ULA/link-local 例外——std 无
//!    `is_unique_local`/`is_unicast_link_local` 的 stable 版，按本仓 `nomifun-knowledge` SSRF
//!    范式用最小位掩码）。这是**硬封禁**：命中即 `failRequest`，无需审批（SSRF 防护不存在「批准
//!    访问云元数据」的合法场景）。
//!
//! 3. **跨域 POST-body 门控**：拦截含 body 的跨域 POST（submit / upload，目标 host 与当前页
//!    origin 不同 eTLD+1）→ 升 **Exec 审批**。审批预览经 [`build_post_preview`] 构造，**只**含
//!    目标 host + body 大小 + 字段名（form field names）——**绝不**含字段值（值可能携带
//!    secret / 敏感数据；安全红线）。**E5 只提供拦截 + 门控判定 + 预览构造**；实际审批路由（接
//!    Exec tier approval pipeline）由 **F1** 接线——见 `decide` 里的 `TODO(E5->F1-egress-approval)`。
//!
//! **跨域判定**用 eTLD+1（[`is_cross_origin`]）：复用 `nomifun-secret` 的 PSL 机器
//! （`same_etld_plus_one`），对 IP / `localhost` 等无 eTLD+1 的 host 退化为裸 host 比较——故
//! 「同一 IP / 同一 localhost 间的 POST」**不**误判为跨域。
//!
//! 不变量（勿破坏）：
//! - **SW 保持 attach 并对其 `Fetch.enable`**（不变量⑬）。
//! - **`BrowserConfig.allowed_origins` 是死字段**（不变量⑭）——本模块**绝不**复用它，防火墙有独立
//!   [`FirewallConfig`]。
//! - **预览绝不含字段值**（[`build_post_preview`] 单测断言）。

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 出口防火墙配置（**独立**于死字段 `BrowserConfig.allowed_origins`，见模块 doc / 不变量⑭）。
///
/// `Default` = 启用 IP 封禁（无副作用的纯 SSRF 防护，恒应开）+ 启用跨域 POST 门控**检测**（E5 只
/// 检测 + 构造预览；F1 接审批路由）+ **空域名策略**（`allow_etld1`/`deny_etld1` 均空 = 不限制出口
/// 域，现行为，零回归）。三挡分开是因为：IP 封禁是「硬封禁」（命中即拒，无审批语义）、跨域 POST 门控
/// 是「升审批」（F1 才有放行/拒绝的人在回路）、域名 allowlist 是「出口域策略」（D1）。
///
/// **P3-D1（裁决⑤）**：加 `allow_etld1`/`deny_etld1` 两个 eTLD+1 域名策略字段（复用 `nomifun_secret`
/// 同一 PSL 机器解析目标域）。**数据源 = secret 的 per-pet `allowed_origins`**（与 secret 域**共用同一份
/// 真值**）。D1 建机制（让 `FirewallConfig` 能携带域名策略 + `decide` 强制）；**P3-X2 已接真值**——
/// `BrowserTool::ensure_secret_store_and_firewall`（`nomi-browser/tool.rs`）从 per-pet vault 加载的
/// `SecretStore::allowed_etld1_union()` 灌进 `allow_etld1`，经 `EngineConfig.firewall` 注入（不再恒
/// `default()`；空 secret store → 空 allowlist = 不限制出口域，零回归）。
///
/// **由 D1 加 `Vec` 字段，`FirewallConfig` 不再 `Copy`（改 `Clone`）**——同步 G1 链路（`cdp.rs` 的
/// `firewall_config` 快照 / `spawn_fetch_firewall_loop` move 传入）从 Copy 用法改 Clone。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirewallConfig {
    /// 封禁解析到内网 / loopback / link-local（含云元数据）/ 其它非公网 IP 的出口请求。
    /// 默认 `true`（SSRF 防护，恒应开）。
    pub block_private_ips: bool,
    /// 对含 body 的跨域 POST 做门控**检测**（构造审批预览）。默认 `true`。实际审批 enforcement
    /// 接线 F1（E5 仅检测 + 预览）。
    pub gate_cross_origin_post: bool,
    /// **D1 域名 allowlist（eTLD+1）**：出口域**白名单**。**空 = 不限制**（任何域放行，现行为/零回归）；
    /// **非空 = 仅放行 eTLD+1 ∈ 本表的目标域**，其余域升 [`FirewallDecision::GatePost`]（交 D2 审批）。
    /// 条目应是 eTLD+1（`x.com`），但调用方传任意 host/origin 也安全——内部用 [`nomifun_secret::etld_plus_one`]
    /// 归一后比较（与 secret `register` 的 `allowed_origins`→eTLD+1 归一同款 PSL 机器）。**真值来自 secret 的
    /// per-pet `allowed_origins`，由 `BrowserTool::ensure_secret_store_and_firewall` 注入（P3-X2）**。无法解析出
    /// eTLD+1 的目标域（IP/localhost/畸形）在 allowlist 非空时**保守门控**（fail-closed：无 registrable domain
    /// 无从证明在白名单内）。
    pub allow_etld1: Vec<String>,
    /// **D1 域名 denylist（eTLD+1）**：出口域**黑名单**，**优先级高于 allowlist**。eTLD+1 命中 → 硬
    /// [`FirewallDecision::Block`]（即便同时在 allowlist 内也阻断）。空 = 无黑名单。条目按 eTLD+1 归一
    /// （同 [`Self::allow_etld1`]）。**secret 配置只携带 allowlist（`allowed_origins`），无 denylist 概念**，
    /// 故 X2 注入恒留空；本字段为机制预留（未来若加显式封禁名单可经此灌入），当前不暴露 UI。
    pub deny_etld1: Vec<String>,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            block_private_ips: true,
            gate_cross_origin_post: true,
            // D1：默认空域名策略 = 不限制出口域（现行为，零回归）。真值由 X2 从 secret 灌入。
            allow_etld1: Vec::new(),
            deny_etld1: Vec::new(),
        }
    }
}

/// 防火墙对一条被拦请求的裁决。映射到 CDP：[`Self::Allow`]→`Fetch.continueRequest`，
/// [`Self::Block`]→`Fetch.failRequest{BlockedByClient}`，[`Self::GatePost`]→**F1** 升 Exec
/// 审批（E5 阶段先放行 + 构造预览留痕，见 `decide` 的 TODO）。
#[derive(Clone, Debug, PartialEq)]
pub enum FirewallDecision {
    /// 放行（`Fetch.continueRequest`）。
    Allow,
    /// 硬阻断（`Fetch.failRequest{BlockedByClient}`）。`reason` 供审计文案。
    Block { reason: String },
    /// 跨域 POST-body 命中门控：携带审批预览（[`PostPreview`]）。E5 构造预览；F1 接审批路由。
    GatePost { preview: PostPreview },
}

/// 跨域 POST-body 审批预览（裁决⑪/DESIGN §16）。**安全红线：只含可安全展示给审批者的元数据，
/// 绝不含任何字段值**——值可能携带 secret / 密码 / 敏感 PII。
///
/// serde 用于跨 F1 审批通道传输（预览展示给人）。`field_names` 是表单字段名（如 `username` /
/// `card_number`），**不含**对应的值。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostPreview {
    /// 目标 host（请求要发往的主机，如 `evil.example.com`）。
    pub host: String,
    /// 请求 body 的字节大小。
    pub size: usize,
    /// 表单字段名列表（**仅名，绝不含值**）。非表单 body（JSON / 二进制 / 不可解析）→ 空 vec。
    pub field_names: Vec<String>,
}

/// **IP 封禁判定（纯逻辑，本模块重点）**：`true` = 该 IP 属于内网 / loopback / link-local（含
/// 云元数据 `169.254.169.254`）/ CGNAT / IPv6 ULA / link-local / 其它非公网 → 应封禁。`false` =
/// 公网 IP → 放行。
///
/// 用 std `IpAddr`/`Ipv4Addr`/`Ipv6Addr` 的方法（`is_private` 覆盖 10/172.16-31/192.168，
/// `is_loopback` 覆盖 127.0.0.0/8 与 `::1`，`is_link_local` 覆盖 `169.254.0.0/16` **含
/// 169.254.169.254 元数据**，`is_unspecified` 覆盖 `0.0.0.0`/`::`，`is_multicast`/`is_broadcast`/
/// `is_documentation`）。CGNAT `100.64.0.0/10`、IPv6 ULA `fc00::/7`、IPv6 link-local `fe80::/10`
/// 无 stable std 谓词，按本仓 `nomifun-knowledge::source_url::forbidden_ip` 同款最小位掩码补。
/// IPv4-mapped IPv6（`::ffff:a.b.c.d`）继承其 v4 裁决（防绕过）。
///
/// **这是硬封禁**（无审批语义）：访问云元数据 / 内网不存在合法的「批准」场景——直接 `failRequest`。
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

/// IPv4 封禁谓词（见 [`is_blocked_ip`]）。
fn is_blocked_ipv4(v4: Ipv4Addr) -> bool {
    let octets = v4.octets();
    v4.is_loopback()           // 127.0.0.0/8
        || v4.is_private()      // 10/8, 172.16/12, 192.168/16（RFC1918）
        || v4.is_link_local()   // 169.254.0.0/16（含 169.254.169.254 云元数据）
        || v4.is_unspecified()  // 0.0.0.0
        || v4.is_broadcast()    // 255.255.255.255
        || v4.is_multicast()    // 224.0.0.0/4
        || v4.is_documentation()// 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
        || octets[0] == 0       // 0.0.0.0/8「本网络」
        // CGNAT 100.64.0.0/10（运营商级 NAT，常落内网拓扑）。
        || (octets[0] == 100 && (64..128).contains(&octets[1]))
        // IETF 协议分配 192.0.0.0/24（含 192.0.0.x 特殊用途）。
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
}

/// IPv6 封禁谓词（见 [`is_blocked_ip`]）。IPv4-mapped 继承 v4 裁决。
fn is_blocked_ipv6(v6: Ipv6Addr) -> bool {
    let seg0 = v6.segments()[0];
    v6.is_loopback()            // ::1
        || v6.is_unspecified()  // ::
        || v6.is_multicast()    // ff00::/8
        // Unique-local（ULA）fc00::/7（fc00:: 与 fd00::）。
        || (seg0 & 0xfe00) == 0xfc00
        // Link-local fe80::/10。
        || (seg0 & 0xffc0) == 0xfe80
        // IPv4-mapped（::ffff:a.b.c.d）/ 兼容地址继承其 v4 裁决，防经 v6 字面量绕过。
        || v6.to_ipv4_mapped().is_some_and(is_blocked_ipv4)
}

/// **跨域判定**（裁决⑪）：`true` = 请求目标 host 与当前页 origin **不同 eTLD+1**（跨域）。
///
/// 策略：
/// - 两侧都能导出 eTLD+1（普通域名）→ 比较 eTLD+1（`same_etld_plus_one`）。`sub.x.com` POST 到
///   `api.x.com` = 同 eTLD+1 = **非**跨域；POST 到 `evil.com` = 跨 eTLD+1 = 跨域。
/// - 至少一侧无 eTLD+1（IP / `localhost`，PSL 不给 registrable domain）→ 退化为**裸 host 比较**
///   （`host_of` 归一化后小写比较）。这样「同一 IP / 同一 localhost 间的 POST」不会被
///   `same_etld_plus_one` 的 fail-closed（任一侧无 eTLD+1 即返 false）误判为跨域。
///
/// 无法导出任何一侧的 host（畸形 URL）→ **保守判跨域**（`true`，fail-closed：宁可多门控一次也不
/// 漏一个出口）。
pub fn is_cross_origin(current_origin: &str, target_url: &str) -> bool {
    use nomifun_secret::{etld_plus_one, host_of};

    let cur_host = host_of(current_origin);
    let tgt_host = host_of(target_url);
    let (Some(cur_host), Some(tgt_host)) = (cur_host, tgt_host) else {
        // 任一侧导不出 host（畸形）→ 保守判跨域（fail-closed）。
        return true;
    };

    match (etld_plus_one(&cur_host), etld_plus_one(&tgt_host)) {
        // 两侧都有 eTLD+1（普通域名）→ 比较 registrable domain。
        (Some(a), Some(b)) => a != b,
        // 至少一侧无 eTLD+1（IP / localhost）→ 退化为裸 host 比较（同 host = 非跨域）。
        _ => cur_host != tgt_host,
    }
}

/// HTTP 方法是否「有 body 的写」——POST（也含 PUT/PATCH/DELETE 等带 body 的写方法，
/// 门控语义同 POST：含 body 的跨域写都该升审批）。GET/HEAD/OPTIONS 等无 body 的不门控。
fn is_body_write_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    )
}

/// **跨域 POST-body 门控判定（纯逻辑）**：`true` = 这是一个「有 body 的跨域写」请求，应升审批。
///
/// 三条件**全**满足才门控：
/// 1. 方法是有 body 的写（POST/PUT/PATCH/DELETE，见 [`is_body_write_method`]）；
/// 2. 确有 body（`has_post_data` 为真 **或** body 字节非空）；
/// 3. 目标 host 与当前页 origin 跨域（[`is_cross_origin`]）。
///
/// `body` 是已解出的请求 body 字节（调用方从 `Fetch.requestPaused` / `getRequestPostData` 取得；
/// 可能为 `None` = 拿不到 body 内容但 `has_post_data` 指示有）。
pub fn is_gated_post(
    method: &str,
    has_post_data: bool,
    body: Option<&[u8]>,
    current_origin: &str,
    target_url: &str,
) -> bool {
    if !is_body_write_method(method) {
        return false;
    }
    let has_body = has_post_data || body.is_some_and(|b| !b.is_empty());
    if !has_body {
        return false;
    }
    is_cross_origin(current_origin, target_url)
}

/// **审批预览构造（纯逻辑，安全红线：绝不含字段值）**。
///
/// 从目标 URL 取 host、从 body 取大小与（若是表单编码）字段名。`field_names` **只**含名——
/// [`parse_form_field_names`] 解析 `application/x-www-form-urlencoded` 形态时**丢弃**每个
/// `key=value` 的 value 部分。非表单 body（JSON / multipart / 二进制 / 拿不到）→ `field_names`
/// 为空 vec（仍给 host + size，够审批者判断「往哪发了多大的东西」）。
///
/// `content_type`：请求的 `Content-Type` 头（决定是否按 urlencoded 表单解析字段名）。`None` →
/// 不尝试解析字段名（保守，避免把 JSON body 的 key 当字段名泄露——虽然只是名，仍从严）。
pub fn build_post_preview(
    target_url: &str,
    body: Option<&[u8]>,
    content_type: Option<&str>,
) -> PostPreview {
    let host = nomifun_secret::host_of(target_url).unwrap_or_default();
    let size = body.map(|b| b.len()).unwrap_or(0);
    let field_names = match (body, content_type) {
        (Some(b), Some(ct)) if is_form_urlencoded(ct) => parse_form_field_names(b),
        _ => Vec::new(),
    };
    PostPreview {
        host,
        size,
        field_names,
    }
}

/// `Content-Type` 是否 `application/x-www-form-urlencoded`（HTML form 默认 POST 编码）。
fn is_form_urlencoded(content_type: &str) -> bool {
    content_type
        .to_ascii_lowercase()
        .contains("application/x-www-form-urlencoded")
}

/// **从 urlencoded 表单 body 解析字段名（只名，绝不含值）**。
///
/// `username=alice&password=hunter2&csrf=abc` → `["username", "password", "csrf"]`。每个
/// `key=value` 对**只取 `key`、丢弃 `value`**（安全红线：值可能是密码 / secret）。key 经
/// percent-decode（best-effort：解失败原样保留——字段名通常 ASCII，无需完美解码）。无 `=` 的裸
/// token（`flag&...`）按字段名（值为空）记入 key。去重保序。
pub fn parse_form_field_names(body: &[u8]) -> Vec<String> {
    let s = String::from_utf8_lossy(body);
    let mut names: Vec<String> = Vec::new();
    for pair in s.split('&') {
        if pair.is_empty() {
            continue;
        }
        // 只取 '=' 左侧的 key；右侧（value）**丢弃**——绝不进预览。
        let key_raw = pair.split('=').next().unwrap_or("");
        if key_raw.is_empty() {
            continue;
        }
        let key = percent_decode_key(key_raw);
        if key.is_empty() {
            continue;
        }
        if !names.contains(&key) {
            names.push(key);
        }
    }
    names
}

/// best-effort percent-decode 表单字段名（`%20`→空格，`+`→空格）。字段名几乎恒 ASCII；解码失败
/// 段原样保留（不为字段名解码引入新依赖，标准库逐字节解析足够）。
fn percent_decode_key(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 一条被拦请求的判定输入（[`decide`] 的参数包）。把请求侧字段聚成结构体，让 [`decide`] 的签名稳定
/// （新增判定维度只改本结构体，不破坏调用方），也避免一长串位置参数易错位。
#[derive(Clone, Debug)]
pub struct RequestInfo<'a> {
    /// 目标 host 解析出的 IP（仅当 host 本身是 IP 字面量同步可得；域名未解析 → `None`，见
    /// [`ip_literal_of_host`]）。
    pub resolved_ip: Option<IpAddr>,
    /// HTTP 方法（`POST`/`GET`/…）。
    pub method: &'a str,
    /// CDP `request.hasPostData`（指示有 body，即使 body 内容此刻不可见）。
    pub has_post_data: bool,
    /// 已解出的请求 body 字节（拿不到内容 → `None`）。
    pub body: Option<&'a [u8]>,
    /// 请求 `Content-Type` 头（决定是否按 urlencoded 表单解析字段名）。
    pub content_type: Option<&'a str>,
    /// 发起请求的文档 origin（跨域判定左侧；通常取请求 `Origin`/`Referer` 头）。
    pub current_origin: &'a str,
    /// 请求目标 URL（跨域判定右侧 + 预览 host 来源）。
    pub target_url: &'a str,
    /// **是否顶层 Document 导航**（CDP `Fetch.requestPaused` 的 `resourceType == Document`）。
    ///
    /// **安全设计（P3 回归修）**：域名 allowlist（[`FirewallConfig::allow_etld1`]）是**出口/数据外泄
    /// 控制**（限制 cross-origin 子请求/POST 把数据发去哪），**不是导航监狱**——agent 主动导航到一个 URL
    /// 是意图行为，不该被「注册了 secret → allowlist 非空」反手关进白名单域。故 [`domain_policy`] 的
    /// **allowlist 档对顶层 Document 导航豁免**（`true` → 不受 allow_etld1 白名单约束）。
    ///
    /// **仅豁免 allowlist 白名单**：IP 封禁（SSRF/元数据，[`is_blocked_ip`]）对顶层导航**仍生效**（导航到
    /// 内网/元数据 IP 必拦）；`deny_etld1` 黑名单对顶层导航**仍硬 Block**（显式封禁名单拦导航合理）；跨域
    /// POST-body 门控不受影响（导航不是 POST-body 写）。
    ///
    /// 接线层（`cdp.rs::handle_paused_request`）由 `paused.resource_type == ResourceType::Document` 置位。
    /// 拿不准 / 非 Document → `false`（保守：子资源请求仍受 allowlist 出口门控）。
    pub is_top_level_navigation: bool,
}

/// **D1 域名策略裁决（纯逻辑）**：据 `allow_etld1`/`deny_etld1` 对目标域给出域名档裁决。
///
/// 返回：
/// - `Some(Block)` —— 目标 eTLD+1 命中 `deny_etld1`（**deny 优先级最高**，即便也在 allowlist 内；
///   **含顶层 Document 导航**——黑名单硬拦导航合理）。
/// - `Some(GatePost{preview})` —— `allow_etld1` **非空** 且目标 eTLD+1 **不在** allowlist（出口到未授权域，
///   交 D2 审批；同时也覆盖目标域**无法解析出 eTLD+1**（IP/localhost/畸形）这一 fail-closed 情形——
///   allowlist 非空时无 registrable domain 无从证明在白名单内，保守门控）。**顶层 Document 导航豁免本档**
///   （`req.is_top_level_navigation` → allowlist 不门控导航；见下）。
/// - `None` —— 域名档不触发（无 deny 命中；且 allowlist 空=不限制，或目标域∈allowlist，或**是顶层
///   Document 导航**）→ 交由后续档（跨域 POST 门控 / 放行）处理。
///
/// 复用 [`nomifun_secret::etld_plus_one`]（同一 PSL 机器，co.uk 等多级后缀正确）解析目标域。`deny`/`allow`
/// 条目同样经 `etld_plus_one` 归一后比较——故调用方传 `x.com` / `https://x.com:443` / `sub.x.com` 都安全
/// （都归一到同一 registrable domain）。
///
/// **GatePost 而非 Block**（裁决⑤/D2）：allowlist 外的域**升审批**（让 D2 的人在回路决定放行/拒），
/// 与跨域 POST 门控复用同一 [`PostPreview`] 通道；唯 `deny` 命中是**硬 Block**（黑名单无审批语义）。
fn domain_policy(config: &FirewallConfig, req: &RequestInfo<'_>) -> Option<FirewallDecision> {
    use nomifun_secret::etld_plus_one;

    // 域名策略两表都空 = 不限制出口域（现行为/零回归）→ 不触发域名档。
    if config.allow_etld1.is_empty() && config.deny_etld1.is_empty() {
        return None;
    }

    // 目标域的 eTLD+1（IP/localhost/畸形 → None）。allow/deny 条目同样归一比较。
    let target_e1 = etld_plus_one(req.target_url);

    // 1) deny 优先（黑名单硬 Block，即便也在 allowlist）。仅当目标域可解析出 eTLD+1 才比较——
    //    无 eTLD+1 的目标（IP/localhost/畸形）不可能匹配 deny 条目（条目本身经 etld_plus_one 归一）。
    if let Some(ref e1) = target_e1 {
        let denied = config
            .deny_etld1
            .iter()
            .filter_map(|d| etld_plus_one(d))
            .any(|d| &d == e1);
        if denied {
            return Some(FirewallDecision::Block {
                reason: format!("blocked egress to denylisted domain {e1}: {}", req.target_url),
            });
        }
    }

    // 2) allowlist（非空 = 仅放行表内域）。空 allowlist 不限制（deny 已在上面单独处理）。
    //
    //    **顶层 Document 导航豁免（P3 回归修，安全设计）**：allowlist 是**出口/数据外泄控制**（限制跨域
    //    子请求/POST 把数据发往哪），**不是导航监狱**——agent 导航到一个 URL 是意图行为，不该因「注册了
    //    secret → allowlist 非空」被反手关进白名单域（含连 file:// fixture 都导不进，defeat 浏览器用途）。
    //    故顶层 Document 导航**跳过 allowlist 白名单门控**。注意：①IP 封禁（SSRF/元数据）已在 `decide`
    //    最高优先档对**所有**请求（含 Document）生效；②`deny_etld1` 黑名单已在上面 `1)` 对**所有**请求
    //    （含 Document）硬 Block；③跨域 POST-body 门控走 `decide` 的独立档（导航不是 POST-body 写）。
    //    本豁免**只**松绑 allow_etld1 白名单对顶层导航的拦截，不触碰任何其它出口/SSRF/黑名单拦截。
    if !config.allow_etld1.is_empty() && !req.is_top_level_navigation {
        let allowed = match target_e1 {
            // 目标域可解析 → 必须 eTLD+1 ∈ allowlist 才放行。
            Some(ref e1) => config
                .allow_etld1
                .iter()
                .filter_map(|a| etld_plus_one(a))
                .any(|a| &a == e1),
            // 目标域无 eTLD+1（IP/localhost/畸形）→ fail-closed：allowlist 非空时无从证明在白名单内，门控。
            None => false,
        };
        if !allowed {
            // 出口到未授权域 → 升审批（D2 接人在回路）。复用 POST 预览通道做审批载体
            // （host/size/field_names；非 POST 时 size=0/field_names 空——仍给目标 host 供审批者判断）。
            return Some(FirewallDecision::GatePost {
                preview: build_post_preview(req.target_url, req.body, req.content_type),
            });
        }
    }

    None
}

/// **防火墙裁决（纯逻辑编排）**：综合 [`FirewallConfig`] 与 [`RequestInfo`]，给出 [`FirewallDecision`]。
/// 接线层（[`crate::backend::cdp`] 的 requestPaused 循环）据此发 `Fetch.continueRequest` /
/// `failRequest` / （F1）升审批。
///
/// 优先级（从严）：
/// 1. **IP 封禁**最高优先（硬 SSRF 防护）：`block_private_ips` 开 且 `resolved_ip` 命中
///    [`is_blocked_ip`] → [`FirewallDecision::Block`]。
/// 2. **域名档（D1，[`domain_policy`]）**：`deny_etld1` 命中 → 硬 Block（黑名单，优先 allow，**含顶层
///    导航**）；`allow_etld1` 非空且目标 eTLD+1 ∉ allowlist → GatePost（出口到未授权域，交 D2 审批）。
///    **allowlist 出口门控豁免顶层 Document 导航**（`is_top_level_navigation`——allowlist 是出口/数据外泄
///    控制，不是导航监狱；agent 导航是意图行为，注册 secret 后仍可自由导航）；子资源请求（XHR/Fetch/...）
///    仍受 allowlist 出口门控。空 allowlist = 不限制（现行为/零回归）。
/// 3. **跨域 POST 门控**：`gate_cross_origin_post` 开 且 [`is_gated_post`] 命中 →
///    [`FirewallDecision::GatePost`]（构造预览）。**与域名档叠加**——域名档放行（域在白名单/不限制）
///    后，跨域 POST-body 仍单独门控。
/// 4. 否则 [`FirewallDecision::Allow`]。
///
/// `RequestInfo::resolved_ip`：把目标 host 解析成 IP 后的结果（`None` = 还没解析 / 是域名未解析——
/// 此时**不**做 IP 封禁判定，靠接线层对 IP 字面量同步判；域名解析的异步路径留 `TODO`）。
///
/// `TODO(E5->F1-egress-approval)`：[`FirewallDecision::GatePost`] 当前由接线层**放行 + 构造预览
/// 留痕**（E5 范围：检测 + 预览）；F1 把它接到 Exec tier approval pipeline 的人在回路审批（批准才
/// `continueRequest`，否则 `failRequest`）。
pub fn decide(config: &FirewallConfig, req: &RequestInfo<'_>) -> FirewallDecision {
    // 1) IP 封禁（最高优先，硬 SSRF 防护）。
    if config.block_private_ips
        && let Some(ip) = req.resolved_ip
        && is_blocked_ip(ip)
    {
        return FirewallDecision::Block {
            reason: format!(
                "blocked egress to non-public IP {ip} (SSRF/metadata guard): {}",
                req.target_url
            ),
        };
    }

    // 2) 域名档（D1，裁决⑤）：deny 硬 Block（优先 allow）；allow 非空且目标域 ∉ allowlist → GatePost
    //    （出口到未授权域，交 D2 审批）。**所有请求都过此档**（allowlist 是导航/资源出口策略，不止 POST）。
    //    空策略（两表皆空）= 不限制 = 现行为/零回归。
    //
    //    X2 已接真值：`allow_etld1` 来自 secret 的 per-pet `allowed_origins`（裁决⑤，与 secret 域共用同一份
    //    配置）——由 `BrowserTool::ensure_secret_store_and_firewall` 从 per-pet vault 加载后经
    //    `EngineConfig.firewall` 注入。`deny_etld1` 为机制预留（secret 配置无 denylist 概念，恒空）。
    if let Some(domain_decision) = domain_policy(config, req) {
        return domain_decision;
    }

    // 3) 跨域 POST-body 门控（升审批；E5 检测 + 构造预览，F1 接路由）。与域名档叠加——域名档放行后，
    //    跨域 POST-body 仍单独门控。
    if config.gate_cross_origin_post
        && is_gated_post(
            req.method,
            req.has_post_data,
            req.body,
            req.current_origin,
            req.target_url,
        )
    {
        return FirewallDecision::GatePost {
            preview: build_post_preview(req.target_url, req.body, req.content_type),
        };
    }

    FirewallDecision::Allow
}

/// **P3-D2：被门控（[`FirewallDecision::GatePost`]）请求的最终裁决**——审批通道给出的放行 / 拒绝。
///
/// 这是接线层（[`crate::backend::cdp::handle_paused_request`]）在**悬挂**一条被门控请求（保留
/// `requestId`、**不**立即 continue/fail）后，经 [`EgressApprover`] 取得的人在回路结果：
/// - [`Self::Continue`] → `Fetch.continueRequest`（批准放行一次）。
/// - [`Self::ContinueAndRemember`] → `Fetch.continueRequest` + 记住此域（决策3 always_allow）。
/// - [`Self::Fail`] → `Fetch.failRequest{BlockedByClient}`（拒绝；泄漏窗口闭合）。
///
/// **fail-closed 默认**（D2 闭合 P2 泄漏窗口）：无审批通道接入（`EgressApprover` 为 `None`）、审批
/// 超时、或审批者主动拒 → 一律 [`Self::Fail`]。**绝不**因「没人答」而放行——这正是 P2 的泄漏窗口
/// （detect-but-continue），D2 必须反转为 detect-but-**fail**（除非显式批准）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EgressVerdict {
    /// 批准放行**一次**（`Fetch.continueRequest`）。同域后续请求仍会再次悬挂审批。
    Continue,
    /// 批准放行**并记住此域**（决策3 `always_allow`）：`Fetch.continueRequest` + 把目标 eTLD+1 记进
    /// 本会话 [`ApprovedDomains`] → 同 registrable domain 的后续出口请求不再悬挂审批，直接放行。
    ContinueAndRemember,
    /// 拒绝阻断（`Fetch.failRequest{BlockedByClient}`）。fail-closed 默认走这里。
    Fail,
}

impl EgressVerdict {
    /// 是否放行（[`Self::Continue`] 或 [`Self::ContinueAndRemember`]）。
    pub fn is_continue(self) -> bool {
        matches!(self, EgressVerdict::Continue | EgressVerdict::ContinueAndRemember)
    }

    /// 是否要「记住此域」（仅 [`Self::ContinueAndRemember`]，决策3 always_allow）。
    pub fn remembers_domain(self) -> bool {
        matches!(self, EgressVerdict::ContinueAndRemember)
    }
}

/// **P3-D2：被门控出口请求的审批通道接缝**（裁决④共用审批通道）。
///
/// **为何是引擎层 trait（异步悬挂 + 跨边界裁决）**：被门控的跨域 POST / 未授权域出口在 CDP
/// `Fetch.requestPaused` handler（[`crate::backend::cdp::spawn_fetch_firewall_loop`]）里命中——那是
/// 一个只持 [`crate::transport::Connection`] + [`FirewallConfig`] 的 `'static` 后台任务，**无会话/审批
/// 上下文**，且**绝不能在 CDP 事件 handler 里同步阻塞**（会卡死整个事件循环——所有 session 的
/// requestPaused/attachedToTarget 都经它）。故 D2 的设计是：
///
/// 1. handler 命中 GatePost → **悬挂**请求（保留 `requestId`，不 continue/不 fail）；
/// 2. `tokio::spawn` 一个 detached 任务（事件循环立即回到 `select!` 继续 pump，**不阻塞**）；
/// 3. 该任务 `await` 本 trait 的 [`Self::approve_egress`]（带超时）取裁决；
/// 4. 据裁决发 `continueRequest`（批准）/ `failRequest`（拒绝/超时/无通道——**fail-closed**）。
///
/// **实现侧（facade / 网关）**：把本 trait 接到 GW2 的同一 pending 审批通道（`nomi_browser_confirm`
/// 那套 stash/take + 手机带外确认），由实现 `await` 用户裁决后返回 [`EgressVerdict`]。引擎只提供
/// 接缝 + 悬挂机制 + fail-closed 兜底；**真值审批是 facade/网关层的活**（与裁决④一致）。
///
/// `preview`：[`build_post_preview`] 构造的安全预览（**只** host/size/字段名，**绝不**字段值——安全
/// 红线，复用 E5）。审批者据它判「出口到哪个域、多大、哪些字段名」。
#[async_trait]
pub trait EgressApprover: Send + Sync {
    /// 对一条被门控的出口请求请求人在回路裁决。返回 [`EgressVerdict::Continue`]（批准）/
    /// [`EgressVerdict::Fail`]（拒绝）。**实现绝不 panic**；拿不准 / 超出能力 → 返回 `Fail`
    /// （fail-closed，与接线层超时/无通道默认一致）。
    async fn approve_egress(&self, preview: &PostPreview) -> EgressVerdict;
}

/// **P3-D2：被门控出口请求等审批裁决的悬挂超时**（裁决④ / 良性 fail-closed）。
///
/// 审批通道在此 deadline 内未给出裁决（用户没在手机上点、通道卡住）→ 接线层 fail-closed
/// （`failRequest`）。**绝不**无限悬挂一条请求（会卡住页面那次提交永不回包）——超时即拒（拒绝跨域
/// POST 比放行安全，泄漏窗口闭合）。120s 给真人审批留足窗口，又不至于让一次卡住的提交永久挂起。
pub const EGRESS_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// **P3-D2：per-session「记住此域」已批准出口域集合**（决策3 `always_allow`）。
///
/// 用户在审批一条被门控的出口请求时可选「记住此域」→ 把目标 eTLD+1 记进本集合 → **同 eTLD+1 的
/// 后续出口请求不再悬挂审批，直接放行**（[`Self::is_approved`] 命中即 continue）。这是 per-session
/// 软放行（进程内、随引擎生命周期；非持久——持久域策略走 `FirewallConfig.allow_etld1` 的 secret 真值，
/// X2 的活），避免同一会话内对同一已信任域反复弹审批。
///
/// 线程安全（`Arc<Mutex<HashSet>>`）：接线层后台任务（spawn 的 detached 审批任务）与可能的并发
/// requestPaused 共享同一份；锁临界区极短（一次 `insert`/`contains`，不跨 await）。
///
/// **eTLD+1 归一**：`record`/`is_approved` 都经 [`registrable_domain_for_trust`] 归一目标 URL/host
/// （与 [`domain_policy`] / `FirewallConfig.allow_etld1` 同款 PSL 机器，但额外排除 IP 字面量）——故记住
/// `https://pay.com/x` 后，`api.pay.com` 的后续请求也命中（同 registrable domain）。无法解析出 eTLD+1
/// 的目标（IP/localhost/畸形）**绝不**记入 / 命中（fail-closed：无可信 registrable domain 无从「记住一个
/// 域」）。
#[derive(Clone, Default)]
pub struct ApprovedDomains {
    inner: Arc<Mutex<HashSet<String>>>,
}

/// **P3-D2 [纯逻辑]：取一个目标 URL 用于「域信任」（always_allow）的 registrable domain**。
///
/// 与 [`domain_policy`]/`FirewallConfig.allow_etld1` 的 [`nomifun_secret::etld_plus_one`] 同款 PSL
/// 归一，**但额外排除 IP 字面量 host**：`psl` 不校验 IP，会对 `10.0.0.5` 吐出伪 registrable domain
/// `0.5`——而 IP 出口归 [`is_blocked_ip`] 的 IP 封禁档管，「记住此域」是**域信任**语义，对 IP 无意义
/// 且危险（会把一个伪域记进白名单）。故 host 是 IP 字面量 → `None`（fail-closed，IP 永不进 always_allow
/// 集合，仍受 IP 封禁档约束）。localhost/畸形（无 eTLD+1）同样 `None`。
fn registrable_domain_for_trust(target: &str) -> Option<String> {
    // host 是 IP 字面量（v4/v6）→ 不作为可信域（归 IP 封禁档）。
    if let Some(host) = nomifun_secret::host_of(target)
        && ip_literal_of_host(&host).is_some()
    {
        return None;
    }
    nomifun_secret::etld_plus_one(target)
}

impl ApprovedDomains {
    /// 新建一个空的已批准域集合。
    pub fn new() -> Self {
        Self::default()
    }

    /// **记住一个域**（`always_allow`）：把 `target` 的 eTLD+1 记进集合。无法解析出 eTLD+1
    /// （localhost/畸形）或 host 是 **IP 字面量**（IP 出口归 IP 封禁档管，不是「域信任」语义；且
    /// `psl` 对 IP 会吐出无意义的伪 registrable domain 如 `10.0.0.5`→`0.5`，绝不能据此「记住一个
    /// 域」）→ no-op。返回是否真的记入（便于调用方日志 / 测试）。
    pub fn record(&self, target: &str) -> bool {
        let Some(e1) = registrable_domain_for_trust(target) else {
            return false;
        };
        self.inner
            .lock()
            .expect("approved domains poisoned")
            .insert(e1)
    }

    /// **是否已批准**：`target` 的 eTLD+1 在集合内 → `true`（后续同域出口直接放行，不再悬挂审批）。
    /// 无法解析出 eTLD+1 / host 是 IP 字面量的目标 → `false`（fail-closed，见 [`Self::record`]）。
    pub fn is_approved(&self, target: &str) -> bool {
        let Some(e1) = registrable_domain_for_trust(target) else {
            return false;
        };
        self.inner
            .lock()
            .expect("approved domains poisoned")
            .contains(&e1)
    }

    /// 已记住的域数量（诊断 / 测试）。
    pub fn len(&self) -> usize {
        self.inner.lock().expect("approved domains poisoned").len()
    }

    /// 是否为空（诊断 / 测试）。
    pub fn is_empty(&self) -> bool {
        self.inner.lock().expect("approved domains poisoned").is_empty()
    }
}

/// 从被拦请求的 host 字符串解析出 IP（仅当 host **本身就是 IP 字面量**——同步可判，无 DNS）。
///
/// 域名 host（`example.com`）→ `None`（需异步 DNS 解析，E5 接线层留 `TODO`：实际 DNS→IP 后再过
/// [`is_blocked_ip`]）。IP 字面量（`10.0.0.1` / `[::1]` / `169.254.169.254`）→ `Some`。这覆盖了
/// 「直接拿内网/元数据 IP 当 URL host」这一最常见、最危险的 SSRF 形态（无需 DNS）。
pub fn ip_literal_of_host(host: &str) -> Option<IpAddr> {
    let h = host.trim();
    // IPv6 字面量在 URL host 里带方括号：[::1] → ::1。
    let h = h.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(h);
    h.parse::<IpAddr>().ok()
}

// ═══════════════════════════════════════════════════════════════════════════════
// SD-1: DNS→IP SSRF guard — injectable resolver + resolution cache + egress check
// ═══════════════════════════════════════════════════════════════════════════════

/// **DNS→IP SSRF 守卫的 host 解析抽象**（SD-1）。注入式设计：
/// - 生产实现用 tokio `lookup_host`（含超时）。
/// - 测试 fake 映射固定 host→IPs（无真实网络，完全隔离）。
///
/// 返回的 `Vec<IpAddr>` 包含该 host 的所有 A/AAAA 记录——ANY 命中 [`is_blocked_ip`] 即拦截（单条
/// 恶意记录即构成 SSRF 风险）。
#[async_trait]
pub trait HostResolver: Send + Sync {
    /// 解析一个 host 到 IP 地址列表。`Err` 代表解析失败（DNS 不可达 / NXDOMAIN / 超时等）。
    async fn resolve(&self, host: &str) -> std::io::Result<Vec<IpAddr>>;
}

/// 生产解析器：通过 `tokio::net::lookup_host` 执行真实 DNS 查询（含超时）。
pub struct TokioResolver {
    /// 单次 DNS 解析的最大耗时。超时 → Err（触发 fail-closed）。
    pub timeout: Duration,
}

impl Default for TokioResolver {
    fn default() -> Self {
        Self {
            // 2s 超时：给公网 DNS 足够窗口（多数解析 <100ms），又不阻塞请求过久。
            timeout: Duration::from_secs(2),
        }
    }
}

#[async_trait]
impl HostResolver for TokioResolver {
    async fn resolve(&self, host: &str) -> std::io::Result<Vec<IpAddr>> {
        // tokio::net::lookup_host 需要 host:port 格式；用 port 0。
        let addr = format!("{host}:0");
        let fut = tokio::net::lookup_host(addr);
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(Ok(addrs)) => Ok(addrs.map(|a| a.ip()).collect()),
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "DNS resolution timed out (SSRF guard fail-closed)",
            )),
        }
    }
}

/// **DNS→IP 解析结果缓存**（SD-1）：短 TTL，避免对同一 host 反复发 DNS 查询（子资源请求常对同域
/// 发大量 GET/Fetch）。缓存 key=host，value=(是否被阻断, 写入时刻)。
///
/// TTL 过期 → 重新解析（DNS 可能变化，不应永久缓存一个「安全」结论）。
/// 线程安全（`Arc<Mutex<…>>`）：防火墙循环是单线程 select，但 detached 审批任务可能并发查询。
#[derive(Clone)]
pub struct DnsResolverCache {
    /// 缓存条目：host → (is_blocked, insertion_time)。
    inner: Arc<Mutex<HashMap<String, (bool, Instant)>>>,
    /// 条目生存时间。
    ttl: Duration,
}

impl DnsResolverCache {
    /// 创建一个新的 DNS 解析缓存。
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    /// 查询缓存：host 是否有未过期的条目。返回 `Some(is_blocked)` 命中或 `None` 未命中/已过期。
    pub fn get(&self, host: &str) -> Option<bool> {
        let map = self.inner.lock().expect("dns cache poisoned");
        if let Some(&(blocked, ts)) = map.get(host)
            && ts.elapsed() < self.ttl
        {
            return Some(blocked);
        }
        None
    }

    /// 写入/更新缓存条目。
    pub fn insert(&self, host: &str, blocked: bool) {
        let mut map = self.inner.lock().expect("dns cache poisoned");
        map.insert(host.to_string(), (blocked, Instant::now()));
    }
}

impl Default for DnsResolverCache {
    fn default() -> Self {
        // 默认 TTL 30s：平衡 DNS 变更及时性 与 查询频率。
        Self::new(Duration::from_secs(30))
    }
}

/// DNS 解析缓存的默认 TTL。
pub const DNS_CACHE_TTL: Duration = Duration::from_secs(30);

/// **SD-1 核心逻辑：对域名 host 做 DNS→IP SSRF 检查**（异步，仅 egress 子资源使用）。
///
/// 返回 `true` = 该域名应被阻断（任一 resolved IP 命中 [`is_blocked_ip`]，或解析失败→fail-closed）。
/// 返回 `false` = 所有解析到的 IP 均为公网，放行。
///
/// **设计约束**：
/// - **仅对域名调用**（host 已是 IP 字面量时不需要走此路径——sync `ip_literal_of_host` + `decide` 已覆盖）。
/// - **fail-closed on error/timeout**（egress）：解析不出来 → 视为阻断（安全默认）。
/// - **缓存命中时不再解析**（避免重复 DNS 查询）。
/// - **检查 ALL resolved IPs**：ANY 命中 is_blocked_ip → 阻断（一个主机可有多 A/AAAA 记录）。
pub async fn check_dns_ssrf(
    host: &str,
    resolver: &dyn HostResolver,
    cache: &DnsResolverCache,
) -> bool {
    // 缓存命中 → 直接返回。
    if let Some(blocked) = cache.get(host) {
        return blocked;
    }

    // 解析 DNS。
    let blocked = match resolver.resolve(host).await {
        Ok(ips) => {
            if ips.is_empty() {
                // 解析成功但无记录 → fail-closed（异常情况，保守阻断）。
                true
            } else {
                // ANY 命中 is_blocked_ip → 阻断。
                ips.iter().any(|ip| is_blocked_ip(*ip))
            }
        }
        Err(_) => {
            // 解析失败（NXDOMAIN / timeout / 网络不可达）→ fail-closed（egress deny）。
            true
        }
    };

    cache.insert(host, blocked);
    blocked
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── [纯逻辑] is_blocked_ip 真值表（PLAN E5 点名）────────────────────────────

    #[test]
    fn blocked_ip_rejects_private_loopback_linklocal_metadata() {
        // 应封禁（true）：私网 / loopback / link-local（含云元数据）/ IPv6 loopback/link-local。
        let blocked: &[&str] = &[
            "10.0.0.1",          // RFC1918 10/8
            "192.168.1.1",       // RFC1918 192.168/16
            "172.16.0.1",        // RFC1918 172.16/12
            "172.31.255.255",    // RFC1918 172.16/12 上界
            "127.0.0.1",         // loopback
            "169.254.0.1",       // link-local
            "169.254.169.254",   // ★ 云元数据端点（经典 SSRF 支点）
            "0.0.0.0",           // unspecified / 本网络
            "100.64.0.1",        // CGNAT 100.64/10
            "100.127.255.255",   // CGNAT 上界
            "224.0.0.1",         // multicast
            "255.255.255.255",   // broadcast
            "192.0.2.5",         // documentation TEST-NET-1
            "::1",               // IPv6 loopback
            "fe80::1",           // IPv6 link-local
            "fc00::1",           // IPv6 ULA
            "fd00::1",           // IPv6 ULA
            "::",                // IPv6 unspecified
            "::ffff:10.0.0.1",   // IPv4-mapped 私网（防绕过）
            "::ffff:169.254.169.254", // IPv4-mapped 元数据（防绕过）
        ];
        for s in blocked {
            let ip: IpAddr = s.parse().unwrap();
            assert!(is_blocked_ip(ip), "expected BLOCKED for {s}");
        }
    }

    #[test]
    fn blocked_ip_allows_public() {
        // 公网放行（false）。
        let public: &[&str] = &[
            "8.8.8.8",            // Google DNS
            "1.1.1.1",            // Cloudflare DNS
            "93.184.216.34",      // example.com 历史 IP
            "140.82.121.4",       // github.com 类公网
            "2606:4700:4700::1111", // Cloudflare IPv6（公网）
            "2001:4860:4860::8888", // Google IPv6（公网）
        ];
        for s in public {
            let ip: IpAddr = s.parse().unwrap();
            assert!(!is_blocked_ip(ip), "expected ALLOWED (public) for {s}");
        }
    }

    #[test]
    fn ip_literal_of_host_parses_v4_v6_and_rejects_domain() {
        assert_eq!(
            ip_literal_of_host("169.254.169.254"),
            Some("169.254.169.254".parse().unwrap())
        );
        assert_eq!(ip_literal_of_host("[::1]"), Some("::1".parse().unwrap()));
        assert_eq!(ip_literal_of_host("10.0.0.1"), Some("10.0.0.1".parse().unwrap()));
        // 域名 → None（需异步 DNS，接线层 TODO）。
        assert_eq!(ip_literal_of_host("example.com"), None);
        assert_eq!(ip_literal_of_host("api.evil.com"), None);
    }

    // ── [纯逻辑] 跨域判定（eTLD+1）─────────────────────────────────────────────

    #[test]
    fn cross_origin_same_etld_plus_one_is_not_cross() {
        // 同 eTLD+1（x.com）→ 非跨域。
        assert!(!is_cross_origin("https://sub.x.com", "https://api.x.com/post"));
        assert!(!is_cross_origin("https://x.com", "https://x.com/submit"));
        assert!(!is_cross_origin("https://login.x.com:8443", "https://x.com/a"));
    }

    #[test]
    fn cross_origin_different_etld_plus_one_is_cross() {
        // 跨 eTLD+1 → 跨域。
        assert!(is_cross_origin("https://x.com", "https://evil.com/post"));
        assert!(is_cross_origin("https://app.x.com", "https://tracker.io/collect"));
        // co.uk 多级后缀：a.co.uk vs b.co.uk 是不同 eTLD+1（PSL 正确处理）→ 跨域。
        assert!(is_cross_origin("https://a.co.uk", "https://b.co.uk/post"));
    }

    #[test]
    fn cross_origin_ip_and_localhost_fall_back_to_host_compare() {
        // IP / localhost 无 eTLD+1 → 退化裸 host 比较：同 host 非跨域，异 host 跨域。
        assert!(!is_cross_origin("http://127.0.0.1:3000", "http://127.0.0.1:3000/post"));
        assert!(!is_cross_origin("http://localhost:5173", "http://localhost:5173/api"));
        assert!(is_cross_origin("http://127.0.0.1", "http://10.0.0.5/post"));
        assert!(is_cross_origin("http://localhost", "http://evil.com/post"));
    }

    #[test]
    fn cross_origin_malformed_is_conservatively_cross() {
        // 导不出 host（畸形）→ 保守判跨域（fail-closed）。
        assert!(is_cross_origin("", "https://x.com"));
        assert!(is_cross_origin("https://x.com", ""));
    }

    // ── [纯逻辑] 跨域 POST 门控判定 ────────────────────────────────────────────

    #[test]
    fn gated_post_same_origin_post_not_gated() {
        // 同 eTLD+1 的 POST → 不门控。
        assert!(!is_gated_post(
            "POST",
            true,
            Some(b"username=alice"),
            "https://x.com",
            "https://api.x.com/login"
        ));
    }

    #[test]
    fn gated_post_cross_origin_post_with_body_is_gated() {
        // 跨 eTLD+1 + POST + 有 body → 门控。
        assert!(is_gated_post(
            "POST",
            true,
            Some(b"card=4111111111111111"),
            "https://shop.com",
            "https://evil.com/collect"
        ));
        // has_post_data=false 但 body 非空也算有 body。
        assert!(is_gated_post(
            "POST",
            false,
            Some(b"x=1"),
            "https://shop.com",
            "https://evil.com/collect"
        ));
    }

    #[test]
    fn gated_post_cross_origin_get_not_gated() {
        // 跨域但 GET（无 body 写）→ 不门控。
        assert!(!is_gated_post(
            "GET",
            false,
            None,
            "https://x.com",
            "https://evil.com/track"
        ));
    }

    #[test]
    fn gated_post_cross_origin_post_without_body_not_gated() {
        // 跨域 POST 但确无 body → 不门控（无 body 就无数据外泄风险）。
        assert!(!is_gated_post(
            "POST",
            false,
            None,
            "https://x.com",
            "https://evil.com/ping"
        ));
        assert!(!is_gated_post(
            "POST",
            false,
            Some(b""),
            "https://x.com",
            "https://evil.com/ping"
        ));
    }

    #[test]
    fn gated_post_covers_other_body_write_methods() {
        // PUT/PATCH/DELETE 跨域带 body 同样门控（含 body 的跨域写）。
        for m in ["PUT", "PATCH", "DELETE", "put", "patch"] {
            assert!(
                is_gated_post(m, true, Some(b"x=1"), "https://x.com", "https://evil.com/r"),
                "method {m} cross-origin with body should be gated"
            );
        }
    }

    // ── [纯逻辑] 字段名解析（绝不含值）─────────────────────────────────────────

    #[test]
    fn parse_form_field_names_extracts_names_only() {
        let body = b"username=alice&password=hunter2&csrf_token=abc123";
        let names = parse_form_field_names(body);
        assert_eq!(names, vec!["username", "password", "csrf_token"]);
    }

    #[test]
    fn parse_form_field_names_never_contains_values() {
        // ★ 安全红线：解析结果绝不含任何字段值（即使值是敏感数据）。
        let body = b"password=SuperSecret123&token=sk-live-abcdef&pin=4242";
        let names = parse_form_field_names(body);
        for v in ["SuperSecret123", "sk-live-abcdef", "4242"] {
            assert!(
                !names.iter().any(|n| n.contains(v)),
                "field names must NOT contain value {v:?}; got {names:?}"
            );
        }
        assert_eq!(names, vec!["password", "token", "pin"]);
    }

    #[test]
    fn parse_form_field_names_percent_decodes_keys_and_dedups() {
        // 字段名 percent-decode（%20→空格，+→空格）+ 去重保序。
        let body = b"first+name=a&first+name=b&field%5Bx%5D=c";
        let names = parse_form_field_names(body);
        assert_eq!(names, vec!["first name", "field[x]"]);
    }

    #[test]
    fn parse_form_field_names_handles_bare_and_empty() {
        // 裸 token（无 '='）算字段名；空对跳过。
        assert_eq!(parse_form_field_names(b"flag&x=1&"), vec!["flag", "x"]);
        assert_eq!(parse_form_field_names(b""), Vec::<String>::new());
        assert_eq!(parse_form_field_names(b"&&"), Vec::<String>::new());
    }

    // ── [纯逻辑] 预览构造（只 host/size/字段名，绝不含值）──────────────────────

    #[test]
    fn build_post_preview_has_host_size_field_names_no_values() {
        let body = b"username=alice&password=hunter2";
        let preview = build_post_preview(
            "https://evil.example.com/collect?q=1",
            Some(body),
            Some("application/x-www-form-urlencoded"),
        );
        assert_eq!(preview.host, "evil.example.com");
        assert_eq!(preview.size, body.len());
        assert_eq!(preview.field_names, vec!["username", "password"]);
        // ★ 红线：预览的任何字段都不含字段值。
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(!serialized.contains("alice"), "preview leaked value 'alice': {serialized}");
        assert!(!serialized.contains("hunter2"), "preview leaked value 'hunter2': {serialized}");
    }

    #[test]
    fn build_post_preview_non_form_body_gives_host_size_no_field_names() {
        // JSON body（非 urlencoded）→ field_names 空（不把 JSON key 当字段名泄露），但仍给 host+size。
        let body = br#"{"password":"secret","amount":9999}"#;
        let preview = build_post_preview(
            "https://evil.com/api",
            Some(body),
            Some("application/json"),
        );
        assert_eq!(preview.host, "evil.com");
        assert_eq!(preview.size, body.len());
        assert!(preview.field_names.is_empty());
        // ★ 红线：连 JSON 的 value 也绝不出现在预览里。
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(!serialized.contains("secret"), "preview leaked JSON value: {serialized}");
    }

    #[test]
    fn build_post_preview_no_content_type_skips_field_parse() {
        // 无 Content-Type → 保守不解析字段名（仍给 host+size）。
        let body = b"username=alice";
        let preview = build_post_preview("https://evil.com/x", Some(body), None);
        assert_eq!(preview.host, "evil.com");
        assert_eq!(preview.size, body.len());
        assert!(preview.field_names.is_empty());
    }

    // ── [纯逻辑] decide 编排（IP 封禁 > 跨域 POST 门控 > 放行）──────────────────

    #[test]
    fn decide_blocks_metadata_ip_highest_priority() {
        // 解析到云元数据 IP → Block（即使同时是跨域 POST，IP 封禁优先）。
        let cfg = FirewallConfig::default();
        let d = decide(
            &cfg,
            &RequestInfo {
                resolved_ip: Some("169.254.169.254".parse().unwrap()),
                method: "POST",
                has_post_data: true,
                body: Some(b"x=1"),
                content_type: Some("application/x-www-form-urlencoded"),
                current_origin: "https://x.com",
                target_url: "http://169.254.169.254/latest/meta-data/",
                is_top_level_navigation: false,
            },
        );
        match d {
            FirewallDecision::Block { reason } => {
                assert!(reason.contains("169.254.169.254"), "reason: {reason}");
                assert!(reason.contains("SSRF") || reason.contains("metadata"), "reason: {reason}");
            }
            other => panic!("expected Block for metadata IP, got {other:?}"),
        }
    }

    #[test]
    fn decide_gates_cross_origin_post() {
        let cfg = FirewallConfig::default();
        let body = b"username=alice&password=hunter2";
        let d = decide(
            &cfg,
            &RequestInfo {
                resolved_ip: None, // 目标是域名，未同步解析 IP
                method: "POST",
                has_post_data: true,
                body: Some(body),
                content_type: Some("application/x-www-form-urlencoded"),
                current_origin: "https://shop.com",
                target_url: "https://evil.com/collect",
                is_top_level_navigation: false,
            },
        );
        match d {
            FirewallDecision::GatePost { preview } => {
                assert_eq!(preview.host, "evil.com");
                assert_eq!(preview.field_names, vec!["username", "password"]);
                // 预览绝不含值。
                assert!(!preview.field_names.iter().any(|n| n.contains("hunter2")));
            }
            other => panic!("expected GatePost for cross-origin POST, got {other:?}"),
        }
    }

    #[test]
    fn decide_allows_benign_same_origin_get() {
        let cfg = FirewallConfig::default();
        let d = decide(
            &cfg,
            &RequestInfo {
                resolved_ip: Some("8.8.8.8".parse().unwrap()), // 公网 IP
                method: "GET",
                has_post_data: false,
                body: None,
                content_type: None,
                current_origin: "https://x.com",
                target_url: "https://api.x.com/data",
                is_top_level_navigation: false,
            },
        );
        assert_eq!(d, FirewallDecision::Allow);
    }

    #[test]
    fn decide_respects_config_toggles() {
        // 关掉 IP 封禁 → 元数据 IP 不再 Block（但仍可能因跨域 POST 门控）。
        let cfg = FirewallConfig {
            block_private_ips: false,
            gate_cross_origin_post: false,
            ..Default::default()
        };
        let d = decide(
            &cfg,
            &RequestInfo {
                resolved_ip: Some("169.254.169.254".parse().unwrap()),
                method: "POST",
                has_post_data: true,
                body: Some(b"x=1"),
                content_type: Some("application/x-www-form-urlencoded"),
                current_origin: "https://x.com",
                target_url: "http://169.254.169.254/meta",
                is_top_level_navigation: false,
            },
        );
        // 两挡都关 → 一律放行（防火墙被显式关闭）。
        assert_eq!(d, FirewallDecision::Allow);
    }

    #[test]
    fn firewall_config_default_is_both_on() {
        let cfg = FirewallConfig::default();
        assert!(cfg.block_private_ips);
        assert!(cfg.gate_cross_origin_post);
        // D1：默认空域名策略（不限制出口域 = 现行为，零回归）。
        assert!(cfg.allow_etld1.is_empty());
        assert!(cfg.deny_etld1.is_empty());
    }

    // ── [纯逻辑] D1 域名档（allow_etld1/deny_etld1，deny>allow，空=不限）──────────
    //
    // 复用 nomifun_secret::etld_plus_one 同一 PSL 机器解析目标域。GatePost = 出口到未授权域升审批
    // （D2 接人在回路）；deny 命中 = 硬 Block（黑名单无审批语义）。**真值来自 secret per-pet
    // allowed_origins，注入是 X2**——这些纯逻辑测试直接构造 FirewallConfig 验 decide 强制域名策略。

    /// 构造一个「公网 GET、无 POST body」的请求（隔离域名档逻辑：IP 封禁不触发、跨域 POST 门控不触发，
    /// 所有裁决差异都来自域名档）。`resolved_ip=None`（域名未解析 IP），方法 GET 无 body。
    /// `is_top_level_navigation=false`（子资源语义：受 allowlist 出口门控；顶层导航豁免见
    /// [`domain_policy_top_level_navigation_exempt_from_allowlist`]）。
    fn req_get<'a>(current_origin: &'a str, target_url: &'a str) -> RequestInfo<'a> {
        RequestInfo {
            resolved_ip: None,
            method: "GET",
            has_post_data: false,
            body: None,
            content_type: None,
            current_origin,
            target_url,
            is_top_level_navigation: false,
        }
    }

    #[test]
    fn domain_policy_empty_allowlist_does_not_restrict() {
        // 空 allowlist + 空 deny = 不限制出口域（现行为/零回归）→ 公网 GET 放行。
        let cfg = FirewallConfig::default();
        assert!(cfg.allow_etld1.is_empty() && cfg.deny_etld1.is_empty());
        let d = decide(&cfg, &req_get("https://x.com", "https://anything.example.org/page"));
        assert_eq!(d, FirewallDecision::Allow);
    }

    #[test]
    fn domain_policy_allowlist_admits_target_in_list() {
        // allow_etld1=["x.com"]：目标 a.x.com（同 eTLD+1）→ 放行。
        let cfg = FirewallConfig {
            allow_etld1: vec!["x.com".to_string()],
            ..Default::default()
        };
        let d = decide(&cfg, &req_get("https://x.com", "https://a.x.com/path"));
        assert_eq!(d, FirewallDecision::Allow, "target in allowlist must be allowed");
        // 裸 eTLD+1 目标同样放行。
        assert_eq!(
            decide(&cfg, &req_get("https://x.com", "https://x.com/")),
            FirewallDecision::Allow
        );
    }

    #[test]
    fn domain_policy_allowlist_gates_target_not_in_list() {
        // allow_etld1=["x.com"]：目标 y.com（不在白名单）→ GatePost（升 D2 审批）。
        let cfg = FirewallConfig {
            allow_etld1: vec!["x.com".to_string()],
            ..Default::default()
        };
        let d = decide(&cfg, &req_get("https://x.com", "https://y.com/collect"));
        match d {
            FirewallDecision::GatePost { preview } => {
                // 非 POST 时预览仍给目标 host（供审批者判断「出口到哪个域」），size=0/字段名空。
                assert_eq!(preview.host, "y.com");
                assert_eq!(preview.size, 0);
                assert!(preview.field_names.is_empty());
            }
            other => panic!("expected GatePost for off-allowlist domain, got {other:?}"),
        }
    }

    #[test]
    fn domain_policy_deny_blocks_and_outranks_allow() {
        // deny 优先 allow：evil.com 同时在 allow 与 deny → deny 命中 → 硬 Block。
        let cfg = FirewallConfig {
            allow_etld1: vec!["evil.com".to_string()],
            deny_etld1: vec!["evil.com".to_string()],
            ..Default::default()
        };
        let d = decide(&cfg, &req_get("https://x.com", "https://sub.evil.com/x"));
        match d {
            FirewallDecision::Block { reason } => {
                assert!(reason.contains("evil.com"), "reason: {reason}");
                assert!(reason.contains("denylist"), "reason: {reason}");
            }
            other => panic!("expected Block (deny outranks allow), got {other:?}"),
        }
    }

    #[test]
    fn domain_policy_deny_only_blocks_listed_admits_rest() {
        // 仅 deny（allow 空）：deny 命中 → Block；其余域不限制（allow 空）→ 放行。
        let cfg = FirewallConfig {
            deny_etld1: vec!["tracker.io".to_string()],
            ..Default::default()
        };
        // deny 命中 → Block。
        assert!(matches!(
            decide(&cfg, &req_get("https://x.com", "https://collect.tracker.io/p")),
            FirewallDecision::Block { .. }
        ));
        // 非 deny 域 + allow 空 = 不限制 → 放行。
        assert_eq!(
            decide(&cfg, &req_get("https://x.com", "https://example.org/page")),
            FirewallDecision::Allow
        );
    }

    #[test]
    fn domain_policy_co_uk_multilevel_suffix_correct() {
        // 复用 PSL：a.co.uk vs b.co.uk 是**不同** eTLD+1（co.uk 是公共后缀）。
        // allow_etld1=["a.co.uk"]：目标 www.a.co.uk（同 eTLD+1）放行；b.co.uk（异 eTLD+1）门控。
        let cfg = FirewallConfig {
            allow_etld1: vec!["a.co.uk".to_string()],
            ..Default::default()
        };
        assert_eq!(
            decide(&cfg, &req_get("https://a.co.uk", "https://www.a.co.uk/x")),
            FirewallDecision::Allow,
            "www.a.co.uk shares eTLD+1 a.co.uk → allowed"
        );
        assert!(
            matches!(
                decide(&cfg, &req_get("https://a.co.uk", "https://b.co.uk/x")),
                FirewallDecision::GatePost { .. }
            ),
            "b.co.uk is a distinct eTLD+1 (NOT collapsed to co.uk) → gated"
        );
    }

    #[test]
    fn domain_policy_allowlist_entries_normalized_to_etld1() {
        // allow 条目传完整 origin/子域 也安全（内部经 etld_plus_one 归一）：
        // ["https://sub.x.com:443"] 归一为 x.com → 目标 api.x.com 放行。
        let cfg = FirewallConfig {
            allow_etld1: vec!["https://sub.x.com:443/login".to_string()],
            ..Default::default()
        };
        assert_eq!(
            decide(&cfg, &req_get("https://x.com", "https://api.x.com/v1")),
            FirewallDecision::Allow,
            "allow entry normalized to eTLD+1 x.com → api.x.com allowed"
        );
    }

    #[test]
    fn domain_policy_failclosed_on_unresolvable_target_when_allowlist_nonempty() {
        // allowlist 非空 + 目标无 eTLD+1（IP/localhost/畸形）→ fail-closed 门控（无 registrable domain
        // 无从证明在白名单内）。
        let cfg = FirewallConfig {
            allow_etld1: vec!["x.com".to_string()],
            ..Default::default()
        };
        for tgt in ["http://10.0.0.5/x", "http://localhost:3000/api", "not a url"] {
            assert!(
                matches!(
                    decide(&cfg, &req_get("https://x.com", tgt)),
                    FirewallDecision::GatePost { .. }
                ),
                "unresolvable target {tgt:?} under non-empty allowlist must be gated (fail-closed)"
            );
        }
    }

    #[test]
    fn domain_policy_ip_block_still_highest_priority() {
        // IP 封禁优先于域名档：目标解析到元数据 IP，即便其 eTLD+1 在 allowlist（不可能，但配置端可乱填），
        // 仍 Block（IP 封禁最高优先）。这里用 deny 也在表但 IP 命中先 Block 证明顺序。
        let cfg = FirewallConfig {
            allow_etld1: vec!["x.com".to_string()],
            deny_etld1: vec!["x.com".to_string()],
            ..Default::default()
        };
        let mut req = req_get("https://x.com", "http://169.254.169.254/latest/meta-data/");
        req.resolved_ip = Some("169.254.169.254".parse().unwrap());
        match decide(&cfg, &req) {
            FirewallDecision::Block { reason } => {
                // IP 封禁的 reason 提 SSRF/metadata，而非 denylist——证明走的是 IP 封禁档（最高优先）。
                assert!(
                    reason.contains("SSRF") || reason.contains("metadata") || reason.contains("non-public IP"),
                    "expected IP-block reason (highest priority), got: {reason}"
                );
            }
            other => panic!("expected IP Block (highest priority), got {other:?}"),
        }
    }

    #[test]
    fn domain_policy_allowed_domain_still_subject_to_cross_origin_post_gate() {
        // 域名档放行（目标域∈allowlist）后，跨域 POST-body 仍单独门控（两档叠加）。
        // allow=["x.com"]，但跨域 POST 从 shop.x.com → api.x.com 是**同 eTLD+1**（非跨域）——
        // 故造一个 allowlist 含两域、目标在 allowlist 但相对当前 origin 跨域的 POST。
        let cfg = FirewallConfig {
            allow_etld1: vec!["shop.com".to_string(), "pay.com".to_string()],
            ..Default::default()
        };
        let body = b"card=4111111111111111";
        let req = RequestInfo {
            resolved_ip: None,
            method: "POST",
            has_post_data: true,
            body: Some(body),
            content_type: Some("application/x-www-form-urlencoded"),
            current_origin: "https://shop.com",   // 当前页
            target_url: "https://pay.com/charge", // 目标在 allowlist，但跨 eTLD+1
            is_top_level_navigation: false,        // POST 子请求（非顶层导航）
        };
        // 目标 pay.com ∈ allowlist → 域名档放行；但相对 shop.com 是跨域 POST-body → 跨域 POST 门控触发。
        match decide(&cfg, &req) {
            FirewallDecision::GatePost { preview } => {
                assert_eq!(preview.host, "pay.com");
                assert_eq!(preview.field_names, vec!["card"]);
            }
            other => panic!("expected GatePost from cross-origin POST gate (stacked after domain pass), got {other:?}"),
        }
    }

    // ── [纯逻辑] P3 回归修：域名 allowlist 只门控出口，不拦顶层 Document 导航 ──────────────────
    //
    // allowlist 是**出口/数据外泄控制**，不是导航监狱。注册任一 secret → allow_etld1 转非空，但 agent
    // 仍应能导航到任意 URL（含非白名单站、file:// fixture）。豁免**只**松绑 allow_etld1 白名单对顶层
    // 导航的拦截——IP 封禁（SSRF）/ deny 黑名单 / 跨域 POST 门控对导航仍生效（没过度豁免）。

    #[test]
    fn domain_policy_top_level_navigation_exempt_from_allowlist() {
        // allow_etld1=["x.com"]（注册 secret 后的典型态）：顶层 Document 导航到非白名单 y.com → **Allow**
        // （allowlist 不拦导航，agent 可自由导航）。
        let cfg = FirewallConfig {
            allow_etld1: vec!["x.com".to_string()],
            ..Default::default()
        };
        let mut nav = req_get("https://x.com", "https://y.com/page");
        nav.is_top_level_navigation = true;
        assert_eq!(
            decide(&cfg, &nav),
            FirewallDecision::Allow,
            "top-level Document navigation must be exempt from allowlist gating"
        );
        // 连无 eTLD+1 的目标（file://、localhost）作为顶层导航也放行（导航不该被 allowlist fail-closed 拦）。
        for tgt in [
            "file:///C:/fixtures/page.html",
            "http://localhost:3000/app",
            "https://anything.example.org/x",
        ] {
            let mut nav = req_get("https://x.com", tgt);
            nav.is_top_level_navigation = true;
            assert_eq!(
                decide(&cfg, &nav),
                FirewallDecision::Allow,
                "top-level navigation to {tgt:?} must be allowed even under a non-empty allowlist"
            );
        }
    }

    #[test]
    fn domain_policy_subresource_still_gated_by_allowlist() {
        // 同条件但**非** Document（子资源：XHR/Fetch 等）→ allowlist 出口门控**仍生效**（GatePost）。
        // （req_get 的 is_top_level_navigation=false 即子资源语义。）
        let cfg = FirewallConfig {
            allow_etld1: vec!["x.com".to_string()],
            ..Default::default()
        };
        let sub = req_get("https://x.com", "https://y.com/collect"); // is_top_level_navigation=false
        assert!(
            matches!(decide(&cfg, &sub), FirewallDecision::GatePost { .. }),
            "non-Document (subresource) egress to off-allowlist domain must still be gated"
        );
    }

    #[test]
    fn domain_policy_deny_blocks_even_top_level_navigation() {
        // deny 黑名单对顶层导航**仍硬 Block**（显式封禁名单拦导航合理；豁免只松绑 allow 白名单，不松绑 deny）。
        let cfg = FirewallConfig {
            deny_etld1: vec!["evil.com".to_string()],
            ..Default::default()
        };
        let mut nav = req_get("https://x.com", "https://sub.evil.com/landing");
        nav.is_top_level_navigation = true;
        assert!(
            matches!(decide(&cfg, &nav), FirewallDecision::Block { .. }),
            "denylisted domain must be blocked even for top-level navigation"
        );
    }

    #[test]
    fn domain_policy_ip_block_blocks_even_top_level_navigation() {
        // IP 封禁（SSRF/元数据）对顶层导航**仍硬 Block**（导航到内网/元数据 IP 必拦；豁免不触碰 IP 档）。
        // 即便 allowlist 非空（导航本会豁免 allow），IP 命中先于域名档 Block。
        let cfg = FirewallConfig {
            allow_etld1: vec!["x.com".to_string()],
            ..Default::default()
        };
        let mut nav = req_get("https://x.com", "http://169.254.169.254/latest/meta-data/");
        nav.is_top_level_navigation = true;
        nav.resolved_ip = Some("169.254.169.254".parse().unwrap());
        match decide(&cfg, &nav) {
            FirewallDecision::Block { reason } => assert!(
                reason.contains("SSRF") || reason.contains("metadata") || reason.contains("non-public IP"),
                "expected IP-block reason for metadata-IP navigation, got: {reason}"
            ),
            other => panic!("expected IP Block for top-level navigation to metadata IP, got {other:?}"),
        }
    }

    // ── [纯逻辑] P3-D2：ApprovedDomains（always_allow 记住域，eTLD+1 归一，fail-closed）─────

    #[test]
    fn approved_domains_starts_empty() {
        let a = ApprovedDomains::new();
        assert!(a.is_empty());
        assert_eq!(a.len(), 0);
        assert!(!a.is_approved("https://pay.com/charge"));
    }

    #[test]
    fn approved_domains_record_then_approves_same_etld1() {
        // 记住 pay.com → 同 eTLD+1（含子域 / 完整 URL）后续命中（决策3 always_allow）。
        let a = ApprovedDomains::new();
        assert!(a.record("https://pay.com/charge?x=1"), "should record a registrable domain");
        assert_eq!(a.len(), 1);
        // 同 eTLD+1 的各种形态都命中（PSL 归一）。
        assert!(a.is_approved("https://pay.com/other"));
        assert!(a.is_approved("https://api.pay.com/v1"), "subdomain shares eTLD+1");
        assert!(a.is_approved("pay.com"));
        // 不同域不命中。
        assert!(!a.is_approved("https://evil.com/x"));
    }

    #[test]
    fn approved_domains_record_is_idempotent_per_etld1() {
        // 同 eTLD+1 记多次只占一个槽（HashSet）。
        let a = ApprovedDomains::new();
        assert!(a.record("https://pay.com/a"));
        assert!(!a.record("https://api.pay.com/b"), "same eTLD+1 → already present");
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn approved_domains_failclosed_on_unresolvable_target() {
        // ★ fail-closed：无 registrable domain（IP/localhost/畸形）绝不记入 / 绝不命中——
        // 「记住一个域」对没有 registrable domain 的目标无意义。
        let a = ApprovedDomains::new();
        for t in ["http://10.0.0.5/x", "http://localhost:3000/api", "not a url"] {
            assert!(!a.record(t), "must not record unresolvable target {t:?}");
            assert!(!a.is_approved(t), "must not approve unresolvable target {t:?}");
        }
        assert!(a.is_empty());
    }

    #[test]
    fn approved_domains_co_uk_multilevel_suffix_correct() {
        // 复用 PSL：记住 a.co.uk → www.a.co.uk 命中（同 eTLD+1），b.co.uk 不命中（异 eTLD+1）。
        let a = ApprovedDomains::new();
        assert!(a.record("https://a.co.uk/login"));
        assert!(a.is_approved("https://www.a.co.uk/x"));
        assert!(!a.is_approved("https://b.co.uk/x"), "b.co.uk is a distinct eTLD+1");
    }

    #[test]
    fn approved_domains_clone_shares_inner() {
        // Clone 共享同一份（Arc）——接线层把 clone 传进后台审批任务，record 后主路径可见。
        let a = ApprovedDomains::new();
        let b = a.clone();
        assert!(b.record("https://pay.com/x"));
        assert!(a.is_approved("https://pay.com/y"), "clone shares the inner set");
    }

    // ── [纯逻辑] P3-D2：EgressVerdict + EgressApprover fail-closed 语义 ────────────────

    #[test]
    fn egress_verdict_is_copy_eq() {
        assert_eq!(EgressVerdict::Continue, EgressVerdict::Continue);
        assert_ne!(EgressVerdict::Continue, EgressVerdict::Fail);
        let v = EgressVerdict::Fail;
        let copied = v; // Copy
        assert_eq!(v, copied);
    }

    /// 一个测试用审批者：恒返回构造时给定的裁决（验接线层据裁决 continue/fail 的纯逻辑契约）。
    struct FixedApprover(EgressVerdict);
    #[async_trait]
    impl EgressApprover for FixedApprover {
        async fn approve_egress(&self, _preview: &PostPreview) -> EgressVerdict {
            self.0
        }
    }

    #[tokio::test]
    async fn egress_approver_returns_configured_verdict() {
        let preview = build_post_preview("https://evil.com/collect", Some(b"x=1"), None);
        let approve = FixedApprover(EgressVerdict::Continue);
        assert_eq!(approve.approve_egress(&preview).await, EgressVerdict::Continue);
        let deny = FixedApprover(EgressVerdict::Fail);
        assert_eq!(deny.approve_egress(&preview).await, EgressVerdict::Fail);
    }

    #[test]
    fn egress_approval_timeout_is_bounded_and_nonzero() {
        // 悬挂超时有界且非零（绝不无限悬挂；又给真人审批留足窗口）。
        assert!(EGRESS_APPROVAL_TIMEOUT > Duration::ZERO);
        assert!(EGRESS_APPROVAL_TIMEOUT <= Duration::from_secs(300));
    }

    // ── SD-1: DNS→IP SSRF guard ── ─────────────────────────────────────────────
    //
    // 测试 fake resolver：映射固定 host→IPs，无真实 DNS（完全隔离、跨平台确定性）。

    /// 测试用 fake resolver：按预配映射返回固定 IP 列表。未在映射中的 host → Err(NXDOMAIN)。
    struct FakeResolver {
        map: HashMap<String, Vec<IpAddr>>,
    }

    impl FakeResolver {
        fn new(entries: &[(&str, &[IpAddr])]) -> Self {
            let map = entries
                .iter()
                .map(|(host, ips)| (host.to_string(), ips.to_vec()))
                .collect();
            Self { map }
        }
    }

    #[async_trait]
    impl HostResolver for FakeResolver {
        async fn resolve(&self, host: &str) -> std::io::Result<Vec<IpAddr>> {
            self.map.get(host).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "NXDOMAIN (fake)")
            })
        }
    }

    #[tokio::test]
    async fn domain_resolving_to_private_ip_is_blocked() {
        // SD-1: 域名解析到内网/元数据 IP → 阻断（SSRF 守卫）。
        let resolver = FakeResolver::new(&[
            ("evil.internal", &["10.0.0.5".parse::<IpAddr>().unwrap()]),
            (
                "metadata.attacker.com",
                &["169.254.169.254".parse::<IpAddr>().unwrap()],
            ),
            (
                "multi.evil.test",
                &[
                    "8.8.8.8".parse::<IpAddr>().unwrap(),     // 公网
                    "192.168.1.1".parse::<IpAddr>().unwrap(), // 私网 ← ANY 命中即阻断
                ],
            ),
        ]);
        let cache = DnsResolverCache::new(Duration::from_secs(60));

        // 解析到 10.x (RFC1918) → blocked
        assert!(
            check_dns_ssrf("evil.internal", &resolver, &cache).await,
            "domain resolving to 10.0.0.5 (private) must be blocked"
        );

        // 解析到 169.254.169.254 (metadata) → blocked
        assert!(
            check_dns_ssrf("metadata.attacker.com", &resolver, &cache).await,
            "domain resolving to 169.254.169.254 (metadata) must be blocked"
        );

        // 多条记录中有一条私网 → blocked (ANY hit)
        assert!(
            check_dns_ssrf("multi.evil.test", &resolver, &cache).await,
            "domain with ANY private IP in resolution must be blocked"
        );
    }

    #[tokio::test]
    async fn domain_resolving_to_public_ip_is_not_blocked() {
        // SD-1: 域名解析到公网 IP → 不阻断。
        let resolver = FakeResolver::new(&[
            (
                "safe.example.com",
                &["93.184.216.34".parse::<IpAddr>().unwrap()],
            ),
            (
                "cdn.example.com",
                &[
                    "8.8.8.8".parse::<IpAddr>().unwrap(),
                    "1.1.1.1".parse::<IpAddr>().unwrap(),
                ],
            ),
        ]);
        let cache = DnsResolverCache::new(Duration::from_secs(60));

        assert!(
            !check_dns_ssrf("safe.example.com", &resolver, &cache).await,
            "domain resolving to public IP must NOT be blocked"
        );

        assert!(
            !check_dns_ssrf("cdn.example.com", &resolver, &cache).await,
            "domain resolving to all-public IPs must NOT be blocked"
        );
    }

    #[tokio::test]
    async fn dns_resolution_failure_is_fail_closed() {
        // SD-1: DNS 解析失败（NXDOMAIN / 超时）→ fail-closed（egress deny）。
        let resolver = FakeResolver::new(&[]); // 所有 host 都会 NXDOMAIN
        let cache = DnsResolverCache::new(Duration::from_secs(60));

        assert!(
            check_dns_ssrf("unknown.host.invalid", &resolver, &cache).await,
            "DNS resolution failure must fail-closed (block)"
        );
    }

    #[tokio::test]
    async fn dns_cache_prevents_repeated_resolution() {
        // SD-1: 缓存命中时不再调用 resolver。
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingResolver {
            inner: FakeResolver,
            count: AtomicUsize,
        }

        #[async_trait]
        impl HostResolver for CountingResolver {
            async fn resolve(&self, host: &str) -> std::io::Result<Vec<IpAddr>> {
                self.count.fetch_add(1, Ordering::SeqCst);
                self.inner.resolve(host).await
            }
        }

        let resolver = CountingResolver {
            inner: FakeResolver::new(&[(
                "cached.example.com",
                &["8.8.8.8".parse::<IpAddr>().unwrap()],
            )]),
            count: AtomicUsize::new(0),
        };
        let cache = DnsResolverCache::new(Duration::from_secs(60));

        // 第一次调用 → 解析一次。
        let blocked = check_dns_ssrf("cached.example.com", &resolver, &cache).await;
        assert!(!blocked);
        assert_eq!(resolver.count.load(Ordering::SeqCst), 1);

        // 第二次调用 → 缓存命中，不再解析。
        let blocked = check_dns_ssrf("cached.example.com", &resolver, &cache).await;
        assert!(!blocked);
        assert_eq!(resolver.count.load(Ordering::SeqCst), 1, "cache must prevent repeated DNS resolution");
    }
}
