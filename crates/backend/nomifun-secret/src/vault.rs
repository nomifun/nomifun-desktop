//! **P3-X2：secret vault 落盘持久化（per-pet）**（裁决⑦ / 收编 P2 X2）。
//!
//! E1 的 [`SecretStore`](crate::SecretStore) 是**纯内存** `HashMap`——注册的凭据进程退出即丢。X2 在
//! 其上塞一层**磁盘 vault**，使注册的 secret **跨会话/重启**仍可用（`secret:NAME` 不再因空 store 恒
//! fail-closed）：
//!
//! ```text
//!   注册：register_secret 端点 → SecretStore.register → save_secret_store(vault)  [本模块]
//!                                                          ↓ 磁盘
//!   用：会话起 → load_secret_store(vault, key)  [本模块] → 注入 BrowserTool
//!                                                → secret:NAME 经 origin 门解析
//! ```
//!
//! ## 为什么**不**再加一层加密（已是密文）
//! [`SecretStore`](crate::SecretStore) 的每条记录 `value` **本就是 AES-256-GCM 密文**
//! （[`register`](crate::SecretStore::register) 调 [`nomifun_common::encrypt_string`] 用机器绑定 key 加密）。
//! 故 vault 文件直接落「记录数组」JSON 即可——**文件内绝无明文**（值是密文，`name`/`allowed_etld1` 是
//! 策略非凭据）。再套一层文件级 AES 是**双重加密**、无安全增益、只增复杂度，故**不做**（裁决⑦：单一
//! AES 栈，per-record 那一层即是）。机器绑定 key（app data-dir 层 `encryption_key`，全后端 `[u8; 32]`
//! 一路穿透的同一把）→ vault 文件即便被拷走，换台机器 [`load_secret_store`] 后 `resolve` 的 GCM 认证
//! 仍失败 → fail-closed 返 `None`（永不泄值）。
//!
//! ## 浏览器身份全局共享（用户决策：去 per-pet 隔离）
//! vault 落**单一共享** [`SHARED_SECRET_DIR`] 子目录（`{data_dir}/browser-secrets/shared/secrets.json`）
//! ——所有桌面伙伴 + 会话用**同一份**凭据保险库（与「多宠物统一记忆」一致），任一伙伴注册的 secret 在
//! 任何会话/伙伴里共享可见。[`pet_vault_path`] 保留 `pet_id` 形参以兼容调用方签名，但**内部忽略**它恒
//! 归一到 [`shared_vault_path`]。（历史 per-pet 隔离布局已退役；W4 引擎层 per-pet context 机制与此 vault
//! 键无关，仍保留休眠。）
//!
//! ## 优雅降级（绝不 panic）
//! [`load_secret_store`] 对**任何**读取/解析失败都返**空 store**（vault 不存在 = 首次、JSON 损坏 = 部分
//! 写入/坏块、形态变了）——secret 是**增强**，vault 坏了应静默退回「无注册凭据」起点（`secret:NAME`
//! 恒 fail-closed，用户重新注册即可），绝不让一个坏 vault 文件阻断引擎启动。密文损坏/换 key 不在此处
//! 暴露——它们在 `resolve` 时 GCM 认证失败成 `None`（fail-closed），同样不致命。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{KEY_SIZE, SecretRecord, SecretStore};

/// secret vault 文件名。落「已加密记录」的 JSON（值是 per-record AES 密文，故文件本身**不**含明文
/// ——见模块 doc「为什么不再加密」）。
pub const SECRET_VAULT_FILE: &str = "secrets.json";

/// secret vault 根目录名（在 `data_dir` 下）。去 per-pet 键化后：`{data_dir}/browser-secrets/shared/
/// secrets.json`（单一共享 vault）。
pub const SECRETS_ROOT: &str = "browser-secrets";

/// vault 文件的 on-disk 形态（versioned，forward-compat）。`secrets` 的每个 value 的 `ciphertext`
/// 本就是 AES 密文 → 文件无明文。
#[derive(Debug, Serialize, Deserialize)]
pub struct SecretVaultFile {
    /// 形态版本（当前 1）。未来若改格式可据此迁移；不认得的版本 → [`load_secret_store`] 退回空 store。
    pub version: u32,
    /// name → 已加密记录（ciphertext + allowed_etld1）。
    pub secrets: HashMap<String, SecretRecord>,
}

const VAULT_VERSION: u32 = 1;

