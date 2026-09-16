//! Request-local model middleware. The engine remains the only owner of the
//! conversation, tool authority and provider call; middleware returns data.
use std::{collections::BTreeSet, sync::Arc, time::Duration};

use async_trait::async_trait;
use nomi_types::tool::ToolDef;
use serde::{Deserialize, Serialize};

use crate::context_contributor::TurnContext;

pub const MAX_INPUT_BYTES: usize = 256 * 1024;
pub const MAX_PATCH_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct ModelToolMetadata {
    pub name: String,
    pub description: String,
}

/// No history, image bytes, tool schemas, handles or mutable engine references.
#[derive(Clone, Debug, Serialize)]
pub struct BeforeModelInput {
    pub phase: &'static str,
    pub turn: TurnContext,
    pub system: String,
    pub tools: Vec<ModelToolMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_names: Option<Vec<String>>,
}

#[async_trait]
pub trait ModelRequestMiddleware: Send + Sync {
    async fn before_model(&self, input: BeforeModelInput) -> Result<ModelRequestPatch, String>;
    fn label(&self) -> &str;
}

/// A chain is atomic with respect to the provider: failure yields no request.
/// Earlier filters cannot be undone by later middleware. The shared deadline
/// drops the current future, allowing the existing host cancellation to run.
pub(crate) async fn apply(
    middleware: &[Arc<dyn ModelRequestMiddleware>],
    turn: &TurnContext,
    mut system: String,
    mut tools: Vec<ToolDef>,
) -> Result<(String, Vec<ToolDef>), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    for entry in middleware {
        let input = BeforeModelInput {
            phase: "before_model",
            turn: turn.clone(),
            system: system.clone(),
            tools: tools
                .iter()
                .map(|tool| ModelToolMetadata {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                })
                .collect(),
        };
        if serde_json::to_vec(&input).map_err(|e| e.to_string())?.len() > MAX_INPUT_BYTES {
            return Err(format!(
                "before_model '{}' input exceeds 256 KiB",
                entry.label()
            ));
        }
        let patch = tokio::time::timeout_at(deadline, entry.before_model(input))
            .await
            .map_err(|_| format!("before_model '{}' exceeded shared deadline", entry.label()))?
            .map_err(|_| format!("before_model '{}' failed", entry.label()))?;
        if serde_json::to_vec(&patch).map_err(|e| e.to_string())?.len() > MAX_PATCH_BYTES {
            return Err(format!(
                "before_model '{}' patch exceeds 64 KiB",
                entry.label()
            ));
        }
        if let Some(names) = patch.tool_names {
            let mut seen = BTreeSet::new();
            let mut selected = Vec::with_capacity(names.len());
            for name in names {
                if !seen.insert(name.clone()) {
                    return Err("before_model returned duplicate tool names".into());
                }
                let tool = tools.iter().find(|tool| tool.name == name).ok_or_else(|| {
                    "before_model returned a tool outside the current request".to_owned()
                })?;
                selected.push(tool.clone());
            }
            tools = selected;
        }
        if let Some(replacement) = patch.system {
            system = replacement;
        }
    }
    Ok((system, tools))
}
