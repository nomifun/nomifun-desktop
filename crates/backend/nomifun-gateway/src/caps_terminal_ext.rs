//! Extended terminal-session capabilities (registry form): get / write_input /
//! kill / delete / resize / relaunch / update.
//!
//! Companion module to `caps_terminal.rs` (which covers create / list). These
//! are the remaining mutation and query endpoints that a gateway-connected agent
//! needs to fully manage PTY sessions.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier, Surface};
use crate::server::ok;

// ─── Params ────────────────────────────────────────────────────────────────

/// Parameters for reading a single terminal session's detail/status.
#[derive(Deserialize, JsonSchema)]
struct GetTerminalParams {
    /// The terminal session id (from nomi_list_terminals).
    id: i64,
}

/// Parameters for writing bytes/keystrokes to a terminal's PTY.
#[derive(Deserialize, JsonSchema)]
struct WriteInputParams {
    /// The terminal session id.
    id: i64,
    /// Base64-encoded bytes to write to the PTY stdin. Encode raw keystrokes
    /// (including control sequences like \r for Enter, \x03 for Ctrl-C) as
    /// base64 before passing here.
    data_b64: String,
}

/// Parameters for terminating a terminal's running process (SIGKILL).
#[derive(Deserialize, JsonSchema)]
struct KillTerminalParams {
    /// The terminal session id.
    id: i64,
}

/// Parameters for permanently deleting a terminal session (kills process + removes row).
#[derive(Deserialize, JsonSchema)]
struct DeleteTerminalParams {
    /// The terminal session id.
    id: i64,
}

/// Parameters for resizing a terminal's PTY (cols x rows).
#[derive(Deserialize, JsonSchema)]
struct ResizeTerminalParams {
    /// The terminal session id.
    id: i64,
    /// Number of columns (width in characters).
    cols: u16,
    /// Number of rows (height in characters).
    rows: u16,
}

/// Parameters for relaunching a terminal's process in place (same session id,
/// fresh child process).
#[derive(Deserialize, JsonSchema)]
struct RelaunchTerminalParams {
    /// The terminal session id.
    id: i64,
}

/// Parameters for updating a terminal session's metadata (rename / pin).
#[derive(Deserialize, JsonSchema)]
struct UpdateTerminalParams {
    /// The terminal session id.
    id: i64,
    /// New display name (omit to keep current).
    #[serde(default)]
    name: Option<String>,
    /// Pin (true) or unpin (false) the terminal; pinned terminals persist in
    /// the sidebar. Omit to keep current.
    #[serde(default)]
    pinned: Option<bool>,
}

// ─── Handlers ──────────────────────────────────────────────────────────────

