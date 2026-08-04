# nomifun-bridge 桌面端模块实施计划（Plan A）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 nomifun-tauri 内新增 `crates/backend/nomifun-bridge`：桌面端的移动远控桥（E2E 加密、配对、精简 RPC、事件推送、LAN 监听 + 中继出站），并接入 Tauri 命令与设置 UI。

**Architecture:** 纯库 crate `nomifun-bridge` 持有克隆的 axum `Router`（in-process oneshot + `x-nomi-local-trust` 头，以安装所有者身份调用既有 HTTP API），`BridgeCore` 统一处理来自 LAN WS 与中继连接的 E2E 帧；`BridgeService` 提供生命周期与状态给 Tauri 命令/UI。

**Tech Stack:** Rust edition 2024；crypto_box 0.9（X25519+XSalsa20-Poly1305）、hkdf 0.12、hmac、sha2、hex、base64；axum 0.8 (ws)、tower(util)、tokio-tungstenite 0.26；前端 React + Arco（仿 `WebuiControlPanel.tsx`）。

## Global Constraints

- 规范：`docs/superpowers/specs/2026-08-03-bridge-protocol-v1.md`（字段/算法/常量 BINDING）与设计 spec `2026-08-03-nomifun-mobile-bridge-design.md`
- LAN 端口 25810（被占则系统分配）；中继默认 wss；单结果 ≤16KB 截断；事件 result_text ≤2048 字符；不转发 `message.stream`
- 提交人必须是 `RiKa0-0 <2206491416@qq.com>`，无 AI 署名 trailer，不得 `--no-verify`；禁止 `.github/workflows/`
- 测试：`cargo test -p nomifun-bridge -- --test-threads=2`（本机内存有限）；全仓验证用 `cargo check --workspace`
- 桌面既有事实（已核实）：`LOCAL_TRUST_HEADER = "x-nomi-local-trust"`（nomifun-auth/src/trust.rs:32）；`ApiResponse{success,data,message}`；`PaginatedResult{items,total,has_more}`；`AgentType` serde lowercase（"nomi"…）；`ConversationStatus` lowercase；`MessageType` snake_case（助手文本 = `type:"text"` 且 `position:"left"`）；事件名 `turn.completed` / `cron.job-executed`；`AppServices.event_bus: Arc<BroadcastEventBus>`（services.rs:973）；send 需 `idempotency-key` 头（≤128 可见 ASCII）

---

### Task A1: crate 脚手架与 workspace 依赖

**Files:**
- Modify: `Cargo.toml`（根：`[workspace.dependencies]` 增加 `crypto_box = "0.9"`、`hkdf = "0.12"`、`nomifun-bridge = { path = "crates/backend/nomifun-bridge" }`）
- Create: `crates/backend/nomifun-bridge/Cargo.toml`, `crates/backend/nomifun-bridge/src/lib.rs`

- [ ] **Step 1: 根 Cargo.toml 追加依赖**（放入现有 `[workspace.dependencies]` 段，字母序就近）

```toml
crypto_box = "0.9"
hkdf = "0.12"
nomifun-bridge = { path = "crates/backend/nomifun-bridge" }
```

- [ ] **Step 2: crate Cargo.toml**

```toml
[package]
name = "nomifun-bridge"
version.workspace = true
edition.workspace = true

[dependencies]
anyhow = { workspace = true }
async-trait = { workspace = true }
axum = { workspace = true }
base64 = { workspace = true }
crypto_box = { workspace = true }
futures-util = { workspace = true }
getrandom = { workspace = true }
hex = { workspace = true }
hkdf = { workspace = true }
hmac = { workspace = true }
http = { workspace = true }
nomifun-api-types = { workspace = true }
nomifun-realtime = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sha2 = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tokio-tungstenite = { workspace = true }
tower = { workspace = true, features = ["util"] }
tracing = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

（若 `http`/`futures-util`/`tempfile` 不在 workspace.dependencies，先 `rg '^http|^futures-util|^tempfile' Cargo.toml` 确认；缺则按既有版本风格补入根表。）

`src/lib.rs`:

```rust
pub mod core;
pub mod crypto;
pub mod events;
pub mod identity;
pub mod lan;
pub mod pairing;
pub mod protocol;
pub mod relay_client;
pub mod rpc;
pub mod service;
```

（先建同名空文件占位使 `cargo check -p nomifun-bridge` 通过——每个文件放 `//! module` 注释即可。）

- [ ] **Step 3: 验证** — Run: `cargo check -p nomifun-bridge`；Expected: OK
- [ ] **Step 4: Commit** — `git add -A && git commit -m "feat(bridge): scaffold nomifun-bridge crate"`

### Task A2: 身份（identity.rs）

**Interfaces:**
- Produces: `Identity { sk: SecretKey, pk: PublicKey, device_id: String }`；`Identity::load_or_create(path: &Path) -> anyhow::Result<Identity>`；`Identity::from_secret_bytes([u8;32]) -> Identity`；`pub fn device_id_from_pk(pk: &PublicKey) -> String`

- [ ] **Step 1: 失败测试**（`src/identity.rs` 内 `#[cfg(test)]`）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_16_hex_of_sha512_prefix() {
        let id = Identity::from_secret_bytes([7u8; 32]);
        assert_eq!(id.device_id.len(), 16);
        assert!(id.device_id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        use sha2::{Digest, Sha512};
        let d = Sha512::digest(id.pk.as_bytes());
        assert_eq!(id.device_id, hex::encode(&d[..8]));
    }

    #[test]
    fn load_or_create_persists_and_reloads_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("identity.key");
        let a = Identity::load_or_create(&p).unwrap();
        let b = Identity::load_or_create(&p).unwrap();
        assert_eq!(a.device_id, b.device_id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
```

- [ ] **Step 2: 验证失败** — Run: `cargo test -p nomifun-bridge identity`；Expected: FAIL
- [ ] **Step 3: 实现**

```rust
use std::path::Path;

use crypto_box::{PublicKey, SecretKey};
use sha2::{Digest, Sha512};

pub fn device_id_from_pk(pk: &PublicKey) -> String {
    let digest = Sha512::digest(pk.as_bytes());
    hex::encode(&digest[..8])
}

#[derive(Clone)]
pub struct Identity {
    pub sk: SecretKey,
    pub pk: PublicKey,
    pub device_id: String,
}

impl Identity {
    pub fn from_secret_bytes(bytes: [u8; 32]) -> Self {
        let sk = SecretKey::from(bytes);
        let pk = sk.public_key();
        let device_id = device_id_from_pk(&pk);
        Self { sk, pk, device_id }
    }

    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if let Ok(raw) = std::fs::read(path) {
            let bytes: [u8; 32] = raw
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("corrupt identity key at {}", path.display()))?;
            return Ok(Self::from_secret_bytes(bytes));
        }
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self::from_secret_bytes(bytes))
    }
}
```

- [ ] **Step 4: 验证通过** — Run: `cargo test -p nomifun-bridge identity`；Expected: 2 passed
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): x25519 identity with persisted key"`

### Task A3: E2E 加解密 + 配对 MAC + 抗重放（crypto.rs）

**Interfaces:**
- Produces:

```rust
pub struct E2EFrame { pub v: u8, pub from: String, pub pk: Option<String>, pub n: String, pub c: String } // serde: skip pk if None
pub fn seal_with_nonce(inner: &serde_json::Value, nonce: &[u8; 24], from: &str, include_pk: Option<&PublicKey>, peer_pk: &PublicKey, self_sk: &SecretKey) -> E2EFrame
pub fn seal(...同 seal_with_nonce 但随机 nonce...) -> E2EFrame
pub fn open(frame: &E2EFrame, peer_pk: &PublicKey, self_sk: &SecretKey) -> Result<serde_json::Value, CryptoError>
pub fn pair_mac(code: &str, desktop_pk: &PublicKey, mobile_pk: &PublicKey) -> String
pub struct CtrGuard { last: u64 } // new(last), accept(ctr)->bool
pub const B64: base64::engine::general_purpose::GeneralPurpose = base64::engine::general_purpose::STANDARD;
```

