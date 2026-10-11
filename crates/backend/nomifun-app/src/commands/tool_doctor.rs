//! Component diagnostics never initialize logging, a database, or Agent data.
use std::process::ExitCode;
use nomifun_runtime::{ToolId, component_inventory, resolve_tool};

pub async fn run_tool_doctor(verify: bool) -> anyhow::Result<ExitCode> {
    let mut report = serde_json::json!({"inventory": component_inventory(), "verified": verify});
    let mut failed = false;
    if verify {
        let mut observations = Vec::new();
        for id in [ToolId::Bun, ToolId::Node, ToolId::OfficeCli] {
            let mut progress = nomifun_runtime::subscribe_tool_preparation(id);
            let operation = resolve_tool(id);
            tokio::pin!(operation);
            let mut observing = true;
            let result = loop {
                tokio::select! {
                    result = &mut operation => break result,
                    changed = progress.changed(), if observing => {
                        if changed.is_ok() {
                            eprintln!("[runtime-preparation] {}", serde_json::to_string(&*progress.borrow())?);
                        } else { observing = false; }
                    },
                }
            };
            match result {
                Ok(tool) => observations.push(serde_json::json!({
                    "component": id.as_str(), "status":"ready", "version":tool.version,
                    "executable":tool.executable_path, "digest":tool.digest,
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
