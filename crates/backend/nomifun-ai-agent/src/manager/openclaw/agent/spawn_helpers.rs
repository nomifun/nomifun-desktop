use nomifun_api_types::OpenClawGatewayConfig;
use nomifun_common::{AppError, CommandSpec, EnvVar};

use super::{DEFAULT_GATEWAY_PORT, GATEWAY_READY_POLL_INTERVAL, GATEWAY_READY_TIMEOUT};

pub(super) fn build_spawn_config(cli_path: &str, workspace: &str, gateway: &OpenClawGatewayConfig) -> CommandSpec {
    let host = gateway.host.as_deref().unwrap_or("127.0.0.1");
    let port = gateway.port.unwrap_or(DEFAULT_GATEWAY_PORT);

    let mut env = vec![
        EnvVar {
            name: "OPENCLAW_GATEWAY_HOST".into(),
            value: host.to_owned(),
        },
        EnvVar {
            name: "OPENCLAW_GATEWAY_PORT".into(),
            value: port.to_string(),
        },
    ];

    if let Some(ref token) = gateway.token {
        env.push(EnvVar {
            name: "OPENCLAW_GATEWAY_TOKEN".into(),
            value: token.clone(),
        });
    }
    if let Some(ref password) = gateway.password {
        env.push(EnvVar {
            name: "OPENCLAW_GATEWAY_PASSWORD".into(),
            value: password.clone(),
        });
    }

    CommandSpec {
        command: cli_path.into(),
        args: vec!["gateway".into(), "--port".into(), port.to_string()],
        env,
        cwd: Some(workspace.to_owned()),
    }
}

pub(super) async fn is_port_listening(host: &str, port: u16) -> bool {
    tokio::net::TcpStream::connect((host, port)).await.is_ok()
}

pub(super) async fn wait_for_gateway_ready(host: &str, port: u16) -> Result<(), AppError> {
    let start = tokio::time::Instant::now();
    while start.elapsed() < GATEWAY_READY_TIMEOUT {
        if is_port_listening(host, port).await {
            return Ok(());
        }
        tokio::time::sleep(GATEWAY_READY_POLL_INTERVAL).await;
    }
    Err(AppError::Internal(format!(
        "OpenClaw gateway did not become ready on {host}:{port} within {}s",
        GATEWAY_READY_TIMEOUT.as_secs()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_kernel::approval_key;

    #[test]
    fn default_gateway_port_is_18789() {
        assert_eq!(DEFAULT_GATEWAY_PORT, 18789);
    }

    fn env_val<'a>(config: &'a CommandSpec, name: &str) -> Option<&'a str> {
        config.env.iter().find(|e| e.name == name).map(|e| e.value.as_str())
    }

    #[test]
    fn build_spawn_config_with_defaults() {
        let gateway = OpenClawGatewayConfig {
            host: None,
            port: None,
            token: None,
            password: None,
            use_external_gateway: false,
            cli_path: Some("/usr/bin/openclaw".into()),
        };
        let config = build_spawn_config("/usr/bin/openclaw", "/proj", &gateway);
        assert_eq!(config.command.to_str().unwrap(), "/usr/bin/openclaw");
        assert_eq!(env_val(&config, "OPENCLAW_GATEWAY_HOST").unwrap(), "127.0.0.1");
        assert_eq!(env_val(&config, "OPENCLAW_GATEWAY_PORT").unwrap(), "18789");
        assert!(env_val(&config, "OPENCLAW_GATEWAY_TOKEN").is_none());
    }

    #[test]
    fn build_spawn_config_with_custom_gateway() {
        let gateway = OpenClawGatewayConfig {
            host: Some("remote.host".into()),
            port: Some(9999),
            token: Some("secret".into()),
            password: Some("pass".into()),
            use_external_gateway: true,
            cli_path: Some("/usr/bin/openclaw".into()),
        };
        let config = build_spawn_config("/usr/bin/openclaw", "/proj", &gateway);
        assert_eq!(env_val(&config, "OPENCLAW_GATEWAY_HOST").unwrap(), "remote.host");
        assert_eq!(env_val(&config, "OPENCLAW_GATEWAY_PORT").unwrap(), "9999");
        assert_eq!(env_val(&config, "OPENCLAW_GATEWAY_TOKEN").unwrap(), "secret");
        assert_eq!(env_val(&config, "OPENCLAW_GATEWAY_PASSWORD").unwrap(), "pass");
    }

    #[test]
    fn approval_key_formats_correctly() {
        assert_eq!(approval_key(Some("edit"), Some("file")), "edit:file");
        assert_eq!(approval_key(Some("edit"), None), "edit");
        assert_eq!(approval_key(None, None), "");
    }
}