- [ ] **Step 1: 失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use serde_json::json;

    fn pair() -> (Identity, Identity) {
        (Identity::from_secret_bytes([1u8; 32]), Identity::from_secret_bytes([2u8; 32]))
    }

    #[test]
    fn seal_open_roundtrip() {
        let (d, m) = pair();
        let inner = json!({"kind":"rpc","ctr":2,"id":"x","method":"device.info","params":{}});
        let f = seal(&inner, &m.device_id, Some(&m.pk), &d.pk, &m.sk);
        assert_eq!(f.v, 1);
        assert!(f.pk.is_some());
        let opened = open(&f, &m.pk, &d.sk).unwrap();
        assert_eq!(opened, inner);
    }

    #[test]
    fn tampered_cipher_fails() {
        let (d, m) = pair();
        let mut f = seal(&json!({"a":1}), &m.device_id, None, &d.pk, &m.sk);
        let mut c = crate::crypto::b64_decode(&f.c).unwrap();
        c[0] ^= 0xff;
        f.c = b64_encode(&c);
        assert!(open(&f, &m.pk, &d.sk).is_err());
    }

    #[test]
    fn pair_mac_matches_manual_hkdf_hmac() {
        let (d, m) = pair();
        let mac = pair_mac("12345678", &d.pk, &m.pk);
        assert_eq!(mac.len(), 64);
        // 换 code 则不同
        assert_ne!(mac, pair_mac("87654321", &d.pk, &m.pk));
        // 参数顺序敏感：desktop_pk 在前
        assert_ne!(mac, pair_mac("12345678", &m.pk, &d.pk));
    }

    #[test]
    fn ctr_guard_rejects_replay() {
        let mut g = CtrGuard::new(0);
        assert!(g.accept(1));
        assert!(g.accept(5));
        assert!(!g.accept(5));
        assert!(!g.accept(4));
        assert!(g.accept(6));
    }

    #[test]
    fn frame_serde_skips_absent_pk() {
        let (d, m) = pair();
        let f = seal(&json!({}), &m.device_id, None, &d.pk, &m.sk);
        let s = serde_json::to_string(&f).unwrap();
        assert!(!s.contains("\"pk\""));
        assert!(s.contains("\"v\":1"));
    }
}
```

- [ ] **Step 2: 验证失败** — Run: `cargo test -p nomifun-bridge crypto`；Expected: FAIL
- [ ] **Step 3: 实现**

```rust
use base64::Engine;
use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub const PAIR_SALT: &[u8] = b"nomifun-bridge-pair-v1";

pub fn b64_encode(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, CryptoError> {
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| CryptoError::Encoding)
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("bad encoding")]
    Encoding,
    #[error("decrypt failed")]
    Decrypt,
    #[error("bad plaintext")]
    Plaintext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2EFrame {
    pub v: u8,
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pk: Option<String>,
    pub n: String,
    pub c: String,
}

pub fn seal_with_nonce(
    inner: &serde_json::Value,
    nonce: &[u8; 24],
    from: &str,
    include_pk: Option<&PublicKey>,
    peer_pk: &PublicKey,
    self_sk: &SecretKey,
) -> E2EFrame {
    let sbox = SalsaBox::new(peer_pk, self_sk);
    let plaintext = serde_json::to_vec(inner).expect("inner message serializes");
    let cipher = sbox.encrypt(nonce.into(), plaintext.as_slice()).expect("xsalsa20poly1305 encrypt");
    E2EFrame {
        v: 1,
        from: from.to_string(),
        pk: include_pk.map(|pk| b64_encode(pk.as_bytes())),
        n: b64_encode(nonce),
        c: b64_encode(&cipher),
    }
}

pub fn seal(
    inner: &serde_json::Value,
    from: &str,
    include_pk: Option<&PublicKey>,
    peer_pk: &PublicKey,
    self_sk: &SecretKey,
) -> E2EFrame {
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let nonce_bytes: [u8; 24] = nonce.into();
    seal_with_nonce(inner, &nonce_bytes, from, include_pk, peer_pk, self_sk)
}

pub fn open(frame: &E2EFrame, peer_pk: &PublicKey, self_sk: &SecretKey) -> Result<serde_json::Value, CryptoError> {
    let nonce = b64_decode(&frame.n)?;
    let cipher = b64_decode(&frame.c)?;
    if nonce.len() != 24 {
        return Err(CryptoError::Encoding);
    }
    let sbox = SalsaBox::new(peer_pk, self_sk);
    let plain = sbox
        .decrypt(crypto_box::Nonce::from_slice(&nonce), cipher.as_slice())
        .map_err(|_| CryptoError::Decrypt)?;
    serde_json::from_slice(&plain).map_err(|_| CryptoError::Plaintext)
}

