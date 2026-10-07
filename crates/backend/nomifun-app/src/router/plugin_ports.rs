//! Production App adapters for Unified Plugin Service ports.
//!
//! The Service process owns no database or Desktop authority. These adapters
//! re-check the current Plugin, Artifact and Grant at every call, then lend one
//! narrowly scoped operation to the process for the lifetime of that future.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{PluginBindingPoint, PluginId, PluginManifest, StrictJsonValue};
use nomifun_db::sqlx::Row as _;
use nomifun_db::{SqlitePool, sqlx};
use nomifun_plugin_platform::{
    AgentPluginBindings, AutomationPluginBindings, BindingFailureSemantics,
    BindingMultiplicity, BindingPointContract, DesktopPluginBindings,
    InMemoryPluginBindingRegistry, PassthroughBindingAdapter, PluginActionCallError,
    PluginActionInvocation, PluginActionRuntimePort, PluginBindingError, PluginCancellation,
    PluginDispatchOptions, PluginSecret, PluginServiceActionsPort, PluginServiceCancellation,
    PluginServiceHostPort, PluginServicePortError, PluginServicePorts, PluginServiceRuntime,
    PluginServiceSecretsPort,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use zeroize::{Zeroize as _, Zeroizing};

const DESKTOP_FILES_OPEN: &str = "desktop.files.open";
const ACTIONS_INVOKE: &str = "actions.invoke";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopFileOpenRequest {
    /// Opaque Host-owned file reference. It is intentionally not a path.
    pub file_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopFileOpenResult {
    pub opened: bool,
}

#[async_trait]
pub trait PluginDesktopOwner: Send + Sync {
    async fn open_file(
        &self,
        caller_plugin_id: &PluginId,
        request: DesktopFileOpenRequest,
        cancellation: PluginServiceCancellation,
    ) -> Result<DesktopFileOpenResult, PluginServicePortError>;
}

#[async_trait]
pub trait UnifiedPluginActionDispatcher: Send + Sync {
    async fn invoke(
        &self,
        caller_plugin_id: &PluginId,
        action: &str,
        input: JsonValue,
        call_chain: Vec<String>,
        preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError>;
}

/// No existing Desktop host owns a safe opaque-file resolver yet. Production
/// explicitly reports unavailability instead of turning Plugin input into a
/// raw path or Tauri command authority.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailablePluginDesktopOwner;

#[async_trait]
impl PluginDesktopOwner for UnavailablePluginDesktopOwner {
    async fn open_file(
        &self,
        _caller_plugin_id: &PluginId,
        _request: DesktopFileOpenRequest,
        _cancellation: PluginServiceCancellation,
    ) -> Result<DesktopFileOpenResult, PluginServicePortError> {
        Err(port_error("desktop_capability_unavailable"))
    }
}

/// Late-bound dispatcher breaks the intentional Service -> Action registry ->
/// Service recursion while retaining one process-wide registry instance.
#[derive(Default)]
pub struct RegistryPluginActionDispatcher {
    registry: OnceLock<InMemoryPluginBindingRegistry>,
}

impl std::fmt::Debug for RegistryPluginActionDispatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegistryPluginActionDispatcher")
            .field("installed", &self.registry.get().is_some())
            .finish()
    }
}

impl RegistryPluginActionDispatcher {
    pub fn install(&self, registry: InMemoryPluginBindingRegistry) -> Result<(), &'static str> {
        self.registry
            .set(registry)
            .map_err(|_| "Plugin Action registry was installed more than once")
    }
}

