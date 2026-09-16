//! Independent system-browser Tool contract. It does not imply native
//! Workspace, local-websearch, or arbitrary protocol/evaluation authority.
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

pub const TOOL_NAME: &str = "nomi_system_browser";
pub const BINDING_ANNOTATION: &str = "x-nomifun-system-browser-binding";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SystemBrowserBinding {
    pub schema_version: u32,
    pub runtime_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum SystemBrowserCommand {
    Tabs {},
    Observe {
        tab_id: String,
    },
    Navigate {
        tab_id: String,
        url: String,
    },
    Dialog {
        tab_id: String,
        dialog_id: String,
        accept: bool,
        prompt_text: Option<String>,
    },
    Click {
        tab_id: String,
        observation_id: String,
        ref_id: String,
    },
    Type {
        tab_id: String,
        observation_id: String,
        ref_id: String,
        text: String,
        #[serde(default)]
        replace: bool,
    },
    Press {
        tab_id: String,
        observation_id: String,
        keys: String,
    },
    Scroll {
        tab_id: String,
        observation_id: String,
        delta_x: f64,
        delta_y: f64,
    },
}

pub fn input_schema() -> Value {
    let id = json!({"type":"string","minLength":1,"maxLength":128});
    let mut variants = Vec::new();
    for operation in [
        "tabs", "observe", "navigate", "dialog", "click", "type", "press", "scroll",
    ] {
        let mut properties = json!({"operation":{"const":operation,"type":"string"}});
        let mut required = vec!["operation"];
        if operation != "tabs" {
            properties["tab_id"] = id.clone();
            required.push("tab_id");
        }
        if matches!(operation, "click" | "type" | "press" | "scroll") {
            properties["observation_id"] = id.clone();
            required.push("observation_id");
        }
        if matches!(operation, "click" | "type") {
            properties["ref_id"] = id.clone();
            required.push("ref_id");
        }
        match operation {
            "dialog" => {
                properties["dialog_id"] = id.clone();
                properties["accept"] = json!({"type":"boolean"});
                properties["prompt_text"] = json!({"type":["string","null"],"maxLength":4096});
                required.extend(["dialog_id", "accept"]);
            }
            "navigate" => {
                properties["url"] = json!({"type":"string","minLength":1,"maxLength":8192});
                required.push("url");
            }
            "type" => {
                properties["text"] = json!({"type":"string","maxLength":16384});
                properties["replace"] = json!({"type":"boolean","default":false});
                required.push("text");
            }
            "press" => {
                properties["keys"] = json!({"type":"string","minLength":1,"maxLength":128,"description":"Page keys such as Tab, Enter, ArrowDown or Ctrl+A. Browser, window and system shortcuts are unavailable."});
                required.push("keys");
            }
            "scroll" => {
                for key in ["delta_x", "delta_y"] {
                    properties[key] = json!({"type":"number","minimum":-10000,"maximum":10000});
                    required.push(key);
                }
            }
            _ => {}
        }
        variants.push(json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}));
    }
    json!({"type":"object","oneOf":variants})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SystemBrowserRuntimeError {
    #[error("Connect the system browser and authorize a tab in this conversation first")]
    Unavailable,
    #[error("This system-browser run is no longer active")]
    StaleRun,
    #[error("The browser tab is in use or has an unfinished browser operation")]
    Busy,
    #[error("The browser connection was lost; reconnect explicitly")]
    Disconnected,
    #[error("The tab is not authorized for this conversation")]
    TabDenied,
    #[error("The browser request or observation reference is invalid or stale")]
    InvalidInput,
    #[error(
        "The browser operation failed; observe the current page before deciding whether to retry"
    )]
    ExecutionFailed,
    #[error("The browser run was cancelled")]
    Cancelled,
}

#[async_trait]
pub trait SystemBrowserHost: Send + Sync {
    fn binding(&self) -> SystemBrowserBinding;
    /// Host checks ownership. Binding a workspace must not connect a browser.
    async fn workspace(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Arc<dyn SystemBrowserWorkspace>, SystemBrowserRuntimeError>;
}
#[async_trait]
pub trait SystemBrowserWorkspace: Send + Sync {
    async fn begin_run(&self) -> Result<Arc<dyn SystemBrowserTurn>, SystemBrowserRuntimeError>;
}
#[async_trait]
pub trait SystemBrowserTurn: Send + Sync {
    fn cancel(&self);
    async fn invoke(
        &self,
        command: SystemBrowserCommand,
    ) -> Result<Value, SystemBrowserRuntimeError>;
    async fn settle(&self) -> Result<(), SystemBrowserRuntimeError>;
    async fn finish(&self) -> Result<(), SystemBrowserRuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_contract_is_bounded_and_requires_an_exact_dialog_identity() {
        let schema = input_schema();
        let dialog = schema["oneOf"].as_array().unwrap().iter()
            .find(|variant| variant["properties"]["operation"]["const"] == "dialog").unwrap();
        assert_eq!(dialog["additionalProperties"], false);
        assert_eq!(dialog["required"], json!(["operation", "tab_id", "dialog_id", "accept"]));
        for key in ["tab_id", "dialog_id"] {
            assert_eq!(dialog["properties"][key], json!({"type":"string","minLength":1,"maxLength":128}));
        }
        assert_eq!(dialog["properties"]["prompt_text"], json!({"type":["string","null"],"maxLength":4096}));
        assert_eq!(dialog["properties"]["accept"], json!({"type":"boolean"}));
        assert_eq!(dialog["properties"].as_object().unwrap().len(), 5);
        for input in [
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":false,"prompt_text":null}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true,"prompt_text":"中文回复"}),
        ] {
            assert!(serde_json::from_value::<SystemBrowserCommand>(input).is_ok());
        }
        for input in [
            json!({"operation":"dialog","tab_id":"tab","accept":true}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog"}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":"true"}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true,"prompt_text":1}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true,"observation_id":"old"}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true,"sessionId":"raw-session"}),
        ] {
            assert!(serde_json::from_value::<SystemBrowserCommand>(input).is_err());
        }
    }
}