/// 解析某目录下的 secret vault 路径 `<dir>/secrets.json`（纯 join，无 I/O）。被 [`shared_vault_path`]
/// 用于在 `{data_dir}/browser-secrets/shared` 下拼出 vault 文件名。
pub fn secret_vault_path(dir: &Path) -> PathBuf {
    dir.join(SECRET_VAULT_FILE)
}

/// 共享 secret vault 的子目录名（在 `browser-secrets` 根下）。用户决策：浏览器身份**全局共享**——所有
/// 桌面伙伴 + 会话用**同一份**凭据保险库（与「多宠物统一记忆」一致），不再 per-pet 隔离。落
/// `{data_dir}/browser-secrets/shared/secrets.json`。
pub const SHARED_SECRET_DIR: &str = "shared";

/// **单一权威：解析共享 secret vault 的完整路径** `{data_dir}/browser-secrets/shared/secrets.json`。
///
/// 用户决策（去 per-pet 键化）：浏览器身份全局共享——**所有伙伴/会话注册与解析走同一份 vault**，凭据
/// 跨伙伴互见。这是 [`pet_vault_path`] 内部归一到的目标路径。
///
/// **两端必须共用此份**：端点侧（`SecretService`，注册落盘）与会话侧（agent factory 构造
/// `BrowserSecretSource`，加载）+ 网关 registry——全部命中同一文件，故任一伙伴注册的 secret 在任何会话/
/// 伙伴里都看得见。
pub fn shared_vault_path(data_dir: &Path) -> PathBuf {
    secret_vault_path(&data_dir.join(SECRETS_ROOT).join(SHARED_SECRET_DIR))
}

/// **解析 secret vault 路径**——历史上 per-pet 键化（`pet_id` 段），现归一到[共享单例](shared_vault_path)。
///
/// 用户决策：浏览器身份全局共享——`pet_id` 形参**保留以兼容现有调用方签名**（端点 URL/factory key/网关
/// key 仍照传），但**内部忽略**它，恒路由到 [`shared_vault_path`]（`{data_dir}/browser-secrets/shared/
/// secrets.json`）。故任一伙伴注册的 secret 在所有会话/伙伴里共享可见——这是「共享」的落点。
///
/// （per-pet 隔离的目录布局已退役；W4a 引擎层 per-pet context 机制仍保留休眠，与此 vault 键无关。）
pub fn pet_vault_path(data_dir: &Path, _pet_id: &str) -> PathBuf {
    // 用户决策：去 per-pet 键化，所有调用方归一到共享单例（pet_id 被忽略，仅保签名兼容）。
    shared_vault_path(data_dir)
}

/// **把 [`SecretStore`] 持久化到磁盘 vault**（注册/删除后的「存」侧）。
///
/// 序列化 store 的（已加密）记录 → JSON → 写 `vault_path`。父目录不存在则 best-effort 建（per-pet 目录
/// 可能首次落 vault）。失败 → `Err`（**绝不 panic**）；调用方（端点）把它转成 5xx 即可。**绝不解密**——
/// 记录全程是密文。
pub fn save_secret_store(store: &SecretStore, vault_path: &Path) -> std::io::Result<()> {
    let file = SecretVaultFile {
        version: VAULT_VERSION,
        secrets: store.to_records(),
    };
    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(vault_path, json.as_bytes())
}

