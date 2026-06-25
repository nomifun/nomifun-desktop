//! **P3-X2：secret 注册服务**（vault 管理 + 端点逻辑）。
//!
//! 把 [`crate::vault`] 的纯 save/load 包成一个 CRUD 服务，供 `/api/browser-secrets/*` 端点
//! （[`crate::routes`]）调用。职责：
//!
//! - 解析 vault 路径：用户决策（去 per-pet 键化）→ 浏览器身份**全局共享**，所有 pet_id 归一到**同一份**
//!   共享 vault `{data_dir}/browser-secrets/shared/secrets.json`（与「多宠物统一记忆」一致；pet_id 形参
//!   保留以兼容端点 URL，内部忽略）。
//! - `register`：load 现有 store → `register(name,value,allowed_origins)` → save 回盘（value 加密落
//!   vault，**绝不**回前端/LLM）。
//! - `list`：load → 返 [`SecretListItem`]（**仅** name + allowed_origins，**绝无** value）。
//! - `remove`：load → `remove(name)` → save。
//!
//! 服务持机器绑定 `encryption_key`（app data-dir 层 provision 的同一把 `[u8; 32]`）；每次操作 load→改→
//! save（无内存缓存——secret 操作低频，避免与会话侧 [`crate::SecretStore`] 的缓存不一致）。

use std::path::PathBuf;

use nomifun_api_types::SecretListItem;
use nomifun_common::AppError;

use crate::vault::{load_secret_store, pet_vault_path, save_secret_store};
use crate::{KEY_SIZE, SecretError};

/// per-pet 浏览器凭据 secret 服务。Clone-cheap（仅 PathBuf + key 拷贝）。
#[derive(Clone)]
pub struct SecretService {
    /// app 数据目录（共享 vault 挂在 `{data_dir}/browser-secrets/shared` 下）。
    data_dir: PathBuf,
    /// 机器绑定 AES-256-GCM key（app `encryption_key`，全后端同一把）。
    key: [u8; KEY_SIZE],
}

impl SecretService {
    /// 用 app 数据目录 + 机器绑定 key 构造。`key` 必须是 app data-dir 层 provision 的
    /// `encryption_key`（`derive_encryption_key`），与会话侧注入 [`crate::SecretStore`] 同一把，
    /// 否则注册的 secret 在会话里 GCM 认证失败（resolve None）。
    pub fn new(data_dir: PathBuf, key: [u8; KEY_SIZE]) -> Self {
        Self { data_dir, key }
    }

    /// 解析 vault 路径（委托共享 [`pet_vault_path`]——去 per-pet 键化后恒归一到共享单例
    /// `{data_dir}/browser-secrets/shared/secrets.json`，与会话侧 agent factory 构造 `BrowserSecretSource`
    /// 用**同一**份，故任一伙伴注册落盘与任何会话加载命中同一文件，凭据跨伙伴共享）。`pet_id` 形参保留
    /// 以兼容端点 URL，内部被忽略。
    pub fn vault_path_for(&self, pet_id: &str) -> PathBuf {
        pet_vault_path(&self.data_dir, pet_id)
    }

    /// 注册（或覆盖）一个 secret。value 加密落 vault（**绝不**回前端/LLM）。`allowed_origins` 空 / 无
    /// 可解析 eTLD+1 → [`AppError::BadRequest`]（绑定不可能匹配任何域，几乎必是调用方错误）。
    pub fn register(&self, pet_id: &str, name: &str, value: &str, allowed_origins: Vec<String>) -> Result<(), AppError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest("secret name must not be empty".into()));
        }
        if value.is_empty() {
            return Err(AppError::BadRequest("secret value must not be empty".into()));
        }
        let path = self.vault_path_for(pet_id);
        let mut store = load_secret_store(&path, self.key);
        store.register(name, value, allowed_origins).map_err(map_secret_err)?;
        save_secret_store(&store, &path)
            .map_err(|e| AppError::Internal(format!("persist secret vault failed: {e}")))?;
        Ok(())
    }

    /// 列出某 pet 已注册 secret 的**元数据**（name + allowed_origins，**绝无 value**）。
    pub fn list(&self, pet_id: &str) -> Vec<SecretListItem> {
        let store = load_secret_store(&self.vault_path_for(pet_id), self.key);
        store
            .list()
            .into_iter()
            .map(|l| SecretListItem {
                name: l.name,
                allowed_origins: l.allowed_etld1,
            })
            .collect()
    }

    /// 删除一个 secret。`Ok(true)` 删了、`Ok(false)` 本就不存在（幂等）。
    pub fn remove(&self, pet_id: &str, name: &str) -> Result<bool, AppError> {
        let path = self.vault_path_for(pet_id);
        let mut store = load_secret_store(&path, self.key);
        let removed = store.remove(name.trim());
        if removed {
            save_secret_store(&store, &path)
                .map_err(|e| AppError::Internal(format!("persist secret vault failed: {e}")))?;
        }
        Ok(removed)
    }
}