#[async_trait]
impl UnifiedPluginActionDispatcher for RegistryPluginActionDispatcher {
    async fn invoke(
        &self,
        _caller_plugin_id: &PluginId,
        action: &str,
        input: JsonValue,
        call_chain: Vec<String>,
        _preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        let registry = self
            .registry
            .get()
            .ok_or_else(|| port_error("action_dispatch_unavailable"))?;
        let registry_cancellation = PluginCancellation::new();
        let dispatch = registry.dispatch_action(
            action,
            StrictJsonValue(input),
            PluginDispatchOptions {
                expected_artifact_digest: None,
                cancellation: registry_cancellation.clone(),
                call_chain,
            },
        );
        tokio::pin!(dispatch);
        tokio::select! {
            result = &mut dispatch => result
                .map(|value| value.0)
                .map_err(binding_port_error),
            () = wait_service_cancellation(&cancellation) => {
                registry_cancellation.cancel();
                let _ = tokio::time::timeout(Duration::from_secs(1), &mut dispatch).await;
                Err(port_error("service_call_canceled"))
            }
        }
    }
}

/// Action runtime consumed by the registry. It resolves the current Service
/// slot by stable Plugin identity and preserves cancellation to the Node call.
pub struct PluginServiceActionRuntime {
    runtime: Arc<PluginServiceRuntime>,
}

impl PluginServiceActionRuntime {
    pub fn new(runtime: Arc<PluginServiceRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl PluginActionRuntimePort for PluginServiceActionRuntime {
    async fn invoke(
        &self,
        invocation: PluginActionInvocation,
    ) -> Result<StrictJsonValue, PluginActionCallError> {
        let service_cancellation = PluginServiceCancellation::default();
        let call = self.runtime.invoke(
            &invocation.publication.plugin_id,
            &invocation.publication.action_id,
            invocation.input.0,
            invocation.call_chain,
            service_cancellation.clone(),
        );
        tokio::pin!(call);
        tokio::select! {
            result = &mut call => result
                .map(StrictJsonValue)
                .map_err(|error| PluginActionCallError::new("plugin_service_failed", error.to_string())),
            () = invocation.cancellation.cancelled() => {
                service_cancellation.cancel();
                Err(PluginActionCallError::new("canceled", "Plugin Action was canceled"))
            }
        }
    }
}

/// Register the complete stable Binding point set before any Artifact is
/// recovered. The registry keeps the canonical JSON envelope; Agent, Desktop,
/// and Automation consumers apply their domain behavior at their real call
/// sites without changing Plugin identity or persistence.
pub fn register_plugin_binding_owners(
    registry: &InMemoryPluginBindingRegistry,
) -> Result<(), PluginBindingError> {
    for (point, failure) in [
        (PluginBindingPoint::AgentTool, BindingFailureSemantics::FailClosed),
        (PluginBindingPoint::AgentContext, BindingFailureSemantics::Continue),
        (PluginBindingPoint::AgentBeforeModel, BindingFailureSemantics::FailClosed),
        (PluginBindingPoint::AgentBeforeTool, BindingFailureSemantics::FailClosed),
        (PluginBindingPoint::DesktopCommand, BindingFailureSemantics::FailClosed),
        (PluginBindingPoint::DesktopEvent, BindingFailureSemantics::Continue),
        (PluginBindingPoint::AutomationAction, BindingFailureSemantics::FailClosed),
    ] {
        let (input_schema, output_schema) = binding_contract(point);
        registry.register_binding_owner(
            BindingPointContract {
                point,
                input_schema,
                output_schema,
                multiplicity: BindingMultiplicity::Multiple,
                failure,
                timeout: Duration::from_secs(30),
            },
            Arc::new(PassthroughBindingAdapter),
        )?;
    }
    Ok(())
}

fn binding_contract(point: PluginBindingPoint) -> (StrictJsonValue, StrictJsonValue) {
    match point {
        PluginBindingPoint::AgentContext => (
            StrictJsonValue(json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["phase", "turn"],
                "properties": {
                    "phase": {"const": "context"},
                    "turn": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["turn_id", "source_message_id", "text", "image_media_types", "cs_dialogue_id"],
                        "properties": {
                            "turn_id": {"type": "string"},
                            "source_message_id": {"type": "string"},
                            "text": {"type": "string"},
                            "image_media_types": {"type": "array", "items": {"type": "string"}},
                            "cs_dialogue_id": {"type": ["string", "null"]}
                        }
                    }
                }
            })),
            StrictJsonValue(json!({})),
        ),
        PluginBindingPoint::AgentBeforeModel => {
            let action = nomifun_agent_contracts::model_middleware::action();
            let schemas = nomifun_agent_contracts::model_middleware::schemas();
            (
                schemas
                    .get(&action.input_schema)
                    .expect("static before_model input schema")
                    .clone(),
                schemas
                    .get(&action.output_schema)
                    .expect("static before_model output schema")
                    .clone(),
            )
        }
        PluginBindingPoint::AgentBeforeTool => {
            let action = nomifun_agent_contracts::tool_middleware::before_action();
            let schemas = nomifun_agent_contracts::tool_middleware::schemas();
            (
                schemas
                    .get(&action.input_schema)
                    .expect("static before_tool input schema")
                    .clone(),
                schemas
                    .get(&action.output_schema)
                    .expect("static before_tool output schema")
                    .clone(),
            )
        }
        PluginBindingPoint::AgentTool
        | PluginBindingPoint::DesktopCommand
        | PluginBindingPoint::DesktopEvent
        | PluginBindingPoint::AutomationAction => (
            StrictJsonValue(json!({})),
            StrictJsonValue(json!({})),
        ),
    }
}