pub fn pair_mac(code: &str, desktop_pk: &PublicKey, mobile_pk: &PublicKey) -> String {
    let hk = Hkdf::<Sha256>::new(Some(PAIR_SALT), code.as_bytes());
    let mut prk = [0u8; 32];
    hk.expand(&[], &mut prk).expect("32B hkdf output");
    let mut mac = Hmac::<Sha256>::new_from_slice(&prk).expect("hmac accepts 32B key");
    mac.update(desktop_pk.as_bytes());
    mac.update(mobile_pk.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[derive(Debug)]
pub struct CtrGuard {
    last: u64,
}

impl CtrGuard {
    pub fn new(last: u64) -> Self {
        Self { last }
    }
    pub fn accept(&mut self, ctr: u64) -> bool {
        if ctr > self.last {
            self.last = ctr;
            true
        } else {
            false
        }
    }
    pub fn last(&self) -> u64 {
        self.last
    }
}
```

（若 `crypto_box::Nonce::from_slice` 在 0.9 不可用，改用 `crypto_box::Nonce::try_from(&nonce[..]).map_err(...)` 或 `GenericArray::from_slice`——以 `cargo doc -p crypto_box` 为准。）

- [ ] **Step 4: 验证通过** — Run: `cargo test -p nomifun-bridge crypto`；Expected: 5 passed
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): e2e frame seal/open, pair mac, replay guard"`

### Task A4: 互操作测试向量（tests/vectors）

**Files:**
- Create: `crates/backend/nomifun-bridge/tests/vectors_gen.rs`, `crates/backend/nomifun-bridge/tests/vectors/bridge_e2e_vectors.json`（由测试生成后检入）

- [ ] **Step 1: 写生成+自校验测试**

```rust
use nomifun_bridge::crypto::{b64_encode, open, pair_mac, seal_with_nonce};
use nomifun_bridge::identity::Identity;
use serde_json::json;

#[test]
fn generate_and_verify_interop_vectors() {
    let desktop = Identity::from_secret_bytes([1u8; 32]);
    let mobile = Identity::from_secret_bytes([2u8; 32]);
    let nonce = [3u8; 24];
    let plaintext = json!({"kind":"rpc","ctr":2,"id":"00000000-0000-4000-8000-000000000000","method":"device.info","params":{}});
    let frame = seal_with_nonce(&plaintext, &nonce, &mobile.device_id, None, &desktop.pk, &mobile.sk);
    // 自校验：desktop 能解开
    assert_eq!(open(&frame, &mobile.pk, &desktop.sk).unwrap(), plaintext);

    let code = "12345678";
    let vectors = json!({
        "desktop_sk": b64_encode(&[1u8; 32]),
        "desktop_pk": b64_encode(desktop.pk.as_bytes()),
        "mobile_sk": b64_encode(&[2u8; 32]),
        "mobile_pk": b64_encode(mobile.pk.as_bytes()),
        "nonce": b64_encode(&nonce),
        "plaintext": plaintext.to_string(),
        "cipher_b64": frame.c,
        "pair_code": code,
        "pair_mac_hex": pair_mac(code, &desktop.pk, &mobile.pk),
        "device_id_of_desktop_pk": desktop.device_id,
        "device_id_of_mobile_pk": mobile.device_id,
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/bridge_e2e_vectors.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let rendered = serde_json::to_string_pretty(&vectors).unwrap();
    match std::fs::read_to_string(&path) {
        Ok(existing) => assert_eq!(existing, rendered, "vectors drifted — protocol change?"),
        Err(_) => std::fs::write(&path, rendered).unwrap(),
    }
}
```

- [ ] **Step 2: 运行两次** — Run: `cargo test -p nomifun-bridge --test vectors_gen && cargo test -p nomifun-bridge --test vectors_gen`；Expected: 两次均 PASS（第一次生成、第二次比对）
- [ ] **Step 3: Commit（含生成的 json）** — `git add -A && git commit -m "test(bridge): interop vectors for mobile parity"`

### Task A5: 内层协议类型（protocol.rs）

**Interfaces:**
- Produces:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inner { PairRequest{ctr,code,name,platform}, PairOk{ctr,name,mac}, PairErr{ctr,code}, Rpc{ctr,id,method,params}, RpcResult{ctr,id,ok,result?,error?}, Event{ctr,name,data} }
pub struct RpcErrorBody { pub code: String, pub message: String }
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayFrame { Register{role,id,ts,auth}, Registered, Forward{to,frame:serde_json::Value}, Deliver{from,frame:serde_json::Value}, Error{code,to?}, Ping, Pong }
pub fn relay_auth(server_key: &str, id: &str, ts: i64) -> String  // hex hmac-sha256(key, id+":"+ts)
```

- [ ] **Step 1: 失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_wire_format_matches_protocol() {
        let m: Inner = serde_json::from_str(r#"{"kind":"pair_request","ctr":1,"code":"12345678","name":"Pixel","platform":"android"}"#).unwrap();
        assert!(matches!(m, Inner::PairRequest { ref code, .. } if code == "12345678"));
        let s = serde_json::to_string(&Inner::RpcResult { ctr: 3, id: "i1".into(), ok: false, result: None, error: Some(RpcErrorBody { code: "not_found".into(), message: "x".into() }) }).unwrap();
        assert_eq!(s, r#"{"kind":"rpc_result","ctr":3,"id":"i1","ok":false,"error":{"code":"not_found","message":"x"}}"#);
    }

    #[test]
    fn relay_auth_is_hex_hmac() {
        let a = relay_auth("key1", "dev1", 1700000000000);
        assert_eq!(a.len(), 64);
        assert_eq!(a, relay_auth("key1", "dev1", 1700000000000));
        assert_ne!(a, relay_auth("key2", "dev1", 1700000000000));
    }
}
```

- [ ] **Step 2: 验证失败** — Run: `cargo test -p nomifun-bridge protocol`；Expected: FAIL
- [ ] **Step 3: 实现**（`Inner`/`RelayFrame` 按上方 Interfaces 全量定义；`RpcResult` 的 `result`/`error` 用 `#[serde(default, skip_serializing_if = "Option::is_none")]`；`relay_auth` 用 `Hmac<Sha256>`，与 Plan B `compute_auth` 相同算法）
- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge protocol`
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): inner message and relay frame types"`

### Task A6: 配对与设备存储（pairing.rs）

**Interfaces:**
- Produces:

```rust
pub struct PairingManager { .. } // Mutex<Option<(code, expires_at_ms)>>
impl PairingManager { pub fn new(); pub fn generate(&self, now_ms: i64) -> (String /*8位数字*/, i64 /*expires*/); pub fn take_if_valid(&self, code: &str, now_ms: i64) -> bool /*单次消费*/ }
#[derive(Serialize, Deserialize, Clone)] pub struct DeviceRecord { pub id: String, pub pk: String /*b64*/, pub name: String, pub platform: String, pub paired_at_ms: i64, pub last_ctr_in: u64, pub last_ctr_out: u64 }
pub struct DeviceStore { .. } // path + Mutex<HashMap<id, DeviceRecord>>; devices.json 原子写(tmp+rename)
impl DeviceStore { pub fn load(path) -> Self; pub fn insert(&self, rec) ; pub fn remove(&self, id) -> bool; pub fn get(&self, id) -> Option<DeviceRecord>; pub fn list(&self) -> Vec<DeviceRecord>; pub fn accept_in_ctr(&self, id, ctr) -> bool; pub fn next_out_ctr(&self, id) -> Option<u64>; }
```

- [ ] **Step 1: 失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_single_use_and_ttl_bound() {
        let pm = PairingManager::new();
        let (code, exp) = pm.generate(1_000);
        assert_eq!(code.len(), 8);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(exp, 1_000 + 300_000);
        assert!(!pm.take_if_valid("00000000", 2_000) || code == "00000000");
        assert!(pm.take_if_valid(&code, 2_000));
        assert!(!pm.take_if_valid(&code, 2_000)); // 单次
        let (code2, _) = pm.generate(1_000);
        assert!(!pm.take_if_valid(&code2, 1_000 + 300_001)); // 过期
    }

    #[test]
    fn store_persists_and_tracks_ctrs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("devices.json");
        let s = DeviceStore::load(&p);
        s.insert(DeviceRecord { id: "m1".into(), pk: "cGs=".into(), name: "Pixel".into(), platform: "android".into(), paired_at_ms: 1, last_ctr_in: 1, last_ctr_out: 1 });
        assert!(s.accept_in_ctr("m1", 2));
        assert!(!s.accept_in_ctr("m1", 2)); // 重放
        assert_eq!(s.next_out_ctr("m1"), Some(2));
        assert_eq!(s.next_out_ctr("m1"), Some(3));
        // 重新加载后计数器仍在（跨重启持久化）
        let s2 = DeviceStore::load(&p);
        assert_eq!(s2.get("m1").unwrap().last_ctr_in, 2);
        assert_eq!(s2.get("m1").unwrap().last_ctr_out, 3);
        assert!(s2.remove("m1"));
        assert!(DeviceStore::load(&p).get("m1").is_none());
    }
}
```

- [ ] **Step 2: 验证失败** — `cargo test -p nomifun-bridge pairing`
- [ ] **Step 3: 实现**（要点：`generate` 用 `getrandom` 取 4 字节 → `u32 % 100_000_000` 零填充 8 位；覆盖旧码；`take_if_valid` 匹配且未过期则 `take()` 消费；DeviceStore 每次变更立即 `serde_json::to_vec_pretty` 写 `devices.json.tmp` 再 `rename`；`accept_in_ctr`/`next_out_ctr` 命中后立刻持久化）
- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge pairing`
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): pairing codes and persistent device store"`

### Task A7: RPC 分发器（rpc.rs）

**Interfaces:**
- Consumes: 克隆的 axum `Router` + trust secret
- Produces:

```rust
pub struct RpcHandler { router: axum::Router, trust: String, device_name: String, version: String }
impl RpcHandler {
    pub fn new(router: Router, trust: String, device_name: String, version: String) -> Self;
    pub async fn dispatch(&self, rpc_id: &str, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, RpcErrorBody>;
}
pub const MAX_RESULT_BYTES: usize = 16 * 1024;
pub fn extract_text(content: &serde_json::Value) -> String; // string | {text} | [ {text}, ... ]
pub fn truncate_utf8(s: &str, max_bytes: usize) -> (String, bool);
```

