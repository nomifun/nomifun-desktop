//! This engine replays only completed answer pairs, never tool calls, provider
//! reasoning, another engine's event schema, or an interrupted operation.
use nomifun_agent_contracts::ResolvedSnapshotRef;
use nomifun_ai_agent::engine_sdk::{EngineTurnOutcome, EngineTurnTerminal};
use nomifun_api_types::RuntimeEngineBinding;
use nomifun_app::{EngineSessionHost, EngineTurnReceipt};
use nomifun_common::AppError;
use serde_json::{Value, json};

use super::{driver::failure, model::bounded_size};

pub async fn context(
    host: &EngineSessionHost,
    receipt: &EngineTurnReceipt,
    binding: &RuntimeEngineBinding,
    snapshot: &ResolvedSnapshotRef,
) -> Result<Vec<Value>, AppError> {
    let history = host.read_history(receipt, 6).await?;
    let mut pairs = Vec::new();
    let mut omitted = history.has_older;
    for turn in history.turns {
        // Imported/forked messages are not evidence and this narrow example
        // explicitly refuses to guess a codec for turns without a journal.
        let Some(first) = turn.records.first() else {
            return Err(failure("history without evidence codec is unsupported"));
        };
        let start: Value = serde_json::from_str(&first.event_json).map_err(failure)?;
        if start["codec"] != "evidence-v1"
            || start["event"] != "started"
            || start["binding"] != serde_json::to_value(binding).map_err(failure)?
            || start["snapshot"] != serde_json::to_value(snapshot).map_err(failure)?
            || start["root"].as_str() != Some(turn.root_message_id.as_str())
        {
            return Err(failure("historical engine/Snapshot identity differs"));
        }
        let mut answer = None;
        let mut cleanup = false;
        let mut terminal = None;
        for record in &turn.records[1..] {
            let event: Value = serde_json::from_str(&record.event_json).map_err(failure)?;
            if terminal.is_some() {
                return Err(failure("historical data after terminal"));
            }
            if event["codec"] != "evidence-v1" {
                if !cleanup
                    && matches!(
                        event["event"].as_str(),
                        Some("host_tool_dispatch" | "host_tool_settled")
                    )
                {
                    continue;
                }
                return Err(failure("unsupported historical event codec"));
            }
            match event["event"].as_str() {
                Some("answer") if !cleanup && answer.is_none() => {
                    answer = Some(
                        event["text"]
                            .as_str()
                            .ok_or_else(|| failure("invalid historical answer"))?
                            .to_owned(),
                    )
                }
                Some("cleanup")
                    if !cleanup
                        && event["root"].as_str() == Some(turn.root_message_id.as_str()) =>
                {
                    cleanup = true
                }
                Some("terminal") if cleanup => {
                    terminal = Some(
                        serde_json::from_value::<EngineTurnOutcome>(event["outcome"].clone())
                            .map_err(failure)?,
                    )
                }
                Some("model_requested" | "plan" | "evidence") if !cleanup => {}
                _ => return Err(failure("invalid historical event ordering")),
            }
        }
        let terminal = terminal
            .ok_or_else(|| failure("interrupted history requires platform recovery, not replay"))?;
        if matches!(
            terminal.terminal,
            EngineTurnTerminal::Completed {
                finish_reason: nomifun_chat_model_broker::ChatFinishReason::Completed
            }
        ) {
            let root: Value = serde_json::from_str(&turn.root_content_json).map_err(failure)?;
            let pair = json!({"user":root.get("content").and_then(Value::as_str).ok_or_else(|| failure("historical root is not text"))?,
                "assistant":answer.ok_or_else(|| failure("completed history has no answer"))?});
            pairs.push(pair);
            if bounded_size(&pairs, 6000).is_err() {
                pairs.pop();
                omitted = true;
                break;
            }
        }
    }
    pairs.reverse();
    if omitted {
        pairs.insert(0, json!({"notice":"Older complete turns omitted by this engine's history budget; do not infer their contents."}));
    }
    Ok(pairs)
}