pub struct PluginBindingConsumers {
    pub registry: InMemoryPluginBindingRegistry,
    pub agent: AgentPluginBindings,
    pub desktop: DesktopPluginBindings,
    pub automation: AutomationPluginBindings,
    pub host: Arc<dyn PluginServiceHostPort>,
}

impl PluginBindingConsumers {
    pub fn new(
        registry: InMemoryPluginBindingRegistry,
        host: Arc<dyn PluginServiceHostPort>,
    ) -> Self {
        Self {
            agent: AgentPluginBindings::new(registry.clone()),
            desktop: DesktopPluginBindings::new(registry.clone()),
            automation: AutomationPluginBindings::new(registry.clone()),
            host,
            registry,
        }
    }
}

/// Compose the exact three ports consumed by the per-Plugin Service process.
pub fn build_plugin_service_ports(
    pool: SqlitePool,
    encryption_key: [u8; 32],
    dispatcher: Arc<dyn UnifiedPluginActionDispatcher>,
    desktop: Arc<dyn PluginDesktopOwner>,
) -> PluginServicePorts {
    let admission = Arc::new(PluginPortAdmission::new(pool.clone()));
    PluginServicePorts {
        secrets: Arc::new(SqlitePluginSecretsPort {
            pool,
            encryption_key,
        }),
        host: Arc::new(ProductionPluginHostPort {
            admission: Arc::clone(&admission),
            desktop,
        }),
        actions: Arc::new(ProductionPluginActionsPort {
            admission,
            dispatcher,
        }),
    }
}

struct SqlitePluginSecretsPort {
    pool: SqlitePool,
    encryption_key: [u8; 32],
}

impl std::fmt::Debug for SqlitePluginSecretsPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqlitePluginSecretsPort")
            .finish_non_exhaustive()
    }
}

impl Drop for SqlitePluginSecretsPort {
    fn drop(&mut self) {
        self.encryption_key.zeroize();
    }
}

#[async_trait]
impl PluginServiceSecretsPort for SqlitePluginSecretsPort {
    async fn get(
        &self,
        plugin_id: &PluginId,
        slot: &str,
        credential_id: &str,
        preview: bool,
    ) -> Result<Option<PluginSecret>, PluginServicePortError> {
        if !valid_slot(slot) {
            return Err(port_error("credential_denied"));
        }
        let plaintext = if preview {
            self.resolve_credential_reference(credential_id).await?
        } else {
            self.resolve_credential_json(plugin_id, slot, credential_id)
                .await?
        };
        Ok(Some(PluginSecret::new(plaintext.as_str())))
    }
}