- [ ] **Step 1: 失败测试**（同文件 `#[cfg(test)]`；构造 stub Router 模拟真实端点契约）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{json, Value};

    fn stub() -> Router {
        Router::new()
            .route(
                "/api/conversations",
                get(|| async {
                    Json(json!({"success":true,"data":{"items":[{"conversation_id":"c1","name":"demo","type":"nomi","status":"finished","runtime":{"state":"idle","is_processing":false,"pending_confirmations":0,"active_turn_id":null,"processing_started_at":null},"modified_at":42}],"total":1,"has_more":false}}))
                })
                .post(|Json(body): Json<Value>| async move {
                    Json(json!({"success":true,"data":{"conversation_id":"c2","name":body["name"],"type":body["type"],"status":"pending"}}))
                }),
            )
            .route(
                "/api/conversations/{id}/messages",
                get(|| async {
                    Json(json!({"success":true,"data":{"items":[
                        {"message_id":"m2","type":"text","position":"right","content":"question","created_at":2},
                        {"message_id":"m1","type":"text","position":"left","content":[{"text":"final answer"}],"created_at":1}
                    ],"total":2,"has_more":false}}))
                })
                .post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                    assert_eq!(body["origin"], "companion");
                    let idem = headers.get("idempotency-key").unwrap().to_str().unwrap().to_string();
                    Json(json!({"success":true,"data":{"msg_id":idem,"completed":true,"result_ok":true,"result_text":"done"}}))
                }),
            )
            .route("/api/cron/jobs", get(|| async { Json(json!({"success":true,"data":[{"cron_job_id":"j1","name":"daily","enabled":true}]})) }))
            .route("/api/conversations/{id}/cancel", post(|| async { Json(json!({"success":true})) }))
            .route(
                "/api/conversations/{id}/confirmations/{call_id}/confirm",
                post(|Path((_, call_id)): Path<(String, String)>, Json(body): Json<Value>| async move {
                    assert_eq!(call_id, "call9");
                    assert_eq!(body["msg_id"], "mid1");
                    Json(json!({"success":true}))
                }),
            )
    }

    fn handler() -> RpcHandler {
        RpcHandler::new(stub(), "secret".into(), "MyDesk".into(), "0.3.7".into())
    }

    #[tokio::test]
    async fn device_info() {
        let r = handler().dispatch("r1", "device.info", &json!({})).await.unwrap();
        assert_eq!(r["name"], "MyDesk");
        assert_eq!(r["capabilities"], json!(["conversations", "cron", "confirmations"]));
    }

    #[tokio::test]
    async fn conversations_list_maps_fields() {
        let r = handler().dispatch("r1", "conversations.list", &json!({})).await.unwrap();
        assert_eq!(r["items"][0]["id"], "c1");
        assert_eq!(r["items"][0]["is_processing"], false);
        assert_eq!(r["items"][0]["updated_at"], 42);
        assert_eq!(r["has_more"], false);
    }

    #[tokio::test]
    async fn send_uses_rpc_id_as_idempotency_key() {
        let r = handler().dispatch("idem-1", "conversations.send", &json!({"conversation_id":"c1","content":"hi"})).await.unwrap();
        assert_eq!(r["msg_id"], "idem-1");
        assert_eq!(r["completed"], true);
    }

    #[tokio::test]
    async fn result_picks_latest_left_text_message() {
        let r = handler().dispatch("r1", "conversations.result", &json!({"conversation_id":"c1"})).await.unwrap();
        assert_eq!(r["text"], "final answer");
        assert_eq!(r["truncated"], false);
    }

    #[tokio::test]
    async fn unknown_method_is_bad_request() {
        let e = handler().dispatch("r1", "nope", &json!({})).await.unwrap_err();
        assert_eq!(e.code, "bad_request");
    }

    #[tokio::test]
    async fn missing_route_is_upstream_error_or_not_found() {
        let e = handler().dispatch("r1", "cron.runNow", &json!({"cron_job_id":"j9"})).await.unwrap_err();
        assert_eq!(e.code, "not_found"); // stub 未挂 run 路由 → 404
    }

    #[test]
    fn truncate_marks_flag() {
        let (s, t) = truncate_utf8(&"啊".repeat(20_000), MAX_RESULT_BYTES);
        assert!(t);
        assert!(s.len() <= MAX_RESULT_BYTES);
        let (s2, t2) = truncate_utf8("short", MAX_RESULT_BYTES);
        assert_eq!((s2.as_str(), t2), ("short", false));
    }

    #[test]
    fn extract_text_variants() {
        assert_eq!(extract_text(&json!("plain")), "plain");
        assert_eq!(extract_text(&json!({"text":"obj"})), "obj");
        assert_eq!(extract_text(&json!([{ "text": "a" }, { "text": "b" }])), "a\nb");
    }
}
```

- [ ] **Step 2: 验证失败** — `cargo test -p nomifun-bridge rpc`
- [ ] **Step 3: 实现**（核心骨架，全部方法按协议 §6 映射）

```rust
use axum::Router;
use http::{Method, Request};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::protocol::RpcErrorBody;

pub const MAX_RESULT_BYTES: usize = 16 * 1024;
pub const LOCAL_TRUST_HEADER: &str = "x-nomi-local-trust";

pub struct RpcHandler {
    router: Router,
    trust: String,
    device_name: String,
    version: String,
}

impl RpcHandler {
    pub fn new(router: Router, trust: String, device_name: String, version: String) -> Self {
        Self { router, trust, device_name, version }
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>, idem: Option<&str>) -> Result<Value, RpcErrorBody> {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(LOCAL_TRUST_HEADER, &self.trust)
            .header("content-type", "application/json");
        if let Some(k) = idem {
            builder = builder.header("idempotency-key", k);
        }
        let req = builder
            .body(axum::body::Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
            .map_err(|e| RpcErrorBody { code: "bad_request".into(), message: e.to_string() })?;
        let resp = self
            .router
            .clone()
            .oneshot(req)
            .await
            .map_err(|e| RpcErrorBody { code: "upstream_error".into(), message: e.to_string() })?;
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
            .await
            .map_err(|e| RpcErrorBody { code: "upstream_error".into(), message: e.to_string() })?;
        if status == http::StatusCode::NOT_FOUND {
            return Err(RpcErrorBody { code: "not_found".into(), message: format!("{path} -> 404") });
        }
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = parsed["message"].as_str().unwrap_or("upstream error").to_string();
            let code = if status.is_client_error() { "bad_request" } else { "upstream_error" };
            return Err(RpcErrorBody { code: code.into(), message: msg });
        }
        // ApiResponse 信封解包；非信封响应原样返回
        if parsed.get("success").is_some() {
            if parsed["success"] == json!(true) {
                Ok(parsed.get("data").cloned().unwrap_or(Value::Null))
            } else {
                Err(RpcErrorBody { code: "upstream_error".into(), message: parsed["message"].as_str().unwrap_or("").into() })
            }
        } else {
            Ok(parsed)
        }
    }

