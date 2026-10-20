//! Explicit real-worker host transport smoke; only static get_guide is called.
use nomifun_agent_kernel::InvocationControl;
use nomifun_computer::{Computer, WorkerConfig, result, surface};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::args_os()
        .nth(1)
        .ok_or("pass an already built private worker path")?;
    let root = tempfile::Builder::new()
        .prefix("nomifun-computer-host-smoke-")
        .tempdir()?;
    let computer = Computer::new(WorkerConfig::new(binary, root.path()));
    let actions = surface::manifest()
        .tools
        .iter()
        .map(|tool| tool.action_id.clone())
        .collect();
    let reply = computer
        .call(
            "protocol-smoke",
            "Protocol smoke",
            "computer/get_guide",
            json!({}),
            &actions,
            false,
            InvocationControl::default(),
        )
        .await;
    let status = computer.status();
    let closed = computer.close_session("protocol-smoke").await;
    let shutdown = computer.shutdown().await;
    let reply = reply?;
    closed?;
    shutdown?;
    let normalized = result::normalize(&reply.result, false);
    if normalized.is_error {
        return Err("static guide request failed".into());
    }
    println!(
        "{}",
        json!({"ok":true,"worker_version":status.worker_version,"completion":reply.completion,"text_parts":normalized.parts.len(),"running_after_shutdown":computer.status().running,"desktop_actions":0})
    );
    Ok(())
}