impl SqlitePluginSecretsPort {
    async fn resolve_credential_json(
        &self,
        plugin_id: &PluginId,
        slot: &str,
        expected_credential_id: &str,
    ) -> Result<Zeroizing<String>, PluginServicePortError> {
        let row = sqlx::query(
            "SELECT binding.credential_id, artifact.manifest_json
             FROM plugin_credential_bindings binding
             JOIN plugins plugin
               ON plugin.plugin_id = binding.plugin_id
              AND plugin.owner_user_id = binding.owner_user_id
             JOIN plugin_artifacts artifact
               ON artifact.artifact_digest = plugin.active_artifact_digest
             JOIN installation_identity installation
               ON installation.singleton_key = 'installation'
              AND installation.owner_user_id = plugin.owner_user_id
             WHERE binding.plugin_id = ? AND binding.slot = ?
               AND plugin.enabled = 1 AND plugin.trashed_at_ms IS NULL",
        )
        .bind(plugin_id.as_ref())
        .bind(slot)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| port_error("credential_unavailable"))?
        .ok_or_else(|| port_error("credential_denied"))?;
        let credential_id: String = row
            .try_get("credential_id")
            .map_err(|_| port_error("credential_invalid"))?;
        if credential_id != expected_credential_id {
            return Err(port_error("credential_denied"));
        }
        let manifest_json: String = row
            .try_get("manifest_json")
            .map_err(|_| port_error("credential_invalid"))?;
        let manifest = PluginManifest::parse(manifest_json.as_bytes())
            .map_err(|_| port_error("credential_invalid"))?;
        if !manifest.secrets.iter().any(|declared| declared == slot) {
            return Err(port_error("credential_denied"));
        }

        self.resolve_credential_reference(&credential_id).await
    }

    async fn resolve_credential_reference(
        &self,
        credential_id: &str,
    ) -> Result<Zeroizing<String>, PluginServicePortError> {
        let encrypted = match parse_credential_reference(credential_id)? {
            HostCredentialReference::Provider(provider_id) => sqlx::query_scalar::<_, String>(
                "SELECT credentials_encrypted FROM providers
                 WHERE provider_id = ? AND enabled = 1",
            )
            .bind(provider_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| port_error("credential_unavailable"))?,
            HostCredentialReference::Connection(connection_id) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT connection.credentials_encrypted
                     FROM provider_connections connection
                     JOIN providers provider ON provider.provider_id = connection.provider_id
                     WHERE connection.connection_id = ? AND provider.enabled = 1",
                )
                .bind(connection_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| port_error("credential_unavailable"))?
            }
        }
        .ok_or_else(|| port_error("credential_denied"))?;
        let plaintext = Zeroizing::new(
            nomifun_common::decrypt_string(&encrypted, &self.encryption_key)
                .map_err(|_| port_error("credential_invalid"))?,
        );
        let parsed: JsonValue = serde_json::from_str(plaintext.as_str())
            .map_err(|_| port_error("credential_invalid"))?;
        if !parsed.is_object() || !credential_json_has_value(&parsed) {
            return Err(port_error("credential_invalid"));
        }
        Ok(plaintext)
    }
}