    pub async fn dispatch(&self, rpc_id: &str, method: &str, params: &Value) -> Result<Value, RpcErrorBody> {
        let str_param = |k: &str| -> Result<String, RpcErrorBody> {
            params[k]
                .as_str()
                .map(String::from)
                .ok_or_else(|| RpcErrorBody { code: "bad_request".into(), message: format!("missing param: {k}") })
        };
        match method {
            "device.info" => Ok(json!({
                "name": self.device_name, "version": self.version, "platform": std::env::consts::OS,
                "agent_types": ["nomi"],
                "capabilities": ["conversations", "cron", "confirmations"],
            })),
            "conversations.list" => {
                let limit = params["limit"].as_u64().unwrap_or(20).min(50);
                let mut path = format!("/api/conversations?limit={limit}");
                if let Some(c) = params["cursor"].as_str() {
                    path.push_str(&format!("&cursor={c}"));
                }
                let data = self.call(Method::GET, &path, None, None).await?;
                let items: Vec<Value> = data["items"].as_array().cloned().unwrap_or_default().iter().map(|it| json!({
                    "id": it["conversation_id"], "name": it["name"], "type": it["type"], "status": it["status"],
                    "is_processing": it["runtime"]["is_processing"], "updated_at": it["modified_at"],
                })).collect();
                Ok(json!({"items": items, "has_more": data["has_more"]}))
            }
            "conversations.create" => {
                let body = json!({
                    "type": params["type"].as_str().unwrap_or("nomi"),
                    "name": params["name"],
                    "extra": {},
                });
                let d = self.call(Method::POST, "/api/conversations", Some(body), None).await?;
                Ok(json!({"id": d["conversation_id"], "name": d["name"], "type": d["type"]}))
            }
            "conversations.send" => {
                let cid = str_param("conversation_id")?;
                let content = str_param("content")?;
                let body = json!({"content": content, "origin": "companion"});
                let d = self.call(Method::POST, &format!("/api/conversations/{cid}/messages"), Some(body), Some(rpc_id)).await?;
                let (text, truncated) = truncate_utf8(d["result_text"].as_str().unwrap_or(""), MAX_RESULT_BYTES);
                Ok(json!({"msg_id": d["msg_id"], "completed": d["completed"], "result_ok": d["result_ok"], "result_text": text, "truncated": truncated}))
            }
            "conversations.status" => {
                let cid = str_param("conversation_id")?;
                let d = self.call(Method::GET, &format!("/api/conversations/{cid}"), None, None).await?;
                Ok(json!({"id": d["conversation_id"], "name": d["name"], "status": d["status"], "runtime": d["runtime"]}))
            }
            "conversations.result" => {
                let cid = str_param("conversation_id")?;
                let d = self.call(Method::GET, &format!("/api/conversations/{cid}/messages?page_size=10&order=desc"), None, None).await?;
                let empty = vec![];
                let items = d["items"].as_array().unwrap_or(&empty);
                let last = items.iter().find(|m| m["type"] == "text" && m["position"] == "left");
                match last {
                    Some(m) => {
                        let (text, truncated) = truncate_utf8(&extract_text(&m["content"]), MAX_RESULT_BYTES);
                        Ok(json!({"message_id": m["message_id"], "text": text, "truncated": truncated, "created_at": m["created_at"]}))
                    }
                    None => Err(RpcErrorBody { code: "not_found".into(), message: "no assistant message yet".into() }),
                }
            }
            "conversations.cancel" => {
                let cid = str_param("conversation_id")?;
                self.call(Method::POST, &format!("/api/conversations/{cid}/cancel"), Some(json!({})), None).await?;
                Ok(json!({}))
            }
            "confirmations.list" => {
                let cid = str_param("conversation_id")?;
                let d = self.call(Method::GET, &format!("/api/conversations/{cid}/confirmations"), None, None).await?;
                Ok(json!({"items": if d.is_array() { d } else { d["items"].clone() }}))
            }
            "confirmations.confirm" => {
                let cid = str_param("conversation_id")?;
                let call_id = str_param("call_id")?;
                let body = json!({
                    "msg_id": str_param("msg_id")?,
                    "data": if params["data"].is_null() { json!({}) } else { params["data"].clone() },
                    "always_allow": params["always_allow"].as_bool().unwrap_or(false),
                });
                self.call(Method::POST, &format!("/api/conversations/{cid}/confirmations/{call_id}/confirm"), Some(body), None).await?;
                Ok(json!({}))
            }
            "cron.list" => {
                let d = self.call(Method::GET, "/api/cron/jobs", None, None).await?;
                Ok(json!({"items": if d.is_array() { d } else { d["items"].clone() }}))
            }
            "cron.create" => {
                let body = json!({
                    "name": str_param("name")?,
                    "schedule": params["schedule"],
                    "message": str_param("message")?,
                    "conversation_id": params["conversation_id"],
                    "agent_type": params["agent_type"].as_str().unwrap_or("nomi"),
                    "created_by": "user",
                });
                self.call(Method::POST, "/api/cron/jobs", Some(body), None).await
            }
            "cron.update" => {
                let id = str_param("cron_job_id")?;
                self.call(Method::PUT, &format!("/api/cron/jobs/{id}"), Some(params["patch"].clone()), None).await
            }
            "cron.delete" => {
                let id = str_param("cron_job_id")?;
                self.call(Method::DELETE, &format!("/api/cron/jobs/{id}"), None, None).await?;
                Ok(json!({}))
            }
            "cron.runNow" => {
                let id = str_param("cron_job_id")?;
                self.call(Method::POST, &format!("/api/cron/jobs/{id}/run"), Some(json!({})), None).await?;
                Ok(json!({}))
            }
            _ => Err(RpcErrorBody { code: "bad_request".into(), message: format!("unknown method: {method}") }),
        }
    }
}

pub fn extract_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn truncate_utf8(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}
```

- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge rpc`；Expected: 全部 PASS
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): rpc dispatcher over in-process router"`

### Task A8: BridgeCore（core.rs）

**Interfaces:**
- Consumes: `Identity`、`DeviceStore`、`PairingManager`、`RpcHandler`、`crypto::*`、`protocol::Inner`
- Produces:

```rust
pub struct BridgeCore { .. }
impl BridgeCore {
    pub fn new(identity: Identity, store: DeviceStore, pairing: PairingManager, rpc: RpcHandler, desktop_name: String) -> Arc<Self>;
    /// 处理一个入站 E2E 帧（JSON 文本）。回复经 reply 发送（E2EFrame 的 JSON 文本）。
    /// 返回 Some(device_id) 表示该连接已认证为此设备（供调用方登记连接）。
    pub async fn handle_frame_json(self: &Arc<Self>, text: &str, reply: &tokio::sync::mpsc::Sender<String>) -> Option<String>;
    pub fn register_conn(&self, device_id: &str, tx: mpsc::Sender<String>);
    pub fn unregister_conn(&self, device_id: &str);
    pub async fn broadcast_event(&self, name: &str, data: serde_json::Value); // 逐在线已配对设备 seal(Inner::Event) 发送
    pub fn identity(&self) -> &Identity; pub fn store(&self) -> &DeviceStore; pub fn pairing(&self) -> &PairingManager;
}
pub const MAX_DECRYPT_FAILURES: u32 = 10;
```

