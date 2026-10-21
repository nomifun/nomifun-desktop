// Copyright (c) 2025 Cua AI, Inc. MIT License; see ../LICENSE.
// Adapted from trycua/cua libs/cua-driver/rust/crates/cua-driver/src/private_worker.rs
// at 4d959bbfa57f0c89f310330e849500b222084194.
//! NomiFun's directly supervised, process-isolated SDK runtime entry point.
//!
//! This is intentionally not a CLI product surface. A trusted SDK host starts
//! it directly and owns the inherited stdin/stdout channel for the lifetime of
//! one runtime generation.

use cua_driver_sdk::worker::{
    ActionCompletion, ChannelRequest, ChannelResponse, PRIVATE_WORKER_PROTOCOL_VERSION,
    WorkerInitialization,
};
use cua_driver_sdk::{CuaDriver, CuaDriverSession, DriverHostOptions, TrustedSessionOptions};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::Arc;

struct InitializationSignal(Option<std::sync::mpsc::SyncSender<bool>>);

#[cfg(target_os = "macos")]
const DIRECT_CAPTURE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq, Eq)]
enum DirectCaptureProbeResult { Ready, Unavailable, TimedOut, Failed }

#[cfg(target_os = "macos")]
impl DirectCaptureProbeResult {
    fn metadata(&self) -> Value {
        serde_json::json!({"status": match self {
            Self::Ready => "ready", Self::Unavailable => "unavailable",
            Self::TimedOut => "timed_out", Self::Failed => "probe_failed",
        }})
    }
}

#[cfg(target_os = "macos")]
async fn bounded_capture_probe(
    timeout: std::time::Duration,
    probe: impl std::future::Future<Output = Result<bool, tokio::task::JoinError>>,
) -> DirectCaptureProbeResult {
    match tokio::time::timeout(timeout, probe).await {
        Ok(Ok(true)) => DirectCaptureProbeResult::Ready,
        Ok(Ok(false)) => DirectCaptureProbeResult::Unavailable,
        Ok(Err(_)) => DirectCaptureProbeResult::Failed,
        Err(_) => DirectCaptureProbeResult::TimedOut,
    }
}

impl InitializationSignal {
    fn ready(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(true);
        }
    }
}

impl Drop for InitializationSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(false);
        }
    }
}

pub fn run(
    generation: String,
    initialized: Option<std::sync::mpsc::SyncSender<bool>>,
) -> anyhow::Result<()> {
    if generation.is_empty() {
        anyhow::bail!("private worker requires one valid --generation value");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async(generation, initialized))
}

async fn run_async(
    generation: String,
    initialized: Option<std::sync::mpsc::SyncSender<bool>>,
) -> anyhow::Result<()> {
    let mut initialized = InitializationSignal(initialized);
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    let Some(first_line) = lines.next() else {
        return Ok(());
    };
    let first_line = first_line?;
    let initialization_request: ChannelRequest = serde_json::from_str(&first_line)?;
    if initialization_request.protocol_version != PRIVATE_WORKER_PROTOCOL_VERSION
        || initialization_request.request_id != 0
        || initialization_request.generation != generation
        || initialization_request.operation != "initialize"
    {
        write_response(
            &mut writer,
            &ChannelResponse::error(
                initialization_request.request_id,
                &generation,
                "invalid_initialization",
                "private worker initialization identity mismatch",
                ActionCompletion::NotStarted,
            ),
        )?;
        return Ok(());
    }
    let initialization: WorkerInitialization = serde_json::from_value(
        initialization_request
            .arguments
            .ok_or_else(|| anyhow::anyhow!("private worker initialization omitted options"))?,
    )?;
    if initialization.host_bundle_id.trim().is_empty() {
        write_response(
            &mut writer,
            &ChannelResponse::error(
                0,
                &generation,
                "invalid_initialization",
                "private worker host identity is empty",
                ActionCompletion::NotStarted,
            ),
        )?;
        return Ok(());
    }
    let driver = match CuaDriver::try_create_configured_for_host(
        initialization.configured_driver,
        DriverHostOptions {
            cursor: cursor_overlay::CursorConfig::default(),
            host_owns_permission_ux: true,
            host_bundle_id: Some(initialization.host_bundle_id.clone()),
            claude_code_compatibility: false,
            prepare_desktop_environment: true,
            register_host_tools: None,
            authorization_host: None,
            activity_observer: None,
        },
    ) {
        Ok(driver) => driver,
        Err(error) => {
            write_response(
                &mut writer,
                &ChannelResponse::error(
                    0,
                    &generation,
                    "runtime_initialization_failed",
                    error.to_string(),
                    ActionCompletion::NotStarted,
                ),
            )?;
            return Ok(());
        }
    };
    initialized.ready();
    let metadata = driver.metadata().await?;
    let diagnostics = crate::native_status::collect(&driver, metadata.embedded).await;
    write_response(
        &mut writer,
        &ChannelResponse::ok(
            0,
            &generation,
            serde_json::json!({
                "ready": true,
                "pid": std::process::id(),
                "host_bundle_id": initialization.host_bundle_id,
                "metadata": metadata,
                "diagnostics": diagnostics,
            }),
        ),
    )?;

    let mut sessions: HashMap<String, Arc<CuaDriverSession>> = HashMap::new();
    for line in lines {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        let request: ChannelRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_response(
                    &mut writer,
                    &ChannelResponse::error(
                        0,
                        &generation,
                        "invalid_request",
                        format!("parse private worker request: {error}"),
                        ActionCompletion::NotStarted,
                    ),
                )?;
                continue;
            }
        };
        if request.protocol_version != PRIVATE_WORKER_PROTOCOL_VERSION
            || request.generation != generation
        {
            write_response(
                &mut writer,
                &ChannelResponse::error(
                    request.request_id,
                    &generation,
                    "generation_mismatch",
                    "private worker request belongs to another runtime generation",
                    ActionCompletion::NotStarted,
                ),
            )?;
            continue;
        }

        let response = handle_request(&driver, &mut sessions, &generation, request).await;
        let shutdown = response
            .result
            .as_ref()
            .and_then(|value| value.get("shutdown"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        write_response(&mut writer, &response)?;
        if shutdown {
            break;
        }
    }

    let session_cleanup = close_sessions(&mut sessions).await;
    let runtime_cleanup = driver.shutdown().await;
    session_cleanup.map_err(anyhow::Error::msg)?;
    runtime_cleanup?;
    Ok(())
}

