//! Application configuration parsed from CLI arguments + key derivation.

use std::path::PathBuf;
use std::sync::Arc;

use nomifun_auth::AuthPolicy;
use sha2::{Digest, Sha256};

/// Application configuration parsed from CLI arguments.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
    pub app_version: String,
    /// Authentication policy (single source of truth, replaces the old
    /// `local: bool`). Desktop = `TrustLocalToken`; standalone web = `Required`;
    /// `--insecure-no-auth` / `--local` = `NoAuth`.
    pub auth_policy: AuthPolicy,
    /// Per-boot secret the desktop's own webview presents to be trusted as the
    /// local client. Only `Some` under `AuthPolicy::TrustLocalToken`.
    pub local_trust_secret: Option<Arc<str>>,
}

impl AppConfig {
    /// Format as `host:port` for socket binding.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Path to the SQLite database file.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("nomifun-backend.db")
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: nomifun_common::constants::DEFAULT_HOST.to_string(),
            port: nomifun_common::constants::DEFAULT_PORT,
            data_dir: PathBuf::from("data"),
            work_dir: PathBuf::from("data"),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            auth_policy: AuthPolicy::Required,
            local_trust_secret: None,
        }
    }
}

/// Derive a 32-byte encryption key from the JWT secret using SHA-256.
pub fn derive_encryption_key(jwt_secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"nomifun-encryption-key:");
    hasher.update(jwt_secret.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 25808);
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert_eq!(config.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_app_config_socket_addr() {
        let config = AppConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
            ..Default::default()
        };
        assert_eq!(config.socket_addr(), "0.0.0.0:3000");
    }

    #[test]
    fn test_app_config_database_path() {
        let config = AppConfig {
            data_dir: PathBuf::from("/tmp/nomifun"),
            ..Default::default()
        };
        assert_eq!(config.database_path(), PathBuf::from("/tmp/nomifun/nomifun-backend.db"));
    }
}