- [ ] **Step 1: 失败测试**（用 Task A7 的 stub router 组 RpcHandler；模拟手机侧直接用 crypto 函数收发）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{open, pair_mac, seal};
    use crate::identity::Identity;
    use crate::pairing::{DeviceRecord, DeviceStore, PairingManager};
    use crate::protocol::Inner;
    use serde_json::json;

    // stub() 与 rpc.rs 测试相同定义，复制一份（任务间不共享测试代码）
    // ... [同 Task A7 Step 1 的 stub()] ...

    fn core_with(dir: &std::path::Path) -> (std::sync::Arc<BridgeCore>, Identity /*desktop*/) {
        let desktop = Identity::from_secret_bytes([1u8; 32]);
        let store = DeviceStore::load(&dir.join("devices.json"));
        let pairing = PairingManager::new();
        let rpc = crate::rpc::RpcHandler::new(stub(), "s".into(), "Desk".into(), "0".into());
        let core = BridgeCore::new(desktop.clone(), store, pairing, rpc, "Desk".into());
        (core, desktop)
    }

    fn now_ms() -> i64 { 0 } // PairingManager 由测试直接调用 generate(now) 控制时间

    #[tokio::test]
    async fn full_pair_then_rpc_flow() {
        let dir = tempfile::tempdir().unwrap();
        let (core, desktop) = core_with(dir.path());
        let mobile = Identity::from_secret_bytes([2u8; 32]);
        let (code, _) = core.pairing().generate(now_ms());

        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        // 1) pair_request（帧带 pk）
        let req = seal(&json!({"kind":"pair_request","ctr":1,"code":code,"name":"Pixel","platform":"android"}), &mobile.device_id, Some(&mobile.pk), &desktop.pk, &mobile.sk);
        let authed = core.handle_frame_json(&serde_json::to_string(&req).unwrap(), &tx).await;
        assert_eq!(authed.as_deref(), Some(mobile.device_id.as_str()));
        // 2) 收到 pair_ok，mac 可验证
        let reply: crate::crypto::E2EFrame = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        let inner = open(&reply, &desktop.pk, &mobile.sk).unwrap();
        let m: Inner = serde_json::from_value(inner).unwrap();
        let Inner::PairOk { mac, .. } = m else { panic!("expected pair_ok, got {m:?}") };
        assert_eq!(mac, pair_mac(&code, &desktop.pk, &mobile.pk));
        // 3) 已配对设备发 rpc
        let rpc = seal(&json!({"kind":"rpc","ctr":2,"id":"r1","method":"device.info","params":{}}), &mobile.device_id, None, &desktop.pk, &mobile.sk);
        core.handle_frame_json(&serde_json::to_string(&rpc).unwrap(), &tx).await;
        let reply2: crate::crypto::E2EFrame = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        let inner2 = open(&reply2, &desktop.pk, &mobile.sk).unwrap();
        assert_eq!(inner2["kind"], "rpc_result");
        assert_eq!(inner2["ok"], true);
        assert_eq!(inner2["result"]["name"], "Desk");
    }

    #[tokio::test]
    async fn wrong_pair_code_gets_pair_err() {
        let dir = tempfile::tempdir().unwrap();
        let (core, desktop) = core_with(dir.path());
        let mobile = Identity::from_secret_bytes([2u8; 32]);
        core.pairing().generate(now_ms());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        let req = seal(&json!({"kind":"pair_request","ctr":1,"code":"00000000","name":"P","platform":"h5"}), &mobile.device_id, Some(&mobile.pk), &desktop.pk, &mobile.sk);
        let authed = core.handle_frame_json(&serde_json::to_string(&req).unwrap(), &tx).await;
        assert_eq!(authed, None);
        let reply: crate::crypto::E2EFrame = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        let inner = open(&reply, &desktop.pk, &mobile.sk).unwrap();
        assert_eq!(inner["kind"], "pair_err");
        assert_eq!(inner["code"], "pair_invalid_code");
    }

    #[tokio::test]
    async fn replayed_ctr_is_dropped_silently() {
        let dir = tempfile::tempdir().unwrap();
        let (core, desktop) = core_with(dir.path());
        let mobile = Identity::from_secret_bytes([2u8; 32]);
        core.store().insert(DeviceRecord { id: mobile.device_id.clone(), pk: crate::crypto::b64_encode(mobile.pk.as_bytes()), name: "P".into(), platform: "h5".into(), paired_at_ms: 0, last_ctr_in: 5, last_ctr_out: 1 });
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        let rpc = seal(&json!({"kind":"rpc","ctr":5,"id":"r1","method":"device.info","params":{}}), &mobile.device_id, None, &desktop.pk, &mobile.sk);
        core.handle_frame_json(&serde_json::to_string(&rpc).unwrap(), &tx).await;
        assert!(rx.try_recv().is_err()); // 静默丢弃
    }

    #[tokio::test]
    async fn unpaired_non_pair_frame_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (core, desktop) = core_with(dir.path());
        let mobile = Identity::from_secret_bytes([9u8; 32]);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        let rpc = seal(&json!({"kind":"rpc","ctr":1,"id":"r1","method":"device.info","params":{}}), &mobile.device_id, None, &desktop.pk, &mobile.sk);
        let authed = core.handle_frame_json(&serde_json::to_string(&rpc).unwrap(), &tx).await;
        assert_eq!(authed, None);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn broadcast_event_reaches_registered_conns() {
        let dir = tempfile::tempdir().unwrap();
        let (core, desktop) = core_with(dir.path());
        let mobile = Identity::from_secret_bytes([2u8; 32]);
        core.store().insert(DeviceRecord { id: mobile.device_id.clone(), pk: crate::crypto::b64_encode(mobile.pk.as_bytes()), name: "P".into(), platform: "h5".into(), paired_at_ms: 0, last_ctr_in: 0, last_ctr_out: 0 });
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        core.register_conn(&mobile.device_id, tx);
        core.broadcast_event("cron.executed", serde_json::json!({"cron_job_id":"j1","status":"ok","ts":1})).await;
        let f: crate::crypto::E2EFrame = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        let inner = open(&f, &desktop.pk, &mobile.sk).unwrap();
        assert_eq!(inner["kind"], "event");
        assert_eq!(inner["name"], "cron.executed");
    }
}
```

- [ ] **Step 2: 验证失败** — `cargo test -p nomifun-bridge core`
- [ ] **Step 3: 实现**（要点）
  - `handle_frame_json`：解析 E2EFrame；`store.get(from)` 命中 → 用存储 pk `open`，失败静默计数；`Inner::Rpc` 先 `store.accept_in_ctr`（失败静默丢弃），再 `rpc.dispatch`，结果 `seal(Inner::RpcResult{ctr: store.next_out_ctr(from)…})` 回 `reply`，并 `register_conn(from, reply.clone())`，返回 `Some(from)`。
  - 未知设备且 `frame.pk` 存在：解析 pk → `open`；`Inner::PairRequest` → `pairing.take_if_valid(code, now)`：成功 → `store.insert(DeviceRecord{last_ctr_in: ctr, last_ctr_out: 1})`、回 `PairOk{ctr:1, name, mac: pair_mac(...)}`、`register_conn`、返回 Some(id)；失败 → 回 `PairErr{ctr:1, code:"pair_invalid_code"}`（用请求帧 pk seal），返回 None。
  - 其余情况静默丢弃。`now` 取 `SystemTime` 毫秒（测试经由 `pairing().generate(now)` 控制过期语义，不需注入时钟）。
  - `broadcast_event`：遍历 conns（`Mutex<HashMap<String, mpsc::Sender<String>>>`），对每个已配对设备 `next_out_ctr` + seal `Inner::Event` 发送；发送失败则 `unregister_conn`。
- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge core`
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): core frame handler with pairing and rpc routing"`

### Task A9: 事件转发器（events.rs）

**Interfaces:**
- Consumes: `Arc<BroadcastEventBus>`（`nomifun_realtime::BroadcastEventBus`，`subscribe_user() -> Receiver<UserEventEnvelope{user_id, event: WebSocketMessage<Value>{name, data}}>`）、`Arc<BridgeCore>`
- Produces: `pub fn spawn_event_forwarder(bus: Arc<BroadcastEventBus>, core: Arc<BridgeCore>, shutdown: tokio_util::sync::CancellationToken) -> tokio::task::JoinHandle<()>`（若 tokio-util 不在依赖，用 `watch::Receiver<bool>` 作停机信号）

- [ ] **Step 1: 失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::websocket::WebSocketMessage;
    use nomifun_realtime::{BroadcastEventBus, UserEventSink};
    use serde_json::json;

    // core_with 同 Task A8 测试（stub router 含 messages GET，供 result_text 抓取）

    #[tokio::test]
    async fn turn_completed_becomes_task_completed_with_result_text() {
        let dir = tempfile::tempdir().unwrap();
        let (core, desktop) = core_with(dir.path());
        let mobile = crate::identity::Identity::from_secret_bytes([2u8; 32]);
        core.store().insert(/* DeviceRecord 同 A8 */);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        core.register_conn(&mobile.device_id, tx);

        let bus = std::sync::Arc::new(BroadcastEventBus::new(16));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let handle = spawn_event_forwarder(bus.clone(), core.clone(), stop_rx);
        tokio::task::yield_now().await;

        bus.send_to_user("u1", WebSocketMessage::new("turn.completed", json!({
            "conversation_id": "c1", "turn_id": "t1", "status": "finished",
            "runtime": {"pending_confirmations": 1}
        })));

        // 期望两帧：task.completed + conversations.attention
        let mut kinds = vec![];
        for _ in 0..2 {
            let f: crate::crypto::E2EFrame = serde_json::from_str(&tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap().unwrap()).unwrap();
            let inner = crate::crypto::open(&f, &desktop.pk, &mobile.sk).unwrap();
            kinds.push(inner["name"].as_str().unwrap().to_string());
            if inner["name"] == "task.completed" {
                assert_eq!(inner["data"]["conversation_id"], "c1");
                assert_eq!(inner["data"]["result_text"], "final answer"); // 来自 stub messages
            }
        }
        kinds.sort();
        assert_eq!(kinds, vec!["conversations.attention", "task.completed"]);
        let _ = stop_tx.send(true);
        handle.abort();
    }

    #[tokio::test]
    async fn message_stream_is_never_forwarded() {
        // 同上装配；发 message.stream 后短暂等待，断言 rx 无帧
        // bus.send_to_user("u1", WebSocketMessage::new("message.stream", json!({"x":1})));
        // tokio::time::sleep(100ms); assert!(rx.try_recv().is_err());
    }
}
```

（`message_stream_is_never_forwarded` 按注释写成完整测试。）

- [ ] **Step 2: 验证失败** — `cargo test -p nomifun-bridge events`
- [ ] **Step 3: 实现**（loop `select!` on `rx.recv()` / shutdown；match `event.name.as_str()`：`"turn.completed"` → 取 `data["conversation_id"]`，调 `core` 内部 rpc `conversations.result` 拿 text（错误则空串），`truncate_utf8(text, 2048*4)` 后按协议 §7 拼 `task.completed` data 并 `core.broadcast_event`；若 `data["runtime"]["pending_confirmations"].as_u64()>0` 再发 `conversations.attention`；`"cron.job-executed"` → `cron.executed`（加 `ts`）；其余（含 `message.stream`）一律忽略；`RecvError::Lagged` 继续循环）
- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge events`
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): result-only event forwarder"`