async fn handle_request(
    driver: &Arc<CuaDriver>,
    sessions: &mut HashMap<String, Arc<CuaDriverSession>>,
    generation: &str,
    request: ChannelRequest,
) -> ChannelResponse {
    let request_id = request.request_id;
    if request.operation == "request_screen_capture" {
        // This is a host-only explicit permission flow, never an SDK/model
        // tool or a Session call. No caller-controlled capture arguments exist.
        if request.name.is_some() || request.arguments.is_some() || request.session_handle.is_some() {
            return ChannelResponse::error(request_id, generation, "invalid_request",
                "request_screen_capture accepts no tool, arguments or session", ActionCompletion::NotStarted);
        }
        #[cfg(target_os = "macos")]
        {
            let result = bounded_capture_probe(DIRECT_CAPTURE_PROBE_TIMEOUT,
                tokio::task::spawn_blocking(platform_macos::capture::capture_probe_frame)).await;
            // A timed-out blocking native call stays in this worker's exact
            // process owner. The host immediately retires/reaps that owner.
            return ChannelResponse::ok(request_id, generation, result.metadata());
        }
        #[cfg(not(target_os = "macos"))]
        return ChannelResponse::error(request_id, generation, "platform_unsupported",
            "Explicit screen capture consent is only supported on macOS", ActionCompletion::NotStarted);
    }
    let result: Result<Value, String> = match request.operation.as_str() {
        "metadata" => driver
            .metadata()
            .await
            .and_then(|metadata| {
                serde_json::to_value(metadata).map_err(|error| {
                    cua_driver_sdk::DriverError::Protocol {
                        reason: error.to_string(),
                    }
                })
            })
            .map_err(|error| error.to_string()),
        "list" => driver
            .list_tools_json()
            .await
            .map_err(|error| error.to_string())
            .and_then(|json| serde_json::from_str(&json).map_err(|error| error.to_string())),
        "call" => {
            let name = request.name.as_deref().unwrap_or("");
            let arguments = request
                .arguments
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            let invocation = if let Some(handle) = request.session_handle.as_deref() {
                match sessions.get(handle) {
                    Some(session) => {
                        session
                            .call_tool(name.to_owned(), arguments.to_string())
                            .await
                    }
                    None => {
                        return ChannelResponse::error(
                            request_id,
                            generation,
                            "session_not_bound",
                            "private worker session handle is not live on this channel",
                            ActionCompletion::NotStarted,
                        );
                    }
                }
            } else {
                driver.call_tool_from_trusted_adapter(name, arguments).await
            };
            invocation.map_err(|error| error.to_string()).and_then(|result| {
                if name == "end_session" && !result.is_error {
                    if let Some(handle) = request.session_handle.as_deref() {
                        if let Some(session) = sessions.remove(handle) {
                            session.close();
                        }
                    }
                }
                serde_json::from_str(&result.raw_json).map_err(|error| error.to_string())
            })
        }
        "bind_session" => {
            let options = request
                .arguments
                .ok_or_else(|| "bind_session omitted options".to_owned())
                .and_then(|value| {
                    serde_json::from_value::<TrustedSessionOptions>(value)
                        .map_err(|error| error.to_string())
                });
            match options {
                Ok(mut options) if !options.public_session.trim().is_empty() => {
                    let handle = uuid::Uuid::new_v4().to_string();
                    // A public label also keys Core lifecycle state. Reusing an
                    // Agent name would collide with another transport, including
                    // the next Turn after the old SDK authorization was revoked.
                    // Keep the visible name first, but never reuse its native ID.
                    options.public_session = format!("{} [{handle}]", options.public_session);
                    match driver.create_trusted_session(options) {
                    Ok(session) => {
                        sessions.insert(handle.clone(), session.clone());
                        match session.call_tool("start_session".into(), "{}".into()).await {
                            Ok(result) if !result.is_error => {
                                Ok(serde_json::json!({"session_handle": handle}))
                            }
                            // Retain the exact native owner until explicit
                            // cleanup, even if initialization partially failed.
                            Ok(result) => Err(format!("start native session: {}", result.text)),
                            Err(error) => Err(error.to_string()),
                        }
                    }
                    Err(error) => Err(error.to_string()),
                    }
                }
                Ok(_) => Err("public_session must not be empty".into()),
                Err(error) => Err(error),
            }
        }
        "close_session" => {
            let Some(handle) = request.session_handle.as_deref() else {
                return ChannelResponse::error(
                    request_id,
                    generation,
                    "invalid_request",
                    "close_session omitted session_handle",
                    ActionCompletion::NotStarted,
                );
            };
            close_session(sessions, handle)
                .await
                .map(|()| serde_json::json!({"closed": true}))
        }
        "shutdown" => {
            match close_sessions(sessions).await {
                Ok(()) => match driver.shutdown().await {
                    Ok(()) => Ok(serde_json::json!({"shutdown": true})),
                    Err(error) => Err(error.to_string()),
                },
                Err(error) => Err(error),
            }
        }
        other => Err(format!("unknown private worker operation: {other}")),
    };

    match result {
        Ok(value) => ChannelResponse::ok(request_id, generation, value),
        Err(error) => ChannelResponse::error(
            request_id,
            generation,
            "worker_request_failed",
            error,
            ActionCompletion::Completed,
        ),
    }
}

