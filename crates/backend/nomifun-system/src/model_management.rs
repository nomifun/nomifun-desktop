//! Conversation-facing, additive model configuration. This facade uses exactly
//! the same validated/encrypted services as the Model Management editor.
use std::sync::Arc;

use nomifun_api_types::{CreateProviderRequest, ModelTask, SaveProviderModelRequest};
use nomifun_common::AppError;
use nomifun_net::secret_redaction::SecretRedactor;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ProviderConnectionService, ProviderModelService, ProviderService};

pub const CAPABILITY_ID: &str = "model.management";
pub const INSPECT: &str = "model.management/inspect";
pub const CREATE_PROVIDER: &str = "model.management/create_provider";
pub const ADD_MODEL: &str = "model.management/add_model";

#[derive(Clone)]
pub struct ModelManagementService {
    providers: ProviderService,
    models: ProviderModelService,
    connections: ProviderConnectionService,
    writes: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Inspect {
    List {
        provider_id: Option<String>,
    },
    Protocols {
        platform: String,
        task: ModelTask,
        base_url: Option<String>,
        model: Option<String>,
    },
}

impl ModelManagementService {
    pub fn new(
        providers: ProviderService,
        models: ProviderModelService,
        connections: ProviderConnectionService,
    ) -> Self {
        Self {
            providers,
            models,
            connections,
            writes: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// The caller must first authorize the local installation owner and exact
    /// Agent Action. No credential-reading or configuration-deletion operation
    /// is exposed by this facade.
    pub async fn execute(&self, action: &str, input: Value) -> Result<Value, String> {
        let mut secrets = Vec::new();
        credential_values(&input, false, &mut secrets);
        let redactor = SecretRedactor::new(secrets);
        let result = self.execute_inner(action, input).await;
        result
            .map(|mut value| {
                // Outputs are explicitly allowlisted above/below and contain
                // no credential fields. Only strip URL credential material:
                // replacing arbitrary key substrings here could corrupt UUIDs
                // and model IDs when a local service uses a short API token.
                redact_values(&mut value, &SecretRedactor::default());
                value
            })
            .map_err(|error| redactor.redact(&error.to_string()))
    }

    async fn execute_inner(&self, action: &str, input: Value) -> Result<Value, AppError> {
        match action {
            INSPECT => match parse(input)? {
                Inspect::List { provider_id } => {
                    let providers = self.providers.list().await?;
                    if let Some(id) = &provider_id {
                        if !providers.iter().any(|provider| &provider.provider_id == id) {
                            return Err(AppError::NotFound("Provider not found".into()));
                        }
                    }
                    let mut result = Vec::new();
                    for provider in providers
                        .into_iter()
                        .filter(|p| provider_id.as_ref().is_none_or(|id| id == &p.provider_id))
                    {
                        // Deliberate allowlist: never project saved credentials,
                        // arbitrary extra/params, or upstream diagnostic prose.
                        let connections = self.connections.list(&provider.provider_id).await?.into_iter().map(|c| json!({
                            "role":c.role, "base_url":c.base_url, "auth_scheme":c.auth_scheme, "has_credentials":c.has_credentials
                        })).collect::<Vec<_>>();
                        let models = provider.models.into_iter().map(|m| json!({
                            "model":m.model, "display_name":m.display_name, "enabled":m.enabled,
                            "capabilities":m.capabilities.into_iter().map(|c| json!({
                                "task":c.task, "protocol":c.protocol, "connection_role":c.connection_role,
                                "traits":c.traits, "context_limit":c.context_limit, "output_limit":c.output_limit
                            })).collect::<Vec<_>>()
                        })).collect::<Vec<_>>();
                        result.push(json!({"provider_id":provider.provider_id,"name":provider.name,
                            "platform":provider.platform,"base_url":provider.base_url,"auth_scheme":provider.auth_scheme,
                            "enabled":provider.enabled,"has_credentials":provider.has_credentials,"models":models,"connections":connections}));
                    }
                    Ok(json!({"providers":result}))
                }
                Inspect::Protocols {
                    platform,
                    task,
                    base_url,
                    model,
                } => Ok(json!({"manifest":
                    nomifun_model_invoke::protocol_manifest_for_model_connection(&platform, base_url.as_deref(), model.as_deref(), task)
                })),
            },
            CREATE_PROVIDER => {
                let request: CreateProviderRequest = parse(input)?;
                let _guard = self.writes.lock().await;
                if self.providers.list().await?.iter().any(|p| {
                    request.provider_id.as_ref() == Some(&p.provider_id)
                        || (p.name == request.name.trim()
                            && p.base_url.trim_end_matches('/')
                                == request.base_url.trim().trim_end_matches('/'))
                }) {
                    return Err(AppError::Conflict("Provider already exists; inspect the list and use add_model with its provider_id. Existing credentials were not changed".into()));
                }
                let provider = self.providers.create(request).await?;
                Ok(
                    json!({"status":"created","provider_id":provider.provider_id,"name":provider.name,
                    "models":provider.models.iter().map(|m| &m.model).collect::<Vec<_>>(),
                    "has_credentials":provider.has_credentials,"connection_tested":false}),
                )
            }
            ADD_MODEL => {
                let request: SaveProviderModelRequest = parse(input)?;
                let _guard = self.writes.lock().await;
                let model = self.models.create(request).await?;
                Ok(
                    json!({"status":"created","provider_id":model.provider_id,"model":model.model,"connection_tested":false}),
                )
            }
            _ => Err(AppError::BadRequest(
                "Unknown model management action".into(),
            )),
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, AppError> {
    serde_json::from_value(input).map_err(|error| AppError::BadRequest(error.to_string()))
}

fn credential_values(value: &Value, sensitive: bool, secrets: &mut Vec<String>) {
    match value {
        Value::String(value) if sensitive => secrets.push(value.clone()),
        Value::Array(values) => values
            .iter()
            .for_each(|v| credential_values(v, sensitive, secrets)),
        Value::Object(values) => values
            .iter()
            .for_each(|(k, v)| credential_values(v, sensitive || k == "credentials", secrets)),
        _ => {}
    }
}

fn redact_values(value: &mut Value, redactor: &SecretRedactor) {
    match value {
        Value::String(text) => *text = redactor.redact(text),
        Value::Array(values) => values.iter_mut().for_each(|v| redact_values(v, redactor)),
        Value::Object(values) => values.values_mut().for_each(|v| redact_values(v, redactor)),
        _ => {}
    }
}