### Task A10: LAN 监听（lan.rs）

**Interfaces:**
- Produces:

```rust
pub const BRIDGE_LAN_PORT: u16 = 25810;
pub struct LanHandle { pub port: u16, shutdown: watch::Sender<bool>, task: JoinHandle<()> }
pub async fn start_lan(core: Arc<BridgeCore>, version: String, preferred_port: u16) -> anyhow::Result<LanHandle>;
impl LanHandle { pub async fn stop(self); }
```

路由：`GET /bridge/info`（JSON `{app:"nomifun",v:1,id,name,pk,version}` + `Access-Control-Allow-Origin: *`）；`GET /bridge/ws`（每连接：mpsc 出帧任务 + 入帧交 `core.handle_frame_json`；`{"type":"ping"}` 回 pong；断开时 `unregister_conn`）。绑定 `0.0.0.0:preferred_port`，`AddrInUse` 时退 `0.0.0.0:0`。

- [ ] **Step 1: 失败集成测试**（`tests/lan_ws.rs`；stub router 复制自 A7 测试；模拟手机端用 tokio-tungstenite + crate 自身 crypto 函数完成配对与 RPC）

```rust
// 关键断言：
// 1) GET /bridge/info 返回 200，含 id/pk，且响应头 access-control-allow-origin: *
// 2) ws 上完成 pair_request→pair_ok（校验 mac）
// 3) 随后 rpc device.info → rpc_result ok:true
// 4) {"type":"ping"} → {"type":"pong"}
// HTTP GET 用 Task B 同款裸 TCP helper 或直接 tokio-tungstenite 前的 http 请求（复制 reqwest_free_get）
```

（测试代码按上述断言完整写出：连接 `ws://127.0.0.1:{port}/bridge/ws`，逐帧 send/recv，用 `Identity::from_secret_bytes` 固定密钥。）

- [ ] **Step 2: 验证失败** — `cargo test -p nomifun-bridge --test lan_ws`
- [ ] **Step 3: 实现**（axum `Router::new().route("/bridge/info", get(info)).route("/bridge/ws", get(ws))`，state `(Arc<BridgeCore>, String)`；ws 循环与 Plan B session 类似但无注册握手/限速——不受信 LAN 客户端只能走 pair 流程，核心保护在 E2E 层）
- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge --test lan_ws`
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): lan listener with discovery probe and ws"`

### Task A11: 中继出站客户端（relay_client.rs）

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Serialize, Deserialize)] pub struct RelayConfig { pub url: String, pub key: String }
pub struct RelayHandle { shutdown: watch::Sender<bool>, task: JoinHandle<()>, pub connected: watch::Receiver<bool> }
pub fn start_relay(core: Arc<BridgeCore>, cfg: RelayConfig, initial_backoff_ms: u64) -> RelayHandle;
impl RelayHandle { pub async fn stop(self); }
```

行为：循环 `connect_async(url)` → 发 `RelayFrame::Register{role:"desktop", id, ts, auth: relay_auth(key,id,ts)}` → 等 `Registered`（置 connected=true）→ `select!`：入站 `Deliver{from,frame}` → `core.handle_frame_json`（reply sender 为“wrap 成 `Forward{to:from}` 后发往 relay”的适配 mpsc）；30s 定时发 `Ping`；断开 → connected=false，退避 `initial_backoff_ms*2^n` 封顶 60s 重连；shutdown 退出。

- [ ] **Step 1: 失败集成测试**（`tests/relay_client_test.rs`：测试内起一个**微型 stub relay**（axum ws，直接内联 ~60 行：注册表 HashMap + forward/deliver，逻辑同 Plan B 但无限速/超时），desktop 侧 `start_relay(initial_backoff_ms=50)`；手机侧用 tungstenite 客户端注册后发 forward(pair_request)，断言收到 deliver(pair_ok)、rpc 往返；再关停 stub relay 重启，断言 desktop 自动重连（再次收到 Register））
- [ ] **Step 2: 验证失败** — `cargo test -p nomifun-bridge --test relay_client_test`
- [ ] **Step 3: 实现**（如上行为描述；`ts` 取系统毫秒；注意 reply 适配器：`mpsc::channel<String>` + 转发任务把 E2EFrame 文本包成 `RelayFrame::Forward{to: from_device, frame: parse(text)}` 写 ws sink）
- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge --test relay_client_test -- --test-threads=2`
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): outbound relay client with reconnect backoff"`

### Task A12: BridgeService 生命周期（service.rs）

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Serialize, Deserialize, Default)] pub struct BridgeConfig { pub lan_enabled: bool, pub relay: Option<RelayConfig>, pub relay_enabled: bool }
#[derive(Clone, Serialize, Default)] #[serde(rename_all = "camelCase")] pub struct BridgeStatus { pub device_id: String, pub lan_running: bool, pub lan_port: Option<u16>, pub lan_ip: Option<String>, pub relay_enabled: bool, pub relay_connected: bool, pub paired_devices: Vec<PairedDeviceView>, pub error: Option<String> }
#[derive(Clone, Serialize)] #[serde(rename_all = "camelCase")] pub struct PairedDeviceView { pub id: String, pub name: String, pub platform: String, pub paired_at_ms: i64 }
#[derive(Clone, Serialize)] #[serde(rename_all = "camelCase")] pub struct PairingInfo { pub code: String, pub expires_at_ms: i64, pub qr_text: String, pub bridge_string: String }
pub struct BridgeService { .. }
impl BridgeService {
    pub fn new(router: axum::Router, trust_secret: String, event_bus: Arc<BroadcastEventBus>, data_dir: PathBuf, desktop_name: String, version: String) -> anyhow::Result<Arc<Self>>; // 读 identity/devices/config；按 config 自启 LAN/relay/event forwarder
    pub async fn start_lan(&self) -> BridgeStatus; pub async fn stop_lan(&self) -> BridgeStatus;
    pub async fn set_relay(&self, cfg: Option<RelayConfig>, enabled: bool) -> BridgeStatus; // 持久化 config.json 并重启 relay 任务
    pub fn status(&self) -> BridgeStatus; pub fn subscribe_status(&self) -> watch::Receiver<BridgeStatus>;
    pub fn generate_pairing(&self) -> PairingInfo; // qr_text = "nomifun-bridge:"+base64url(json §5，含 code)；bridge_string = 同 json 去 code
    pub fn list_devices(&self) -> Vec<PairedDeviceView>; pub async fn revoke_device(&self, id: &str) -> bool;
}
fn primary_lan_ip() -> Option<String> // UdpSocket connect 8.8.8.8:80 取 local_addr
```

- [ ] **Step 1: 失败测试**（单测：`new` 后 `status().device_id` 非空；`generate_pairing` 的 qr_text 以 `nomifun-bridge:` 开头且 base64url 解码后 JSON 含 id/pk/code，bridge_string 解码无 code；`start_lan`→status.lan_running=true→`stop_lan`→false；`set_relay(Some(cfg), true)` 持久化后重建 service 仍带配置；`revoke_device` 删除 store 记录）
- [ ] **Step 2: 验证失败** — `cargo test -p nomifun-bridge service`
- [ ] **Step 3: 实现**（base64url 用 `base64::engine::general_purpose::URL_SAFE_NO_PAD`；状态由内部 `refresh()` 汇总并 `status_tx.send`；relay connected 用 `RelayHandle.connected` watch 监听并转入状态）
- [ ] **Step 4: 验证通过** — `cargo test -p nomifun-bridge service`
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): bridge service lifecycle, config and pairing info"`