/// `SecretError` → `AppError`（注册期的策略/crypto 错；resolve 期是 fail-closed 的 `None`，不走这）。
fn map_secret_err(e: SecretError) -> AppError {
    match e {
        SecretError::InvalidAllowedOrigin(o) => AppError::BadRequest(format!(
            "invalid allowed origin '{o}': must be a host/origin with a registrable domain (eTLD+1)"
        )),
        SecretError::Crypto(m) => AppError::Internal(format!("secret crypto error: {m}")),
        SecretError::NotFound => AppError::NotFound("secret not found".into()),
        SecretError::OriginNotAllowed => {
            AppError::BadRequest("origin not allowed for this secret".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> (tempfile::TempDir, SecretService) {
        let dir = tempfile::tempdir().expect("tempdir");
        let svc = SecretService::new(dir.path().to_path_buf(), [0x42; KEY_SIZE]);
        (dir, svc)
    }

    #[test]
    fn register_then_list_excludes_value() {
        let (_d, svc) = service();
        svc.register("pet-1", "github", "ghp_supersecret", vec!["github.com".into()]).unwrap();
        let listed = svc.list("pet-1");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "github");
        assert_eq!(listed[0].allowed_origins, vec!["github.com".to_string()]);
        // **安全断言**：the listing's serialized form must never contain the value.
        let json = serde_json::to_string(&listed).unwrap();
        assert!(!json.contains("ghp_supersecret"), "list must NOT leak value: {json}");
    }

    #[test]
    fn register_persists_across_service_instances() {
        // 模拟「注册（端点） → 新会话 load」：同 data_dir + key 的新服务实例看得见已注册的 secret。
        let dir = tempfile::tempdir().expect("tempdir");
        let s1 = SecretService::new(dir.path().to_path_buf(), [0x42; KEY_SIZE]);
        s1.register("pet-1", "pw", "secret-val", vec!["x.com".into()]).unwrap();

        let s2 = SecretService::new(dir.path().to_path_buf(), [0x42; KEY_SIZE]);
        let listed = s2.list("pet-1");
        assert_eq!(listed.len(), 1, "registered secret must persist to disk across instances");
        // 会话侧的 SecretStore 从同一 vault 加载后，resolve 应取到真值（origin 门）。
        let store = load_secret_store(&s2.vault_path_for("pet-1"), [0x42; KEY_SIZE]);
        assert_eq!(store.resolve("pw", "https://x.com").unwrap().expose(), "secret-val");
    }

    #[test]
    fn all_pets_share_one_vault() {
        // 用户决策（去 per-pet 键化）：浏览器身份全局共享——不同 pet_id 注册的 secret 互见（同一份
        // 共享 vault）。推翻旧版「per-pet 隔离不串」断言。
        let (_d, svc) = service();
        svc.register("pet-1", "github", "ghp_a", vec!["x.com".into()]).unwrap();
        svc.register("pet-2", "stripe", "sk_b", vec!["y.com".into()]).unwrap();
        // 任一 pet_id 列出都看得见**全部**已注册 secret（共享单例）。
        let l1 = svc.list("pet-1");
        let l2 = svc.list("pet-2");
        assert_eq!(l1.len(), 2, "pet-1 must see both secrets (shared vault): {l1:?}");
        assert_eq!(l2.len(), 2, "pet-2 must see both secrets (shared vault): {l2:?}");
        // 甚至空/陌生 pet_id 也看见同一份。
        assert_eq!(svc.list("").len(), 2, "empty pet_id routes to the same shared vault");
    }

    #[test]
    fn remove_is_idempotent() {
        let (_d, svc) = service();
        svc.register("pet-1", "pw", "v", vec!["x.com".into()]).unwrap();
        assert!(svc.remove("pet-1", "pw").unwrap(), "first remove deletes");
        assert!(!svc.remove("pet-1", "pw").unwrap(), "second remove is a no-op (idempotent)");
        assert!(svc.list("pet-1").is_empty());
    }

    #[test]
    fn register_rejects_empty_value_and_name() {
        let (_d, svc) = service();
        assert!(matches!(svc.register("p", "", "v", vec!["x.com".into()]), Err(AppError::BadRequest(_))));
        assert!(matches!(svc.register("p", "n", "", vec!["x.com".into()]), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn register_rejects_unparseable_origin() {
        let (_d, svc) = service();
        // bare public suffix → no eTLD+1 → 400.
        assert!(matches!(
            svc.register("p", "n", "v", vec!["co.uk".into()]),
            Err(AppError::BadRequest(_))
        ));
        // empty allowed_origins → 400.
        assert!(matches!(svc.register("p", "n", "v", vec![]), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn any_pet_id_routes_to_the_shared_vault() {
        // 用户决策（去 per-pet 键化）：任意 pet_id（空 / 含分隔符 / companion / conversation）都解析到
        // **同一份**共享 vault `{data_dir}/browser-secrets/shared/secrets.json`。
        let (_d, svc) = service();
        let tail = std::path::Path::new("browser-secrets").join("shared").join("secrets.json");
        for id in ["", "../../etc", "conversation:5", "pet-x"] {
            assert!(svc.vault_path_for(id).ends_with(&tail), "pet_id {id:?} must route to shared vault");
        }
        // 不同 id 解析同一路径（共享单例硬证据）。
        assert_eq!(svc.vault_path_for("a"), svc.vault_path_for("b"));
    }
}