enum HostCredentialReference<'a> {
    Provider(&'a str),
    Connection(&'a str),
}

pub(crate) async fn validate_plugin_credential_reference(
    pool: &SqlitePool,
    credential_id: &str,
) -> Result<(), PluginServicePortError> {
    match plugin_credential_reference_state(pool, credential_id).await {
        PluginCredentialReferenceState::Available => Ok(()),
        PluginCredentialReferenceState::Missing => Err(port_error("credential_denied")),
        PluginCredentialReferenceState::Invalid => Err(port_error("credential_invalid")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PluginCredentialReferenceState {
    Available,
    Missing,
    Invalid,
}

pub(crate) async fn plugin_credential_reference_state(
    pool: &SqlitePool,
    credential_id: &str,
) -> PluginCredentialReferenceState {
    let enabled = match parse_credential_reference(credential_id) {
        Ok(HostCredentialReference::Provider(provider_id)) => {
            sqlx::query_scalar::<_, bool>("SELECT enabled FROM providers WHERE provider_id = ?")
                .bind(provider_id)
                .fetch_optional(pool)
                .await
        }
        Ok(HostCredentialReference::Connection(connection_id)) => sqlx::query_scalar::<_, bool>(
            "SELECT provider.enabled FROM provider_connections connection \
             JOIN providers provider ON provider.provider_id = connection.provider_id \
             WHERE connection.connection_id = ?",
        )
        .bind(connection_id)
        .fetch_optional(pool)
        .await,
        Err(_) => return PluginCredentialReferenceState::Invalid,
    };
    match enabled {
        Ok(Some(true)) => PluginCredentialReferenceState::Available,
        Ok(Some(false) | None) | Err(_) => PluginCredentialReferenceState::Missing,
    }
}

fn parse_credential_reference(value: &str) -> Result<HostCredentialReference<'_>, PluginServicePortError> {
    let (kind, identity) = value
        .split_once(':')
        .filter(|(_, identity)| !identity.contains(':'))
        .ok_or_else(|| port_error("credential_invalid"))?;
    nomifun_common::validate_uuidv7(identity).map_err(|_| port_error("credential_invalid"))?;
    match kind {
        "provider" => Ok(HostCredentialReference::Provider(identity)),
        "connection" => Ok(HostCredentialReference::Connection(identity)),
        _ => Err(port_error("credential_invalid")),
    }
}

fn credential_json_has_value(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::String(value) => !value.trim().is_empty(),
        JsonValue::Array(values) => values.iter().any(credential_json_has_value),
        JsonValue::Object(values) => values.values().any(credential_json_has_value),
        JsonValue::Bool(_) | JsonValue::Number(_) => true,
    }
}

struct PluginPortAdmission {
    pool: SqlitePool,
}

impl PluginPortAdmission {
    fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn require_grant(
        &self,
        plugin_id: &PluginId,
        permission: &str,
    ) -> Result<PluginManifest, PluginServicePortError> {
        let manifest: Option<String> = sqlx::query_scalar(
            "SELECT artifact.manifest_json
             FROM plugins plugin
             JOIN plugin_artifacts artifact
               ON artifact.artifact_digest = plugin.active_artifact_digest
             JOIN plugin_grants grant
               ON grant.plugin_id = plugin.plugin_id
              AND grant.owner_user_id = plugin.owner_user_id
              AND grant.permission = ?
              AND grant.granted = 1
              AND grant.confirmed_artifact_digest = plugin.active_artifact_digest
             JOIN installation_identity installation
               ON installation.singleton_key = 'installation'
              AND installation.owner_user_id = plugin.owner_user_id
             WHERE plugin.plugin_id = ?
               AND plugin.enabled = 1 AND plugin.trashed_at_ms IS NULL",
        )
        .bind(permission)
        .bind(plugin_id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| port_error("grant_unavailable"))?;
        let manifest = manifest.ok_or_else(|| port_error("grant_denied"))?;
        let manifest = PluginManifest::parse(manifest.as_bytes())
            .map_err(|_| port_error("grant_invalid"))?;
        if !manifest.permissions.contains(permission) {
            return Err(port_error("grant_denied"));
        }
        Ok(manifest)
    }

}

struct ProductionPluginHostPort {
    admission: Arc<PluginPortAdmission>,
    desktop: Arc<dyn PluginDesktopOwner>,
}

#[async_trait]
impl PluginServiceHostPort for ProductionPluginHostPort {
    async fn invoke(
        &self,
        plugin_id: &PluginId,
        capability: &str,
        input: JsonValue,
        preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        if cancellation.is_canceled() {
            return Err(port_error("service_call_canceled"));
        }
        match capability {
            DESKTOP_FILES_OPEN => {
                if !preview {
                    self.admission.require_grant(plugin_id, capability).await?;
                }
                let request: DesktopFileOpenRequest = serde_json::from_value(input)
                    .map_err(|_| port_error("desktop_request_invalid"))?;
                validate_opaque_file_id(&request.file_id)?;
                let result = self
                    .desktop
                    .open_file(plugin_id, request, cancellation.clone())
                    .await?;
                if cancellation.is_canceled() {
                    return Err(port_error("service_call_canceled"));
                }
                Ok(json!({"opened": result.opened}))
            }
            _ => Err(port_error("host_capability_denied")),
        }
    }
}

struct ProductionPluginActionsPort {
    admission: Arc<PluginPortAdmission>,
    dispatcher: Arc<dyn UnifiedPluginActionDispatcher>,
}

#[async_trait]
impl PluginServiceActionsPort for ProductionPluginActionsPort {
    async fn invoke(
        &self,
        caller_plugin_id: &PluginId,
        action: &str,
        input: JsonValue,
        call_chain: Vec<String>,
        preview: bool,
        cancellation: PluginServiceCancellation,
    ) -> Result<JsonValue, PluginServicePortError> {
        if cancellation.is_canceled() {
            return Err(port_error("service_call_canceled"));
        }
        validate_action_identity(action)?;
        if !preview {
            self.admission
                .require_grant(caller_plugin_id, ACTIONS_INVOKE)
                .await?;
        }
        let result = self
            .dispatcher
            .invoke(
                caller_plugin_id,
                action,
                input,
                call_chain,
                preview,
                cancellation.clone(),
            )
            .await?;
        if cancellation.is_canceled() {
            Err(port_error("service_call_canceled"))
        } else {
            Ok(result)
        }
    }
}

fn validate_action_identity(value: &str) -> Result<(), PluginServicePortError> {
    let (plugin_id, action_id) = value
        .strip_prefix("plugin:")
        .and_then(|value| value.split_once('/'))
        .filter(|(_, action)| !action.contains('/'))
        .ok_or_else(|| port_error("action_invalid"))?;
    nomifun_common::validate_uuidv7(plugin_id).map_err(|_| port_error("action_invalid"))?;
    let valid_action = !action_id.is_empty()
        && action_id.len() <= 96
        && action_id.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase())
        && action_id.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.')
        });
    if valid_action {
        Ok(())
    } else {
        Err(port_error("action_invalid"))
    }
}

fn validate_opaque_file_id(value: &str) -> Result<(), PluginServicePortError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
        });
    if valid {
        Ok(())
    } else {
        Err(port_error("desktop_request_invalid"))
    }
}

fn valid_slot(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-'))
}

async fn wait_service_cancellation(cancellation: &PluginServiceCancellation) {
    while !cancellation.is_canceled() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn binding_port_error(error: PluginBindingError) -> PluginServicePortError {
    match error {
        PluginBindingError::Canceled => port_error("service_call_canceled"),
        PluginBindingError::ActionNotFound(_)
        | PluginBindingError::ActionUnavailable { .. }
        | PluginBindingError::ActionNotBound { .. } => port_error("action_unavailable"),
        PluginBindingError::RecursiveCall(_) | PluginBindingError::CallDepthExceeded => {
            port_error("action_call_chain_rejected")
        }
        PluginBindingError::Timeout => port_error("action_timeout"),
        _ => port_error("action_dispatch_failed"),
    }
}

fn port_error(code: &str) -> PluginServicePortError {
    PluginServicePortError::new(code)
}

#[cfg(test)]
#[path = "plugin_ports_tests.rs"]
mod tests;
