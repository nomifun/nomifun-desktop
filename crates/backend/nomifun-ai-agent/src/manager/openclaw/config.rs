use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use tracing::debug;

use super::agent::DEFAULT_GATEWAY_PORT;

#[derive(Debug, Deserialize, Default)]
pub struct OpenClawFileConfig {
    pub gateway: Option<GatewayFileConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GatewayFileConfig {
    pub port: Option<u16>,
    pub auth: Option<AuthFileConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AuthFileConfig {
    pub mode: Option<String>,
    pub token: Option<String>,
    pub password: Option<String>,
}

const STATE_DIR_NAMES: &[&str] = &[".openclaw", ".clawdbot", ".moltbot", ".moldbot"];
const CONFIG_FILE_NAMES: &[&str] = &["openclaw.json", "clawdbot.json", "moltbot.json", "moldbot.json"];

fn resolve_state_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OPENCLAW_STATE_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(dir) = std::env::var("CLAWDBOT_STATE_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }

    let home = dirs::home_dir()?;
    for name in STATE_DIR_NAMES {
        let p = home.join(name);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn strip_jsonc_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escape_next = false;
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if escape_next {
            result.push(chars[i]);
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            if chars[i] == '\\' {
                escape_next = true;
                result.push(chars[i]);
            } else if chars[i] == '"' {
                in_string = false;
                result.push(chars[i]);
            } else {
                result.push(chars[i]);
            }
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            result.push(chars[i]);
            i += 1;
            continue;
        }

        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '/' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            }
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }
    result
}

pub fn load_openclaw_config() -> Option<OpenClawFileConfig> {
    if let Ok(path) = std::env::var("OPENCLAW_CONFIG_PATH") {
        let p = PathBuf::from(path);
        if p.is_file() {
            return read_config_file(&p);
        }
    }

    let state_dir = resolve_state_dir()?;
    for name in CONFIG_FILE_NAMES {
        let p = state_dir.join(name);
        if p.is_file()
            && let Some(config) = read_config_file(&p)
        {
            debug!(path = %p.display(), "Loaded OpenClaw config");
            return Some(config);
        }
    }
    None
}

fn read_config_file(path: &PathBuf) -> Option<OpenClawFileConfig> {
    let content = fs::read_to_string(path).ok()?;
    let clean = strip_jsonc_comments(&content);
    serde_json::from_str(&clean).ok()
}

pub fn get_gateway_port(config: Option<&OpenClawFileConfig>) -> u16 {
    config
        .and_then(|c| c.gateway.as_ref())
        .and_then(|g| g.port)
        .unwrap_or(DEFAULT_GATEWAY_PORT)
}

pub fn get_gateway_auth_token(config: Option<&OpenClawFileConfig>) -> Option<String> {
    config
        .and_then(|c| c.gateway.as_ref())
        .and_then(|g| g.auth.as_ref())
        .and_then(|a| a.token.clone())
}

pub fn get_gateway_auth_password(config: Option<&OpenClawFileConfig>) -> Option<String> {
    config
        .and_then(|c| c.gateway.as_ref())
        .and_then(|g| g.auth.as_ref())
        .and_then(|a| a.password.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_jsonc_line_comments() {
        let input = r#"{
  "gateway": {
    "port": 18789 // default port
  }
}"#;
        let clean = strip_jsonc_comments(input);
        let config: OpenClawFileConfig = serde_json::from_str(&clean).unwrap();
        assert_eq!(config.gateway.as_ref().and_then(|g| g.port), Some(18789));
    }

    #[test]
    fn strip_jsonc_block_comments() {
        let input = r#"{
  "gateway": {
    /* authentication settings */
    "auth": {
      "mode": "token",
      "token": "secret"
    }
  }
}"#;
        let clean = strip_jsonc_comments(input);
        let config: OpenClawFileConfig = serde_json::from_str(&clean).unwrap();
        let token = config
            .gateway
            .as_ref()
            .and_then(|g| g.auth.as_ref())
            .and_then(|a| a.token.as_deref());
        assert_eq!(token, Some("secret"));
    }

    #[test]
    fn comments_inside_strings_preserved() {
        let input = r#"{"gateway":{"auth":{"token":"my//token/*value*/"}}}"#;
        let clean = strip_jsonc_comments(input);
        let config: OpenClawFileConfig = serde_json::from_str(&clean).unwrap();
        let token = config
            .gateway
            .as_ref()
            .and_then(|g| g.auth.as_ref())
            .and_then(|a| a.token.as_deref());
        assert_eq!(token, Some("my//token/*value*/"));
    }

    #[test]
    fn parse_full_config() {
        let json = r#"{
  "gateway": {
    "port": 9999,
    "auth": {
      "mode": "password",
      "password": "my-pass"
    }
  }
}"#;
        let config: OpenClawFileConfig = serde_json::from_str(json).unwrap();
        assert_eq!(get_gateway_port(Some(&config)), 9999);
        assert_eq!(get_gateway_auth_password(Some(&config)).as_deref(), Some("my-pass"));
        assert!(get_gateway_auth_token(Some(&config)).is_none());
    }

    #[test]
    fn default_port_when_no_config() {
        assert_eq!(get_gateway_port(None), DEFAULT_GATEWAY_PORT);
    }

    #[test]
    fn empty_config_returns_defaults() {
        let config: OpenClawFileConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(get_gateway_port(Some(&config)), DEFAULT_GATEWAY_PORT);
        assert!(get_gateway_auth_token(Some(&config)).is_none());
        assert!(get_gateway_auth_password(Some(&config)).is_none());
    }
}
