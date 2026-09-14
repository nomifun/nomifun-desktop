//! This engine chooses strict, small request/response budgets, not Coding's
//! compaction, native-round retention, reasoning replay or repair loop.
use std::{collections::BTreeMap, io::Write, time::Duration};

use futures_util::StreamExt;
use nomifun_app::{EngineJournalWrite, EngineTurnJournal};
use nomifun_chat_model_broker::{
    ChatFinishReason, ChatModelEvent, ChatModelRequest, ChatToolCall, EngineModelPort,
};
use nomifun_common::AppError;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::driver::failure;

pub struct Sample {
    pub text: String,
    pub calls: Vec<ChatToolCall>,
}

pub fn bounded_size(value: &impl Serialize, limit: usize) -> Result<usize, AppError> {
    struct Counter(usize, usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let next = self.0.saturating_add(bytes.len());
            if next > self.1 {
                return Err(std::io::Error::other("serialized budget exceeded"));
            }
            self.0 = next;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0, limit);
    serde_json::to_writer(&mut counter, value).map_err(failure)?;
    Ok(counter.0)
}

pub async fn sample(
    port: &dyn EngineModelPort,
    journal: &EngineTurnJournal,
    request: ChatModelRequest,
    input_budget: usize,
    cancellation: &CancellationToken,
) -> Result<Sample, AppError> {
    request.validate().map_err(failure)?;
    bounded_size(&request.input, input_budget)?;
    journal
        .append(
            serde_json::json!({"codec":"evidence-v1", "event":"model_requested",
        "operation":request.causality.operation_id})
            .to_string(),
            Some(request.causality.operation_id.as_ref().to_owned()),
            EngineJournalWrite::Progress,
        )
        .await?;
    // Deadline includes opening and consuming the stream. Cancel the Broker
    // child on every exit, never an already admitted Kernel task.
    let token = cancellation.child_token();
    let _cancel_on_drop = token.clone().drop_guard();
    tokio::select! {
        _ = cancellation.cancelled() => Err(failure("model sampling cancelled")),
        result = tokio::time::timeout(Duration::from_secs(120), consume(port, request, token)) =>
            result.map_err(|_| failure("model sampling deadline exceeded"))?,
    }
}

async fn consume(
    port: &dyn EngineModelPort,
    request: ChatModelRequest,
    token: CancellationToken,
) -> Result<Sample, AppError> {
    let mut stream = port
        .open_stream(request.clone(), token)
        .await
        .map_err(failure)?;
    let mut result = Sample {
        text: String::new(),
        calls: Vec::new(),
    };
    let mut deltas: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut completed = std::collections::BTreeSet::new();
    let mut bytes = 0usize;
    let mut events = 0usize;
    while let Some(event) = stream.next().await {
        let event = event.map_err(failure)?;
        events += 1;
        if events > 8192 {
            return Err(failure("stream event budget exceeded"));
        }
        bytes += bounded_size(&event, (256 * 1024usize).saturating_sub(bytes))?;
        match event {
            ChatModelEvent::OutputTextDelta { text } => {
                if result.text.len() + text.len() > 16 * 1024 {
                    return Err(failure("response text budget exceeded"));
                }
                result.text.push_str(&text);
            }
            ChatModelEvent::ToolCallDelta {
                call_id,
                name,
                arguments_delta,
            } => {
                identity(call_id.as_ref(), &name)?;
                if completed.contains(call_id.as_ref()) {
                    return Err(failure("delta after completed call"));
                }
                if !deltas.contains_key(call_id.as_ref()) && deltas.len() + completed.len() >= 4 {
                    return Err(failure("at most four tool calls per sample"));
                }
                let entry = deltas.entry(call_id.as_ref().to_owned()).or_default();
                if !name.is_empty() {
                    if !entry.0.is_empty() && entry.0 != name {
                        return Err(failure("tool name changed mid-stream"));
                    }
                    entry.0 = name;
                }
                if entry.1.len() + arguments_delta.len() > 8192 {
                    return Err(failure("tool arguments exceed budget"));
                }
                entry.1.push_str(&arguments_delta);
            }
            ChatModelEvent::ToolCallCompleted { call } => {
                identity(call.call_id.as_ref(), &call.name)?;
                call.validate().map_err(failure)?;
                bounded_size(&call, 12 * 1024)?;
                nomifun_engine_core::parse_completed_arguments(&call).map_err(failure)?;
                if !request
                    .input
                    .tools
                    .iter()
                    .any(|tool| tool.name == call.name)
                {
                    return Err(failure("model requested a tool not offered in this phase"));
                }
                if let Some((name, arguments)) = deltas.remove(call.call_id.as_ref()) {
                    if (!name.is_empty() && name != call.name)
                        || (!arguments.is_empty()
                            && serde_json::from_str::<serde_json::Value>(&arguments)
                                .map_err(failure)?
                                != call.arguments.0)
                    {
                        return Err(failure("completed tool differs from its streamed call"));
                    }
                }
                if !completed.insert(call.call_id.as_ref().to_owned())
                    || completed.len() + deltas.len() > 4
                {
                    return Err(failure("duplicate or excessive completed tool call"));
                }
                result.calls.push(call);
            }
            ChatModelEvent::Completed { finish_reason } => {
                if !deltas.is_empty() {
                    return Err(failure("stream ended with incomplete tool calls"));
                }
                let expected = if result.calls.is_empty() {
                    ChatFinishReason::Completed
                } else {
                    ChatFinishReason::ToolCalls
                };
                if finish_reason != expected {
                    return Err(failure(format!(
                        "incomplete/refused sample: {finish_reason:?}"
                    )));
                }
                if result.calls.is_empty() && result.text.trim().is_empty() {
                    return Err(failure("empty model response"));
                }
                return Ok(result);
            }
            ChatModelEvent::OutputAudioDelta { .. }
            | ChatModelEvent::NativeResponsesItem { .. } => {
                return Err(failure(
                    "reference engine supports plain text/function tools only",
                ));
            }
            // Not retained as task evidence or provider continuation state.
            ChatModelEvent::ResponseStarted { .. }
            | ChatModelEvent::ReasoningDelta { .. }
            | ChatModelEvent::ReasoningSignature { .. }
            | ChatModelEvent::ProviderRoundId { .. }
            | ChatModelEvent::Usage { .. } => {}
        }
    }
    Err(failure("model stream ended without a terminal record"))
}

fn identity(id: &str, name: &str) -> Result<(), AppError> {
    if id.is_empty()
        || id.len() > 256
        || name.len() > 128
        || id.chars().any(char::is_control)
        || name.chars().any(char::is_control)
    {
        return Err(failure("invalid model tool identity"));
    }
    Ok(())
}
