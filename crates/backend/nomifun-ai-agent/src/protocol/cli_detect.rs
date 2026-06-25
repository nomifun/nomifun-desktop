use std::sync::Arc;
use std::time::Instant;

use crate::registry::AgentRegistry;
use nomifun_api_types::{AcpHealthCheckResponse, AgentMetadata};
use nomifun_runtime::resolve_command_path;

/// Perform a health check for an ACP backend.
///
/// Checks CLI availability and measures detection latency.
pub(crate) async fn health_check(registry: &Arc<AgentRegistry>, backend: &str) -> AcpHealthCheckResponse {
    let start = Instant::now();

    let Some(meta) = registry.find_builtin_by_backend(backend).await else {
        return AcpHealthCheckResponse {
            available: false,
            latency: None,
            error: Some(format!("No agent_metadata row for backend '{backend}'")),
        };
    };

    let path = probe_command(&meta);
    let latency_ms = start.elapsed().as_millis() as u64;
    let available = path.is_some();

    AcpHealthCheckResponse {
        available,
        latency: Some(latency_ms),
        error: if available {
            None
        } else {
            Some(format!("Spawn command for backend '{backend}' not found in PATH"))
        },
    }
}

fn probe_command(meta: &AgentMetadata) -> Option<String> {
    let cmd = meta.command.as_deref()?;
    resolve_command_path(cmd).map(|p| p.to_string_lossy().into_owned())
}
