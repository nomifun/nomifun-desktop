//! Opt-in harness utility, not a test and not a success assertion. Call while
//! the fixture still owns its workspace (or retain it for an external grader).
use std::path::Path;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use nomifun_agent_contracts::{AgentSessionId, OperationId, digest_bytes};

pub enum CaptureSource { LiveProvider, ScriptedProvider }

pub struct CaptureMetadata<'a> {
    pub suite_id: &'a str,
    pub trial_id: &'a str,
    pub runtime_build_digest: &'a str,
    pub model: &'a str,
    pub observed_models: &'a [String],
    pub duration_ms: u64,
    pub source: CaptureSource,
}

/// Export only canonical identities, lifecycle metadata and hashes. Provider
/// reasoning, prompt bodies, file bodies and credential-bearing output are not
/// exported. Independent acceptance reads the actual retained workspace.
pub async fn capture_native_turn(
    services: &nomifun_app::compatibility::AppServices,
    session_id: &str,
    operation_id: &str,
    workspace: &Path,
    output: &Path,
    metadata: CaptureMetadata<'_>,
) -> Result<()> {
    ensure!(!output.exists(), "capture output must not overwrite an earlier run");
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await?;
    let facts = store.chat_causality_facts(&AgentSessionId::from(session_id),&OperationId::from(operation_id)).await?;
    let mut input_sha256 = None;
    let mut events = Vec::new();
    for event in &facts.events {
        let original = facts.event_payloads.get(event.event_id.as_ref()).context("canonical payload missing")?;
        if event.kind.0 == "message/user-accepted" && input_sha256.is_none() {
            if let Some(content) = original.get("content").and_then(Value::as_str) {
                input_sha256 = Some(digest_bytes(content.as_bytes()).as_ref().to_owned());
            }
        }
        let mut projected = serde_json::Map::new();
        for key in ["operation_id","turn_id","effect_id","outcome","input_digest","model_steps","finish_reason"] {
            if let Some(value) = original.get(key).filter(|value| value.is_string() || value.is_number() || value.is_boolean()) {
                projected.insert(key.to_owned(),value.clone());
            }
        }
        if event.kind.0 == "runtime/progress-recorded" {
            let mut native = serde_json::Map::new();
            if let Some(body) = original.get("event") {
                for key in ["event","step","model_steps","operation_id","call_id","capability_id","action_id","continuation"] {
                    if let Some(value) = body.get(key).filter(|value|value.is_string() || value.is_number() || value.is_boolean()) {
                        native.insert(key.to_owned(),value.clone());
                    }
                }
                if let Some(digest) = body.pointer("/binding/build_digest").and_then(Value::as_str) {
                    native.insert("binding".into(),json!({"build_digest":digest}));
                }
            }
            projected.insert("event".into(),Value::Object(native));
        }
        events.push(json!({"agent_session_id":event.agent_session_id,"seq":event.seq,"event_id":event.event_id,
            "kind":event.kind.0,"correlation_id":event.correlation_id,"resolved_payload":projected}));
    }
    let source = match metadata.source { CaptureSource::LiveProvider => "live_product", CaptureSource::ScriptedProvider => "scripted_product" };
    let captured = json!({"schema_version":1,"source":source,"suite_id":metadata.suite_id,"trial_id":metadata.trial_id,
        "session_id":session_id,"operation_id":operation_id,"workspace":dunce::canonicalize(workspace)?,
        "runtime_build_digest":metadata.runtime_build_digest,"model":metadata.model,
        "observed_models":metadata.observed_models.iter().collect::<std::collections::BTreeSet<_>>(),
        "duration_ms":metadata.duration_ms,"input_sha256":input_sha256,"events":events});
    if let Some(parent) = output.parent() { std::fs::create_dir_all(parent)?; }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output)?;
    serde_json::to_writer_pretty(&mut file,&captured)?;
    Ok(())
}
