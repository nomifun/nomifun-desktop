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
    let environment = match nomifun_runtime::process_environment_for(
        nomifun_runtime::ProcessEnvironmentTarget::Program(path.as_os_str()), &HashMap::new()) {
        Ok(environment) => environment,
        Err(error) => return ExecutableReport { path: Some(path), version: None, error: Some(error.to_string()) },
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
    let mut command = ChildProcessBuilder::clean_cli(path);
    command.env_clear();
    command.arg("--version");
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_script_version_detection_uses_standard_modules_and_shared_literal_launch() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join(" detector 'quoted' & %LITERAL%");
        std::fs::create_dir(&directory).unwrap();
        let before = nomifun_runtime::tool_preparation_snapshot(nomifun_runtime::ToolId::Node);
        for (extension, source) in [
            ("ps1", "if ($args.Count -ne 1 -or $args[0] -cne '--version') {exit 91}\r\n[void](Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256)\r\n[Console]::Write('v99.0.1')\r\n"),
            ("cmd", "@echo off\r\nif not \"%~1\"==\"--version\" exit /b 91\r\nif not \"%~2\"==\"\" exit /b 92\r\necho v99.0.1\r\n"),
            ("bat", "@echo off\r\nif not \"%~1\"==\"--version\" exit /b 91\r\necho v99.0.1\r\n"),
        ] {
            let path = directory.join(format!("node.{extension}"));
            std::fs::write(&path, source).unwrap();
            let report = executable("node", Some(path.clone())).await;
            assert_eq!(report.path, Some(path));
            assert_eq!(report.version.as_deref(), Some("v99.0.1"), "{extension}: {:?}", report.error);
            assert!(report.error.is_none());
        }
        assert_eq!(nomifun_runtime::tool_preparation_snapshot(nomifun_runtime::ToolId::Node), before);
    }
}
