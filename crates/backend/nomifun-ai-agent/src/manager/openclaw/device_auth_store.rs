use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthEntry {
    pub token: String,
    pub role: String,
    pub scopes: Vec<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceAuthStore {
    version: u32,
    device_id: String,
    tokens: HashMap<String, DeviceAuthEntry>,
}

fn store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openclaw")
        .join("identity")
        .join("device-auth.json")
}

pub fn load_device_auth_token(device_id: &str, role: &str) -> Option<DeviceAuthEntry> {
    let path = store_path();
    let content = fs::read_to_string(&path).ok()?;
    let store: DeviceAuthStore = serde_json::from_str(&content).ok()?;

    if store.version != 1 || store.device_id != device_id {
        return None;
    }

    store.tokens.get(role).cloned()
}

pub fn store_device_auth_token(device_id: &str, role: &str, token: &str, scopes: &[String]) {
    let path = store_path();

    let mut store = fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str::<DeviceAuthStore>(&c).ok())
        .filter(|s| s.version == 1 && s.device_id == device_id)
        .unwrap_or_else(|| DeviceAuthStore {
            version: 1,
            device_id: device_id.to_owned(),
            tokens: HashMap::new(),
        });

    let mut sorted_scopes: Vec<String> = scopes.to_vec();
    sorted_scopes.sort();
    sorted_scopes.dedup();

    store.tokens.insert(
        role.to_owned(),
        DeviceAuthEntry {
            token: token.to_owned(),
            role: role.to_owned(),
            scopes: sorted_scopes,
            updated_at_ms: nomifun_common::now_ms(),
        },
    );

    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        warn!(error = %e, "Failed to create device auth store directory");
        return;
    }

    match serde_json::to_string_pretty(&store) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, format!("{json}\n")) {
                warn!(error = %e, "Failed to write device auth store");
                return;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
            }
            debug!(role = role, "Stored device auth token");
        }
        Err(e) => warn!(error = %e, "Failed to serialize device auth store"),
    }
}

pub fn clear_device_auth_token(device_id: &str, role: &str) {
    let path = store_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut store: DeviceAuthStore = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(_) => return,
    };

    if store.version != 1 || store.device_id != device_id {
        return;
    }

    if store.tokens.remove(role).is_some()
        && let Ok(json) = serde_json::to_string_pretty(&store)
    {
        let _ = fs::write(&path, format!("{json}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_store_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device-auth.json");

        let store = DeviceAuthStore {
            version: 1,
            device_id: "dev-123".into(),
            tokens: HashMap::from([(
                "operator".into(),
                DeviceAuthEntry {
                    token: "tok-abc".into(),
                    role: "operator".into(),
                    scopes: vec!["admin".into()],
                    updated_at_ms: 1700000000000,
                },
            )]),
        };

        let json = serde_json::to_string_pretty(&store).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let loaded: DeviceAuthStore = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.device_id, "dev-123");
        assert_eq!(loaded.tokens["operator"].token, "tok-abc");
    }

    #[test]
    fn scopes_sorted_and_deduped() {
        let mut scopes: Vec<String> = vec!["z".into(), "a".into(), "z".into(), "m".into()];
        scopes.sort();
        scopes.dedup();
        assert_eq!(scopes, vec!["a", "m", "z"]);
    }

    #[test]
    fn version_mismatch_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device-auth.json");

        let store = DeviceAuthStore {
            version: 99,
            device_id: "dev-123".into(),
            tokens: HashMap::new(),
        };
        let json = serde_json::to_string(&store).unwrap();
        fs::write(&path, &json).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let loaded: DeviceAuthStore = serde_json::from_str(&content).unwrap();
        assert_ne!(loaded.version, 1);
    }
}