/// **从磁盘 vault 读出 [`SecretStore`]**（「取」侧），绑机器绑定 `key`。
///
/// 读 `vault_path` → 解析 JSON 记录 → [`SecretStore::from_records`]（**不解密**，记录是密文；真正的
/// 解密 + origin 门发生在后续 `resolve`，换机/换 key → GCM 认证失败 → `None`，fail-closed）。
///
/// **任何失败都返「空 store」（绝不 panic / 绝不 `Err`）**——secret 是增强：
/// - vault 不存在（首次注册前）→ 空 store（连 warn 都不必）；
/// - 读文件 I/O 失败 → 空 store（warn 留痕）；
/// - JSON 损坏 / 版本不认得（部分写入/坏块/旧格式）→ 空 store（warn 留痕）。
///
/// 这把「坏 vault 阻断启动」彻底消除：最坏情况是丢注册凭据（`secret:NAME` fail-closed），不是崩。
pub fn load_secret_store(vault_path: &Path, key: [u8; KEY_SIZE]) -> SecretStore {
    let json = match std::fs::read_to_string(vault_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SecretStore::new(key),
        Err(e) => {
            tracing::warn!(
                target: "nomifun_secret::vault",
                error = %e, path = %vault_path.display(),
                "read secret vault failed; starting with an empty store (no persisted credentials)"
            );
            return SecretStore::new(key);
        }
    };
    let file: SecretVaultFile = match serde_json::from_str(&json) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                target: "nomifun_secret::vault",
                error = %e, path = %vault_path.display(),
                "parse secret vault JSON failed (corrupt or old format); starting with an empty store"
            );
            return SecretStore::new(key);
        }
    };
    if file.version != VAULT_VERSION {
        tracing::warn!(
            target: "nomifun_secret::vault",
            found = file.version, expected = VAULT_VERSION, path = %vault_path.display(),
            "secret vault version mismatch; starting with an empty store"
        );
        return SecretStore::new(key);
    }
    SecretStore::from_records(key, file.secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_SIZE] = [0x42; KEY_SIZE];

    #[test]
    fn secret_vault_path_joins_filename() {
        // secret_vault_path 是纯 join：<dir>/secrets.json（被 shared_vault_path 用于拼共享 vault）。
        let dir = Path::new("/data/browser-secrets/shared");
        let p = secret_vault_path(dir);
        assert_eq!(p, Path::new("/data/browser-secrets/shared/secrets.json"));
        assert!(p.starts_with(dir), "vault file must live under the given dir");
    }

    #[test]
    fn pet_vault_path_routes_to_shared_singleton() {
        // 用户决策（去 per-pet 键化）：任意 pet_id 都归一到**同一份**共享 vault
        // `{data_dir}/browser-secrets/shared/secrets.json`——浏览器身份全局共享。
        let data = Path::new("/data");
        let shared_tail = Path::new("browser-secrets").join("shared").join("secrets.json");
        // 任意 pet_id（companion / conversation / 空 / 含分隔符）都落同一份共享 vault。
        for id in ["companion-1", "conversation:5", "  ", "../../etc", ""] {
            let p = pet_vault_path(data, id);
            assert!(p.ends_with(&shared_tail), "pet_id {id:?} must route to the shared vault, got {p:?}");
        }
        // 共享单例：不同「pet」解析出**同一**路径（这是「共享」的硬证据，对比旧版 per-pet 互异）。
        assert_eq!(pet_vault_path(data, "pet-a"), pet_vault_path(data, "pet-b"));
        assert_eq!(pet_vault_path(data, "pet-a"), shared_vault_path(data));
    }

    #[test]
    fn multiple_pets_share_one_store_credentials_visible_across() {
        // **共享证据（纯逻辑）**：伙伴 A 在「自己的」pet_id 下注册 secret → 伙伴 B 用「另一个」pet_id
        // 解析时（同 data_dir）命中同一共享 vault → 看得见 A 注册的凭据（凭据跨伙伴共享）。
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path();

        // 伙伴 A 注册（端点侧落盘到「companion-A」键——内部归一到共享）。
        let path_a = pet_vault_path(data, "companion-A");
        let mut store_a = load_secret_store(&path_a, KEY);
        store_a.register("pw", "shared-login-secret", vec!["x.com".into()]).unwrap();
        save_secret_store(&store_a, &path_a).expect("save A");

        // 伙伴 B（不同 pet_id）加载 → 看得见 A 的凭据（共享单例，互见）。
        let path_b = pet_vault_path(data, "companion-B");
        assert_eq!(path_a, path_b, "去 per-pet 键化：两伙伴解析同一共享 vault 文件");
        let store_b = load_secret_store(&path_b, KEY);
        assert_eq!(
            store_b.resolve("pw", "https://x.com").unwrap().expose(),
            "shared-login-secret",
            "credential registered by A must be visible to B (shared identity)"
        );
        // 安全不变：origin 门仍 fail-closed（非绑定域 → None）。
        assert!(store_b.resolve("pw", "https://evil.com").is_none(), "origin gate still fail-closed");
    }

    #[test]
    fn save_then_load_round_trips_and_resolves() {
        // **核心：注册 → 落盘 → 重载 → resolve 往返**（机器绑定 key 加密往返，跨「会话」保真）。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = secret_vault_path(dir.path());

        let mut store = SecretStore::new(KEY);
        store.register("pw", "the-real-password", vec!["x.com".into()]).unwrap();
        store.register("token", "ghp_abc", vec!["github.com".into(), "https://api.github.com".into()]).unwrap();
        save_secret_store(&store, &path).expect("save");

        // 新 store（模拟新会话/重启）从 vault 重载 —— resolve 仍按 origin 门解析。
        let reloaded = load_secret_store(&path, KEY);
        assert_eq!(reloaded.resolve("pw", "https://login.x.com").unwrap().expose(), "the-real-password");
        assert_eq!(reloaded.resolve("token", "https://api.github.com").unwrap().expose(), "ghp_abc");
        // origin 门仍 fail-closed（非绑定域 → None）。
        assert!(reloaded.resolve("pw", "https://evil.com").is_none());
        // list 不含 value（重载后元数据保真）。
        assert_eq!(reloaded.list().len(), 2);
    }

    #[test]
    fn vault_file_is_ciphertext_not_plaintext() {
        // **加密验收：落盘内容不含明文 value**（值是 per-record AES 密文；文件无明文）。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = secret_vault_path(dir.path());
        let mut store = SecretStore::new(KEY);
        store.register("pw", "deadbeef-secret-token", vec!["x.com".into()]).unwrap();
        save_secret_store(&store, &path).expect("save");

        let on_disk = std::fs::read_to_string(&path).expect("read raw vault");
        assert!(!on_disk.contains("deadbeef-secret-token"), "value must NOT be plaintext on disk: {on_disk}");
        // name / allowed_etld1 是策略（非凭据），可明文（用于 list/firewall）；这里只断言 value 不明文。
        assert!(on_disk.contains("x.com"), "policy (allowed_etld1) may be plaintext on disk");
    }

    #[test]
    fn load_with_wrong_key_resolves_to_none_fail_closed() {
        // 换机/换 key → 重载本身不报错（记录是密文，from_records 不解密），但 resolve 的 GCM 认证失败
        // → None（fail-closed；vault 拷走也解不开）。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = secret_vault_path(dir.path());
        let mut store = SecretStore::new([0x01; KEY_SIZE]);
        store.register("pw", "v", vec!["x.com".into()]).unwrap();
        save_secret_store(&store, &path).expect("save");

        let wrong = load_secret_store(&path, [0x02; KEY_SIZE]);
        assert!(wrong.resolve("pw", "https://x.com").is_none(), "wrong key must fail-closed to None");
        // 对的 key 仍解得回（证明只是 key 不对，非 vault 坏）。
        let right = load_secret_store(&path, [0x01; KEY_SIZE]);
        assert_eq!(right.resolve("pw", "https://x.com").unwrap().expose(), "v");
    }

    #[test]
    fn load_missing_vault_is_empty_store_not_panic() {
        // 优雅：vault 不存在（首次注册前）→ 空 store（绝不 panic）。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = secret_vault_path(dir.path()); // 没 save 过
        let s = load_secret_store(&path, KEY);
        assert!(s.is_empty(), "missing vault must load to an empty store");
    }

    #[test]
    fn load_corrupt_json_is_empty_store_not_panic() {
        // 优雅：JSON 损坏（部分写入/坏块）→ 空 store（绝不 panic）。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = secret_vault_path(dir.path());
        std::fs::write(&path, "{not valid json!!!").expect("write garbage");
        assert!(load_secret_store(&path, KEY).is_empty(), "corrupt vault must degrade to empty store");
    }

    #[test]
    fn load_unknown_version_is_empty_store() {
        // 优雅：版本不认得（旧/未来格式）→ 空 store（绝不 panic）。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = secret_vault_path(dir.path());
        std::fs::write(&path, r#"{"version":999,"secrets":{}}"#).expect("write");
        assert!(load_secret_store(&path, KEY).is_empty(), "unknown version must degrade to empty store");
    }

    #[test]
    fn save_creates_parent_dir() {
        // per-pet 目录可能首次落 vault（父目录还没建）→ save best-effort 建父目录。
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("browser-secrets").join("companion-new");
        let path = secret_vault_path(&nested);
        assert!(!nested.exists(), "precondition: parent not yet created");
        let mut store = SecretStore::new(KEY);
        store.register("pw", "v", vec!["x.com".into()]).unwrap();
        save_secret_store(&store, &path).expect("save into not-yet-existing dir");
        assert!(path.exists(), "save must create parent dirs");
        assert_eq!(load_secret_store(&path, KEY).resolve("pw", "https://x.com").unwrap().expose(), "v");
    }

    #[test]
    fn save_empty_store_round_trips() {
        // 空 store（删光后）也能存取（登出/全删后落盘）。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = secret_vault_path(dir.path());
        let store = SecretStore::new(KEY);
        save_secret_store(&store, &path).expect("save empty");
        assert!(load_secret_store(&path, KEY).is_empty());
    }
}