async fn close_session(
    sessions: &mut HashMap<String, Arc<CuaDriverSession>>,
    handle: &str,
) -> Result<(), String> {
    let Some(session) = sessions.get(handle).cloned() else {
        return Ok(());
    };
    // SDK close only revokes authorization. End Core lifecycle resources while
    // the exact transport is still authorized, and retain it if cleanup fails.
    // The SDK's typed end_session closes even on failure, so use call_tool here.
    let result = session
        .call_tool("end_session".into(), "{}".into())
        .await
        .map_err(|error| error.to_string())?;
    if result.is_error {
        return Err(format!("end native session: {}", result.text));
    }
    session.close();
    sessions.remove(handle);
    Ok(())
}

async fn close_sessions(
    sessions: &mut HashMap<String, Arc<CuaDriverSession>>,
) -> Result<(), String> {
    let handles = sessions.keys().cloned().collect::<Vec<_>>();
    let mut failure = None;
    for handle in handles {
        if let Err(error) = close_session(sessions, &handle).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn write_response(writer: &mut impl Write, response: &ChannelResponse) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *writer, response)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn direct_capture_statuses_are_bounded_without_any_native_capture() {
        let budget = std::time::Duration::from_millis(20);
        assert_eq!(bounded_capture_probe(budget, async { Ok(true) }).await.metadata(), serde_json::json!({"status":"ready"}));
        assert_eq!(bounded_capture_probe(budget, async { Ok(false) }).await.metadata(), serde_json::json!({"status":"unavailable"}));
        assert_eq!(bounded_capture_probe(budget, tokio::spawn(async { panic!("fixture probe failure"); #[allow(unreachable_code)] false })).await.metadata(), serde_json::json!({"status":"probe_failed"}));
        assert_eq!(bounded_capture_probe(budget, std::future::pending()).await.metadata(), serde_json::json!({"status":"timed_out"}));
    }
}
