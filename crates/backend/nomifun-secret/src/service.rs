//! **P3-X2：secret 注册服务**（vault 管理 + 端点逻辑）。
//!
//! 把 [`crate::vault`] 的纯 save/load 包成一个 CRUD 服务，供 `/api/browser-secrets/*` 端点
//! （[`crate::routes`]）调用。职责：
//!
//! - 解析 vault 路径：浏览器身份**全局共享**，始终使用唯一的
//!   `{data_dir}/browser-secrets/shared/secrets.json`。
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

use crate::vault::{load_secret_store, save_secret_store, shared_vault_path};
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

    /// 唯一共享 vault 路径；端点不接受无意义的伙伴 ID。
    pub fn vault_path(&self) -> PathBuf {
        shared_vault_path(&self.data_dir)
    }

    /// 注册（或覆盖）一个 secret。value 加密落 vault（**绝不**回前端/LLM）。`allowed_origins` 空 / 无
    /// 可解析 eTLD+1 → [`AppError::BadRequest`]（绑定不可能匹配任何域，几乎必是调用方错误）。
    pub fn register(&self, name: &str, value: &str, allowed_origins: Vec<String>) -> Result<(), AppError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest("secret name must not be empty".into()));
        }
        if value.is_empty() {
            return Err(AppError::BadRequest("secret value must not be empty".into()));
        }
        let path = self.vault_path();
        let mut store = load_secret_store(&path, self.key);
        store.register(name, value, allowed_origins).map_err(map_secret_err)?;
        save_secret_store(&store, &path)
            .map_err(|e| AppError::Internal(format!("persist secret vault failed: {e}")))?;
        Ok(())
    }

    /// 列出某 pet 已注册 secret 的**元数据**（name + allowed_origins，**绝无 value**）。
    pub fn list(&self) -> Vec<SecretListItem> {
        let store = load_secret_store(&self.vault_path(), self.key);
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
    pub fn remove(&self, name: &str) -> Result<bool, AppError> {
        let path = self.vault_path();
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
        svc.register("github", "ghp_supersecret", vec!["github.com".into()]).unwrap();
        let listed = svc.list();
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
        s1.register("pw", "secret-val", vec!["x.com".into()]).unwrap();

        let s2 = SecretService::new(dir.path().to_path_buf(), [0x42; KEY_SIZE]);
        let listed = s2.list();
        assert_eq!(listed.len(), 1, "registered secret must persist to disk across instances");
        // 会话侧的 SecretStore 从同一 vault 加载后，resolve 应取到真值（origin 门）。
        let store = load_secret_store(&s2.vault_path(), [0x42; KEY_SIZE]);
        assert_eq!(store.resolve("pw", "https://x.com").unwrap().expose(), "secret-val");
    }

    #[test]
    fn registrations_share_one_global_vault() {
        let (_d, svc) = service();
        svc.register("github", "ghp_a", vec!["x.com".into()]).unwrap();
        svc.register("stripe", "sk_b", vec!["y.com".into()]).unwrap();
        assert_eq!(svc.list().len(), 2);
    }

    #[test]
    fn remove_is_idempotent() {
        let (_d, svc) = service();
        svc.register("pw", "v", vec!["x.com".into()]).unwrap();
        assert!(svc.remove("pw").unwrap(), "first remove deletes");
        assert!(!svc.remove("pw").unwrap(), "second remove is a no-op (idempotent)");
        assert!(svc.list().is_empty());
    }

    #[test]
    fn register_rejects_empty_value_and_name() {
        let (_d, svc) = service();
        assert!(matches!(svc.register("", "v", vec!["x.com".into()]), Err(AppError::BadRequest(_))));
        assert!(matches!(svc.register("n", "", vec!["x.com".into()]), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn register_rejects_unparseable_origin() {
        let (_d, svc) = service();
        // bare public suffix → no eTLD+1 → 400.
        assert!(matches!(
            svc.register("n", "v", vec!["co.uk".into()]),
            Err(AppError::BadRequest(_))
        ));
        // empty allowed_origins → 400.
        assert!(matches!(svc.register("n", "v", vec![]), Err(AppError::BadRequest(_))));
    }
}
