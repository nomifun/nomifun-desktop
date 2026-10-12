//! Component diagnostics never initialize logging, a database, or Agent data,
//! and verification never downloads missing components.
use std::process::ExitCode;
use nomifun_runtime::{ToolId, component_inventory, resolve_existing_tool};

pub async fn run_tool_doctor(verify: bool) -> anyhow::Result<ExitCode> {
    let mut report = serde_json::json!({"inventory": component_inventory(), "verified": verify});
    let mut failed = false;
    if verify {
        let mut observations = Vec::new();
        for id in [ToolId::Bun, ToolId::Node, ToolId::OfficeCli] {
            match resolve_existing_tool(id).await {
                Ok(tool) => observations.push(serde_json::json!({
                    "component": id.as_str(), "status":"ready", "version":tool.version,
                    "executable":tool.executable_path, "digest":tool.digest,
                    "source":tool.source,
                })),
                Err(error) => {
                    failed = true;
                    observations.push(serde_json::json!({"component":id.as_str(),"status":"unavailable","error":error.to_string()}));
                }
            }
        }
        report["observations"] = observations.into();
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}