### Task A13: 接入 nomifun-app 与 Tauri 命令

**Files:**
- Modify: `crates/backend/nomifun-app/Cargo.toml`（dep `nomifun-bridge = { workspace = true }`）
- Modify: `crates/backend/nomifun-app/src/desktop.rs`（构建并持有 `bridge: Arc<BridgeService>`）
- Modify: `apps/desktop/src/main.rs`（Tauri 命令 + 注册）

- [ ] **Step 1: desktop.rs 装配**（在 `start_with_outcome` 中路由与 secret 就绪、`DesktopServer` 构造前后）

```rust
let bridge = nomifun_bridge::service::BridgeService::new(
    router.clone(),
    secret.to_string(),
    services.event_bus.clone(),
    data_dir.join("bridge"),
    hostname_or("NomiFun Desktop"),
    env!("CARGO_PKG_VERSION").to_string(),
)?;
```

`DesktopServer` 增加字段 `bridge: Arc<nomifun_bridge::service::BridgeService>` 与 `pub fn bridge(&self) -> Arc<BridgeService>`。`data_dir` 使用与 SQLite 相同的数据目录变量（在 `start` 内已有；`rg "data_dir" crates/backend/nomifun-app/src/desktop.rs` 定位）。`hostname_or`：读 `hostname::get()`? 若无 hostname 依赖则用 `whoami`/固定串——先 `rg hostname Cargo.toml`，无现成依赖就用 `std::env::var("HOSTNAME").unwrap_or_else(|_| "NomiFun Desktop".into())`。

- [ ] **Step 2: Tauri 命令**（`apps/desktop/src/main.rs`，仿 `webui_*`（main.rs:515–536），注册进 `generate_handler![...]`（~main.rs:2511））

```rust
#[tauri::command]
async fn bridge_get_status(server: tauri::State<'_, Arc<DesktopServer>>) -> Result<nomifun_bridge::service::BridgeStatus, String> {
    Ok(server.bridge().status())
}
#[tauri::command]
async fn bridge_start_lan(server: tauri::State<'_, Arc<DesktopServer>>) -> Result<nomifun_bridge::service::BridgeStatus, String> {
    Ok(server.bridge().start_lan().await)
}
#[tauri::command]
async fn bridge_stop_lan(server: tauri::State<'_, Arc<DesktopServer>>) -> Result<nomifun_bridge::service::BridgeStatus, String> {
    Ok(server.bridge().stop_lan().await)
}
#[tauri::command]
async fn bridge_set_relay(server: tauri::State<'_, Arc<DesktopServer>>, url: Option<String>, key: Option<String>, enabled: bool) -> Result<nomifun_bridge::service::BridgeStatus, String> {
    let cfg = match (url, key) { (Some(u), Some(k)) if !u.is_empty() => Some(nomifun_bridge::relay_client::RelayConfig { url: u, key: k }), _ => None };
    Ok(server.bridge().set_relay(cfg, enabled).await)
}
#[tauri::command]
async fn bridge_generate_pairing(server: tauri::State<'_, Arc<DesktopServer>>) -> Result<nomifun_bridge::service::PairingInfo, String> {
    Ok(server.bridge().generate_pairing())
}
#[tauri::command]
async fn bridge_revoke_device(server: tauri::State<'_, Arc<DesktopServer>>, device_id: String) -> Result<nomifun_bridge::service::BridgeStatus, String> {
    server.bridge().revoke_device(&device_id).await;
    Ok(server.bridge().status())
}
```

`apps/desktop/Cargo.toml` 加 `nomifun-bridge = { workspace = true }`。

- [ ] **Step 3: 验证** — Run: `cargo check --workspace 2>&1 | tail -20`；Expected: OK（修复所有编译错误）
- [ ] **Step 4: 针对性测试** — Run: `cargo test -p nomifun-bridge -- --test-threads=2`；Expected: 全 PASS
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): wire bridge service into desktop server and tauri commands"`

### Task A14: 设置 UI（BridgeControlPanel）+ i18n

**Files:**
- Modify: `ui/src/common/adapter/tauriShell.ts`（invoke 包装，仿 `tauriWebuiGetStatus` :282）
- Modify: `ui/src/common/adapter/ipcBridge.ts`（导出 `bridge = {...}`，仿 `webui`）
- Create: `ui/src/renderer/components/layout/Sider/BridgeControlPanel.tsx`（仿 `WebuiControlPanel.tsx`：Arco 组件、`useTranslation`、`qrcode.react` React.lazy）
- Modify: `ui/src/renderer/pages/openCapabilities/index.tsx`（在 WebuiControlPanel 之后渲染 BridgeControlPanel）
- Modify: i18n 资源（`rg -l '"webui' ui/src --glob '*.json'` 定位 zh-CN/en 文件，新增 `bridge.*` 键）

- [ ] **Step 1: adapter**（tauriShell.ts 增加 `tauriBridgeGetStatus/StartLan/StopLan/SetRelay/GeneratePairing/RevokeDevice` = `invoke('bridge_get_status')` 等；ipcBridge.ts 导出 `bridge` 对象）
- [ ] **Step 2: 面板**（功能块：LAN 开关（Switch → startLan/stopLan）＋显示 `lan_ip:port`；中继表单（url/key Input + 启用 Switch → setRelay）＋connected 徽标；“配对新设备”按钮 → Modal 展示 `QRCodeSVG value={qr_text}` + 配对码大字 + bridge_string 可复制文本（H5 兜底）；已配对设备 List + 吊销按钮（Popconfirm）；状态 5s 轮询或按钮后刷新）
- [ ] **Step 3: i18n**（zh-CN 与 en 各加：`bridge.title` 远程桥接/Remote Bridge、`bridge.lan` 局域网直连/LAN、`bridge.relay` 中继服务器/Relay、`bridge.pair` 配对新设备/Pair device、`bridge.devices` 已配对设备/Paired devices、`bridge.revoke` 吊销/Revoke、`bridge.code` 配对码/Pairing code、`bridge.bridgeString` 桥接串/Bridge string、`bridge.connected` 已连接/Connected、`bridge.disconnected` 未连接/Disconnected）
- [ ] **Step 4: 验证** — Run: `cat ui/package.json | grep -A20 '"scripts"'` 找到 typecheck/build 脚本并运行（如 `bun run --cwd ui typecheck && bun run --cwd ui build`）；Expected: 通过
- [ ] **Step 5: Commit** — `git commit -am "feat(bridge): settings panel with pairing qr and device management"`

### Task A15: 收尾验证

- [ ] **Step 1:** `cargo test -p nomifun-bridge -- --test-threads=2` 全 PASS
- [ ] **Step 2:** `cargo clippy -p nomifun-bridge -- -D warnings` 无告警
- [ ] **Step 3:** `cargo check --workspace` 通过；`ls .github/workflows 2>/dev/null` 为空
- [ ] **Step 4:** `git log --format='%an <%ae>' -20 | sort -u` 仅人类作者；提交信息无 AI trailer