async fn get_terminal(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: GetTerminalParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity (NOMI_GW_MCP_USER_ID)"});
    }
    match deps.terminal_service.get(p.id).await {
        Ok(resp) => ok(json!({
            "id": resp.id,
            "name": resp.name,
            "status": resp.last_status,
            "cwd": resp.cwd,
            "command": resp.command,
            "args": resp.args,
            "backend": resp.backend,
            "mode": resp.mode,
            "cols": resp.cols,
            "rows": resp.rows,
            "exit_code": resp.exit_code,
            "pinned": resp.pinned,
            "created_at": resp.created_at,
            "updated_at": resp.updated_at,
        })),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn write_input(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: WriteInputParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity (NOMI_GW_MCP_USER_ID)"});
    }
    match deps.terminal_service.input(p.id, &p.data_b64).await {
        Ok(()) => ok(json!({"written": true})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn kill_terminal(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: KillTerminalParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity (NOMI_GW_MCP_USER_ID)"});
    }
    match deps.terminal_service.kill(p.id).await {
        Ok(()) => ok(json!({"killed": true, "id": p.id})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn delete_terminal(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: DeleteTerminalParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity (NOMI_GW_MCP_USER_ID)"});
    }
    match deps.terminal_service.delete(p.id).await {
        Ok(()) => ok(json!({"deleted": true, "id": p.id})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn resize_terminal(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ResizeTerminalParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity (NOMI_GW_MCP_USER_ID)"});
    }
    match deps.terminal_service.resize(p.id, p.cols, p.rows).await {
        Ok(()) => ok(json!({"resized": true, "id": p.id, "cols": p.cols, "rows": p.rows})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn relaunch_terminal(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: RelaunchTerminalParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity (NOMI_GW_MCP_USER_ID)"});
    }
    match deps.terminal_service.relaunch(p.id).await {
        Ok(resp) => ok(json!({
            "id": resp.id,
            "name": resp.name,
            "status": resp.last_status,
            "cwd": resp.cwd,
            "command": resp.command,
            "args": resp.args,
            "backend": resp.backend,
            "mode": resp.mode,
            "note": "process relaunched in place (same session id, fresh child)"
        })),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn update_terminal(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: UpdateTerminalParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity (NOMI_GW_MCP_USER_ID)"});
    }
    if p.name.is_none() && p.pinned.is_none() {
        return json!({"error": "nothing to update: provide at least one of name / pinned"});
    }
    match deps.terminal_service.update_meta(p.id, p.name, p.pinned).await {
        Ok(resp) => ok(json!({
            "id": resp.id,
            "name": resp.name,
            "pinned": resp.pinned,
            "status": resp.last_status,
        })),
        Err(e) => json!({"error": e.to_string()}),
    }
}

// ─── Registration ──────────────────────────────────────────────────────────

/// Register the extended terminal-domain capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<GetTerminalParams, _, _>(
        CapabilityMeta::new(
            "nomi_terminal_get",
            "terminal",
            "Get a single terminal session's detail and current status (running/exited, exit code, dimensions, etc.).",
            DangerTier::Read,
        ),
        |deps, ctx, p| get_terminal(deps, ctx, p),
    ));
    out.push(Capability::new::<WriteInputParams, _, _>(
        CapabilityMeta::new(
            "nomi_terminal_write_input",
            "terminal",
            "Write base64-encoded bytes/keystrokes to a terminal's PTY stdin. Powerful: can execute arbitrary commands in the running shell.",
            DangerTier::Write,
        )
        .deny_on(&[Surface::Channel]),
        |deps, ctx, p| write_input(deps, ctx, p),
    ));
    out.push(Capability::new::<KillTerminalParams, _, _>(
        CapabilityMeta::new(
            "nomi_terminal_kill",
            "terminal",
            "Send SIGKILL to terminate the terminal's running process. The session remains (status becomes 'exited'); use relaunch to restart or delete to remove entirely.",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        |deps, ctx, p| kill_terminal(deps, ctx, p),
    ));
    out.push(Capability::new::<DeleteTerminalParams, _, _>(
        CapabilityMeta::new(
            "nomi_terminal_delete",
            "terminal",
            "Permanently delete a terminal session (kills the process if running, removes the row and all associated data).",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        |deps, ctx, p| delete_terminal(deps, ctx, p),
    ));
    out.push(Capability::new::<ResizeTerminalParams, _, _>(
        CapabilityMeta::new(
            "nomi_terminal_resize",
            "terminal",
            "Resize a terminal's PTY to the given cols x rows (triggers deferred spawn if the session was created with defer_spawn).",
            DangerTier::Write,
        ),
        |deps, ctx, p| resize_terminal(deps, ctx, p),
    ));
    out.push(Capability::new::<RelaunchTerminalParams, _, _>(
        CapabilityMeta::new(
            "nomi_terminal_relaunch",
            "terminal",
            "Relaunch a terminal's process in place: kills the old child and spawns a fresh one reusing the same session id, command, and cwd.",
            DangerTier::Write,
        )
        .deny_on(&[Surface::Channel]),
        |deps, ctx, p| relaunch_terminal(deps, ctx, p),
    ));
    out.push(Capability::new::<UpdateTerminalParams, _, _>(
        CapabilityMeta::new(
            "nomi_terminal_update",
            "terminal",
            "Update a terminal session's metadata: rename it and/or pin/unpin it.",
            DangerTier::Write,
        ),
        |deps, ctx, p| update_terminal(deps, ctx, p),
    ));
}
