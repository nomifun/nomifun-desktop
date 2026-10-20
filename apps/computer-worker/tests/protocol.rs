//! These tests use the shipped worker, never a fake transport or desktop tool.

#[allow(dead_code)]
#[path = "../src/probe.rs"]
mod probe;

use cua_driver_sdk::worker::{ActionCompletion, ChannelRequest, PRIVATE_WORKER_PROTOCOL_VERSION};
use probe::WorkerProbe;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

fn spawn() -> WorkerProbe {
    WorkerProbe::spawn(Path::new(env!("CARGO_BIN_EXE_nomifun-computer-worker"))).unwrap()
}

fn initialized() -> WorkerProbe {
    let mut worker = spawn();
    let ready = worker.initialize().unwrap();
    assert!(ready.ok, "{:?}", ready.error);
    assert_eq!(ready.completion, ActionCompletion::Completed);
    assert_eq!(ready.result.as_ref().unwrap()["ready"], true);
    assert_ne!(ready.result.as_ref().unwrap()["pid"], std::process::id());
    worker
}

fn shutdown(mut worker: WorkerProbe) {
    let request = worker.request("shutdown", None, None, None);
    let response = worker.exchange(&request).unwrap();
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(response.result.unwrap()["shutdown"], true);
    assert!(worker.wait().unwrap().success());
}

