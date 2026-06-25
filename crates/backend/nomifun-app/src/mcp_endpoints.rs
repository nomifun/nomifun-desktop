//! Dynamic MCP endpoint beacon. The backend rewrites `<data_dir>/mcp-endpoints.json`
//! (0600) on every boot with the CURRENT per-process {port, token} of each
//! in-process MCP server. stdio bridges (and externally-registered CLIs) read it
//! at runtime so a one-time registration survives backend restarts WITHOUT baking
//! a stale port/token into the user's CLI config.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpEndpoints {
    #[serde(default)]
    pub knowledge: Option<Endpoint>,
    #[serde(default)]
    pub requirement: Option<Endpoint>,
    #[serde(default)]
    pub lifecycle: Option<Endpoint>,
}

impl McpEndpoints {
    pub fn knowledge_then_token(&self) -> Option<String> {
        self.knowledge.as_ref().map(|e| e.token.clone())
    }
}

pub const BEACON_FILE: &str = "mcp-endpoints.json";

pub fn beacon_path(data_dir: &Path) -> PathBuf {
    data_dir.join(BEACON_FILE)
}

/// Write the beacon atomically with 0600 perms (owner-only — it holds tokens).
pub fn write_beacon(data_dir: &Path, eps: &McpEndpoints) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = beacon_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600) // restrictive at creation — no world-readable window
            .open(&tmp)?;
        f.write_all(
            &serde_json::to_vec_pretty(eps)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        )?;
        f.sync_all()?; // durable before rename
    }
    #[cfg(not(unix))]
    {
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(eps)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        )?;
    }
    std::fs::rename(&tmp, &path)?; // atomic replace; final file keeps 0600
    Ok(())
}

pub fn read_beacon_at(path: &Path) -> std::io::Result<McpEndpoints> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Resolve + read the beacon from a bridge process (no host env required for
/// external terminals). Honors `NOMI_MCP_ENDPOINTS_FILE` override, else computes
/// the standard data-dir path (same resolver the backend uses, NOMI_CHANNEL-aware).
///
/// Priority:
/// 1. `NOMI_MCP_ENDPOINTS_FILE` env -> read that exact path.
/// 2. `NOMIFUN_DATA_DIR` env -> `<that>/mcp-endpoints.json`.
/// 3. Platform default (`cli::default_data_dir()`) -> `<that>/mcp-endpoints.json`.
///
/// Returns `None` if the beacon cannot be found or parsed (the caller should
/// fall back to legacy env vars).
pub fn read_beacon_for_bridge() -> Option<McpEndpoints> {
    // Priority 1: explicit beacon file path (set by internal terminal spawn).
    if let Ok(p) = std::env::var("NOMI_MCP_ENDPOINTS_FILE") {
        return read_beacon_at(Path::new(&p)).ok();
    }
    // Priority 2: explicit data dir env (user/dev override).
    if let Ok(dir) = std::env::var("NOMIFUN_DATA_DIR") {
        if !dir.trim().is_empty() {
            return read_beacon_at(&beacon_path(Path::new(&dir))).ok();
        }
    }
    // Priority 3: platform-standard data dir (same logic as `cli::default_data_dir()`).
    let data_dir = crate::cli::default_data_dir();
    read_beacon_at(&beacon_path(&data_dir)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrips_and_is_owner_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let eps = McpEndpoints {
            knowledge: Some(Endpoint { port: 51123, token: "ktok".into() }),
            requirement: Some(Endpoint { port: 51124, token: "rtok".into() }),
            lifecycle: Some(Endpoint { port: 51125, token: "ltok".into() }),
        };
        write_beacon(dir.path(), &eps).unwrap();
        let p = beacon_path(dir.path());
        assert!(p.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "beacon must be owner-only");
        }
        let back = read_beacon_at(&p).unwrap();
        assert_eq!(back.knowledge.as_ref().unwrap().port, 51123);
        assert_eq!(back.knowledge_then_token(), Some("ktok".to_string()));
    }

    #[test]
    fn read_beacon_for_bridge_with_env_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let eps = McpEndpoints {
            knowledge: Some(Endpoint { port: 60001, token: "bridge_tok".into() }),
            requirement: None,
            lifecycle: None,
        };
        write_beacon(dir.path(), &eps).unwrap();
        let beacon = beacon_path(dir.path());

        // Set NOMI_MCP_ENDPOINTS_FILE to point at our temp beacon.
        unsafe { std::env::set_var("NOMI_MCP_ENDPOINTS_FILE", &beacon) };
        let result = read_beacon_for_bridge();
        unsafe { std::env::remove_var("NOMI_MCP_ENDPOINTS_FILE") };

        assert!(result.is_some());
        assert_eq!(result.unwrap().knowledge.unwrap().port, 60001);
    }

    #[test]
    fn read_beacon_for_bridge_returns_none_when_missing() {
        // Point at a nonexistent file.
        unsafe { std::env::set_var("NOMI_MCP_ENDPOINTS_FILE", "/tmp/nonexistent-beacon-12345.json") };
        let result = read_beacon_for_bridge();
        unsafe { std::env::remove_var("NOMI_MCP_ENDPOINTS_FILE") };
        assert!(result.is_none());
    }
}
