use std::{path::{Path, PathBuf}, time::Duration};
use nomi_process_runtime::ChildProcessBuilder;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutableReport {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EnvironmentReport {
    pub node: ExecutableReport,
    pub npm: ExecutableReport,
    pub bun: ExecutableReport,
    pub node_download_url: String,
}

pub async fn detect_environment(node_path: Option<&str>) -> EnvironmentReport {
    let node_path = node_path.filter(|p| !p.trim().is_empty()).map(PathBuf::from);
    let (node, npm, bun) = tokio::join!(
        executable("node", node_path), executable("npm", None), executable("bun", None));
    EnvironmentReport { node, npm, bun, node_download_url: "https://nodejs.org/en/download".into() }
}

async fn executable(name: &str, custom: Option<PathBuf>) -> ExecutableReport {
    let environment = match nomifun_runtime::process_environment(&HashMap::new()) {
        Ok(environment) => environment,
        Err(error) => return ExecutableReport { path: None, version: None, error: Some(error.to_string()) },
    };
    let path = custom.or_else(|| environment.iter().find(|(key, _)| if cfg!(windows) { key.eq_ignore_ascii_case("PATH") } else { key.as_str() == "PATH" })
        .and_then(|(_, value)| std::env::split_paths(value).find_map(|directory| nomifun_runtime::resolve_command_in(name, &directory))));
    let Some(path) = path else {
        return ExecutableReport { path: None, version: None, error: Some(format!("{name} executable is not installed or not on PATH")) };
    };
    let mut command = version_command(&path);
    command.envs(&environment);
    for key in ["NOMIFUN_HOST_URL", "NOMIFUN_LOCAL_TRUST", "NOMIFUN_AGENT_BINDING_ID"] { command.env_remove(key); }
    match tokio::time::timeout(Duration::from_secs(5), command.output()).await {
        Ok(Ok(output)) if output.status.success() => ExecutableReport {
            path: Some(path), version: Some(crate::decode_output(&output.stdout).trim().to_owned()), error: None },
        Ok(Ok(output)) => ExecutableReport { path: Some(path), version: None,
            error: Some(format!("{name} --version failed: {}", nomi_redact::redact_secrets(&crate::decode_output(&output.stderr)))) },
        Ok(Err(error)) => ExecutableReport { path: Some(path), version: None, error: Some(error.to_string()) },
        Err(_) => ExecutableReport { path: Some(path), version: None, error: Some(format!("{name} --version timed out")) },
    }
}

fn version_command(path: &Path) -> ChildProcessBuilder {
    #[cfg(windows)]
    if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat")) {
        let mut command = ChildProcessBuilder::clean_cli("cmd.exe");
        command.env_clear();
        command.env("NOMIFUN_VERSION_COMMAND", format!("\"{}\" --version", path.display()))
            .args(["/d", "/s", "/c", "%NOMIFUN_VERSION_COMMAND%"]);
        return command;
    }
    let mut command = ChildProcessBuilder::clean_cli(path);
    command.env_clear();
    command.arg("--version");
    command
}