fn bind(worker: &mut WorkerProbe, label: &str) -> String {
    let request = worker.request(
        "bind_session",
        Some(json!({
            "public_session":label, "mode":"standard",
            "ttl_seconds":3600, "idle_ttl_seconds":600,
            "capability_manifest_path":null, "bounded_manifest_path":null,
        })),
        None,
        None,
    );
    let response = worker.exchange(&request).unwrap();
    assert!(response.ok, "{:?}", response.error);
    response.result.unwrap()["session_handle"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn session_call(worker: &mut WorkerProbe, handle: &str, name: &str) -> Value {
    let request = worker.request("call", Some(json!({})), Some(name), Some(handle));
    let response = worker.exchange(&request).unwrap();
    assert!(response.ok, "{name}: {:?}", response.error);
    let result = response.result.unwrap();
    assert_ne!(result["isError"], true, "{name}: {result}");
    result
}

fn close(worker: &mut WorkerProbe, handle: &str) {
    let request = worker.request("close_session", None, None, Some(handle));
    let response = worker.exchange(&request).unwrap();
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(response.result.unwrap()["closed"], true);
}

#[test]
fn same_label_close_rebind_starts_a_fresh_native_lifecycle() {
    let mut worker = initialized();
    let label = "Repeated Agent name";
    let first = bind(&mut worker, label);
    let first_state = session_call(&mut worker, &first, "get_session");
    assert_eq!(first_state["structuredContent"]["state"], "active");
    assert!(first_state["structuredContent"]["session"].as_str().unwrap().starts_with(label));
    session_call(&mut worker, &first, "get_guide");
    close(&mut worker, &first);

    let second = bind(&mut worker, label);
    assert_ne!(first, second);
    let second_state = session_call(&mut worker, &second, "get_session");
    assert_eq!(second_state["structuredContent"]["state"], "active");
    assert_ne!(first_state["structuredContent"]["session"], second_state["structuredContent"]["session"]);
    session_call(&mut worker, &second, "get_guide");
    close(&mut worker, &second);
    shutdown(worker);
}

#[test]
fn same_label_live_handles_have_independent_native_owners() {
    let mut worker = initialized();
    let first = bind(&mut worker, "Concurrent Agent name");
    let second = bind(&mut worker, "Concurrent Agent name");
    assert_ne!(first, second);
    let first_state = session_call(&mut worker, &first, "get_session");
    let second_state = session_call(&mut worker, &second, "get_session");
    assert_eq!(first_state["structuredContent"]["state"], "active");
    assert_eq!(second_state["structuredContent"]["state"], "active");
    assert_ne!(first_state["structuredContent"]["session"], second_state["structuredContent"]["session"]);
    close(&mut worker, &first);
    assert_eq!(session_call(&mut worker, &second, "get_session")["structuredContent"]["state"], "active");
    session_call(&mut worker, &second, "get_guide");
    assert_eq!(session_call(&mut worker, &second, "end_session")["structuredContent"]["active"], false);
    close(&mut worker, &second);
    let request = worker.request("call", Some(json!({})), Some("get_session"), Some(&second));
    let response = worker.exchange(&request).unwrap();
    assert_eq!(response.error_code.as_deref(), Some("session_not_bound"));
    assert_eq!(response.completion, ActionCompletion::NotStarted);
    shutdown(worker);
}

#[test]
fn initialize_metadata_list_bind_close_shutdown() {
    let mut worker = initialized();
    let metadata = worker.request("metadata", None, None, None);
    let metadata = worker.exchange(&metadata).unwrap();
    assert!(metadata.ok);
    assert!(metadata.result.unwrap().is_object());
    let list = worker.request("list", None, None, None);
    let list = worker.exchange(&list).unwrap();
    assert!(list.ok);
    let result = list.result.unwrap();
    let tools = result["tools"].as_array().unwrap();
    assert!(!tools.is_empty());
    assert!(tools.iter().any(|tool| tool["name"] == "get_window_state"));
    assert!(!tools.iter().any(|tool| tool["name"] == "check_update"));
    let bind = worker.request(
        "bind_session",
        Some(json!({
            "public_session":"Isolated protocol test", "mode":"standard",
            "ttl_seconds":3600, "idle_ttl_seconds":600,
            "capability_manifest_path":null, "bounded_manifest_path":null,
        })),
        None,
        None,
    );
    let bound = worker.exchange(&bind).unwrap();
    assert!(bound.ok, "{:?}", bound.error);
    let handle = bound.result.unwrap()["session_handle"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!handle.is_empty());
    for _ in 0..2 {
        let close = worker.request("close_session", None, None, Some(&handle));
        let close = worker.exchange(&close).unwrap();
        assert!(close.ok);
        assert_eq!(close.result.unwrap()["closed"], true);
    }
    let call = worker.request(
        "call",
        Some(json!({})),
        Some("get_window_state"),
        Some(&handle),
    );
    let call = worker.exchange(&call).unwrap();
    assert!(!call.ok);
    assert_eq!(call.error_code.as_deref(), Some("session_not_bound"));
    assert_eq!(call.completion, ActionCompletion::NotStarted);
    shutdown(worker);
}

#[test]
fn unbound_session_rejects_before_any_tool_dispatch() {
    let mut worker = initialized();
    let call = worker.request("call", Some(json!({})), Some("click"), Some("unbound"));
    let call = worker.exchange(&call).unwrap();
    assert!(!call.ok);
    assert_eq!(call.error_code.as_deref(), Some("session_not_bound"));
    assert_eq!(call.completion, ActionCompletion::NotStarted);
    shutdown(worker);
}

#[test]
fn mismatched_generation_and_protocol_leave_live_channel_usable() {
    let mut worker = initialized();
    for mismatch_protocol in [false, true] {
        let mut request = worker.request("metadata", None, None, None);
        if mismatch_protocol {
            request.protocol_version += 1;
        } else {
            request.generation = "other-generation".into();
        }
        let response = worker.exchange(&request).unwrap();
        assert!(!response.ok);
        assert_eq!(response.error_code.as_deref(), Some("generation_mismatch"));
        assert_eq!(response.completion, ActionCompletion::NotStarted);
    }
    shutdown(worker);
}

#[test]
fn unknown_operation_returns_upstream_worker_error() {
    let mut worker = initialized();
    let request = worker.request("unknown-operation", None, None, None);
    let response = worker.exchange(&request).unwrap();
    assert!(!response.ok);
    assert_eq!(
        response.error_code.as_deref(),
        Some("worker_request_failed")
    );
    assert_eq!(response.completion, ActionCompletion::Completed);
    assert!(
        response
            .error
            .unwrap()
            .contains("unknown private worker operation")
    );
    shutdown(worker);
}

#[test]
fn malformed_request_is_rejected_and_channel_can_shutdown() {
    let mut worker = initialized();
    worker.write_line("{malformed").unwrap();
    let response = worker.response().unwrap();
    assert!(!response.ok);
    assert_eq!(response.request_id, 0);
    assert_eq!(response.error_code.as_deref(), Some("invalid_request"));
    assert_eq!(response.completion, ActionCompletion::NotStarted);
    shutdown(worker);
}

#[test]
fn initialization_identity_must_match_before_runtime_creation() {
    for variant in ["generation", "protocol", "request_id", "operation"] {
        let mut worker = spawn();
        let mut request = ChannelRequest {
            protocol_version: PRIVATE_WORKER_PROTOCOL_VERSION,
            request_id: 0,
            generation: worker.generation.clone(),
            operation: "initialize".into(),
            name: None,
            arguments: Some(json!({})),
            session_handle: None,
        };
        match variant {
            "generation" => request.generation = "other-generation".into(),
            "protocol" => request.protocol_version += 1,
            "request_id" => request.request_id += 1,
            "operation" => request.operation = "list".into(),
            _ => unreachable!(),
        }
        let response = worker.exchange(&request).unwrap();
        assert!(!response.ok, "{variant}");
        assert_eq!(
            response.error_code.as_deref(),
            Some("invalid_initialization")
        );
        assert_eq!(response.completion, ActionCompletion::NotStarted);
        assert!(worker.wait().unwrap().success());
    }
}

#[test]
fn stdin_eof_before_initialization_exits_cleanly() {
    assert!(spawn().wait().unwrap().success());
}

#[test]
fn stdin_eof_after_initialization_exits_cleanly() {
    assert!(initialized().wait().unwrap().success());
}

#[test]
fn packaged_self_check_exercises_real_protocol() {
    let output = Command::new(env!("CARGO_BIN_EXE_nomifun-computer-worker"))
        .arg("--self-check")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["protocol_version"], PRIVATE_WORKER_PROTOCOL_VERSION);
    assert!(result["tool_count"].as_u64().unwrap() > 0);
}

#[test]
fn unsupported_cli_arguments_never_start_a_runtime() {
    for args in [
        vec![],
        vec!["__private-worker", "--generation", "bad_generation"],
        vec!["__private-worker", "--generation", "valid", "extra"],
        vec!["--self-check", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_nomifun-computer-worker"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[cfg(target_os = "windows")]
#[test]
fn private_user_app_helper_rejects_missing_or_forged_capabilities_without_a_runtime() {
    for arguments in [
        vec!["--nomifun-user-app-launch"],
        vec!["--nomifun-user-app-launch", ".", "not-a-private-nonce"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_nomifun-computer-worker"))
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(126));
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
