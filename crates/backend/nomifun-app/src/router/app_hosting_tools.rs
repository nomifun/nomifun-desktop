//! Small application-management tools for ordinary coding Sessions.
//! Every operation uses the same hosting owner as the application UI.
use std::{collections::{BTreeMap, BTreeSet}, path::Path, sync::Arc};
use async_trait::async_trait;
use nomifun_agent_contracts::*;
use nomifun_agent_domain_support::{CapabilitySpec, PackageSpec};
use nomifun_agent_kernel::{CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration};
use nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver;
use nomifun_app_hosting::{AppDetails, AppOperation, AppSettingsPatch, HostingOwner, ImportKind};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) const MODULE_ID: &str = "app.hosting";
pub(crate) const ACTIONS: &[&str] = &["app.hosting/guide", "app.hosting/current", "app.hosting/import", "app.hosting/list", "app.hosting/update", "app.hosting/preview", "app.hosting/rebuild", "app.hosting/start", "app.hosting/status", "app.hosting/stop"];
const DESCRIPTION: &str = "NomiFun application development: build, import, configure, preview and run ordinary applications, including apps that communicate with NomiFun. Call read_app_guide on demand before application development. Call current_app to identify an application already owned by the current workspace; edit its actual files and update_app settings instead of importing another copy. Use already-granted coding tools. A running process is not proof that business functions work. These tools are available only to Agents that select this capability, including the built-in application Agent.";
const DEVELOPMENT_GUIDE: &str = include_str!("../../../../../docs/guides/communicating-app-development.zh.md");

fn action_effect(action: &str) -> EffectClass {
    if action == "app.hosting/guide" { EffectClass::ReadLocal }
    else if matches!(action, "app.hosting/current" | "app.hosting/list" | "app.hosting/status") {
        EffectClass::ReadSensitive
    } else { EffectClass::WriteDurable }
}

pub(super) fn capability_ids() -> BTreeSet<CapabilityId> { BTreeSet::from([MODULE_ID.into()]) }

pub(super) fn registration(services: &crate::services::AppServices,
    session_owner: std::sync::Weak<super::nomi_core_session::NomiCoreSessionOwner>,
    workspace_owner: Arc<super::nomi_core_wave2::NomiCoreWave2Host>) -> anyhow::Result<PluginRegistration> {
    const CAPABILITIES: &[CapabilitySpec] = &[CapabilitySpec::tool(MODULE_ID, EffectClass::WriteDurable, &[])];
    let base = nomifun_agent_domain_support::registration(PackageSpec {
        id: "nomifun.app-hosting", mount_id: "app-hosting", display_name: "NomiFun application development", description: DESCRIPTION,
        capabilities: CAPABILITIES, supported_surfaces: &["desktop", "headless"],
    })?;
    let mut metadata = base.metadata;
    let port = HostPortRef { id: "host.app-hosting.invoke".into(), version: "1.0.0".into() };
    let capability = &mut metadata.manifest.payload.contributions.capabilities[0];
    capability.display.name = "NomiFun application development / NomiFun 应用开发".into();
    capability.contributions.host_ports = vec![port.clone()];
    capability.contributions.actions = ACTIONS.iter().map(|action| CapabilityActionDescriptor {
        action_id: (*action).into(), input_schema: schema_ref(action, "input", &input_schema(action)),
        output_schema: schema_ref(action, "output", &json!({"type":"object"})),
        effect_class: action_effect(action),
        presentation: ToolPresentationKind::FunctionTool,
    }).collect();
    metadata.manifest = ArtifactEnvelope::new(metadata.manifest.payload)?;
    metadata.registrar.allowed_operations.insert(PluginRegistrarOperation::BindHostPort);
    metadata.registrar.declared_host_ports.insert(port.id.clone());
    metadata.context.host_ports.push(HostPortBindingDescriptor {
        port, request_schema: schema_ref(MODULE_ID, "host-request", &json!({"type":"object"})),
        response_schema: schema_ref(MODULE_ID, "host-response", &json!({"type":"object"})),
    });
    let mut registration = PluginRegistration::new(metadata);
    registration.add_capability_handler(MODULE_ID.into(), Arc::new(Owner {
        owner_id: services.authoritative_user_id.clone(), hosting: services.app_hosting.clone(),
        events: services.event_bus.clone(),
        mcp: services.app_mcp_sources.clone(),
        receipts: super::hosted_effect_receipts::HostedEffectReceipts::new(services.database.pool().clone()),
        background: services.background_tasks.clone(),
        session_owner, workspace_owner,
    }))?;
    Ok(registration)
}

#[derive(Clone)]
struct Owner {
    owner_id: Arc<str>, hosting: Arc<HostingOwner>, events: Arc<dyn nomifun_realtime::UserEventSink>,
    mcp: Arc<super::app_mcp_sources::AppMcpSources>,
    receipts: super::hosted_effect_receipts::HostedEffectReceipts,
    background: Arc<crate::services::BackgroundTaskRegistry>,
    session_owner: std::sync::Weak<super::nomi_core_session::NomiCoreSessionOwner>,
    workspace_owner: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
}

struct OwnerFailure { reason: String, outcome_known: bool }
impl OwnerFailure {
    fn hosting(error: nomifun_app_hosting::AppError) -> Self {
        Self { outcome_known: !matches!(error, nomifun_app_hosting::AppError::OutcomeUnknown(_)), reason: error.to_string() }
    }
    fn kernel(self) -> KernelError { KernelError::CapabilityExecution { reason: self.reason } }
}

/// Only ordinary running settings are authorable by the management tool.
/// Connection references and development Session associations stay with their owners.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ImportSettings {
    project_directory: Option<String>,
    #[serde(deserialize_with = "command_override")]
    install_command: Option<Option<String>>,
    #[serde(deserialize_with = "command_override")]
    preview_command: Option<Option<String>>,
    #[serde(deserialize_with = "command_override")]
    build_command: Option<Option<String>>,
    #[serde(deserialize_with = "command_override")]
    output_directory: Option<Option<String>>,
    #[serde(deserialize_with = "command_override")]
    start_command: Option<Option<String>>,
    #[serde(deserialize_with = "command_override")]
    web_url: Option<Option<String>>,
    #[serde(deserialize_with = "command_override")]
    node_path: Option<Option<String>>,
    environment: Option<BTreeMap<String,String>>,
    #[serde(deserialize_with = "command_override")]
    mcp_url: Option<Option<String>>,
}

fn command_override<'de,D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

impl ImportSettings {
    fn into_patch(self) -> AppSettingsPatch {
        AppSettingsPatch {
            project_directory: self.project_directory, install_command: self.install_command,
            preview_command: self.preview_command, build_command: self.build_command,
            output_directory: self.output_directory, start_command: self.start_command,
            web_url: self.web_url, node_path: self.node_path, environment: self.environment,
            mcp_url: self.mcp_url, agent_id: None,
        }
    }
}

#[async_trait]
impl CapabilityHandler for Owner {
    async fn invoke(&self, context: CapabilityInvocationContext, input: StrictJsonValue) -> Result<StrictJsonValue, KernelError> {
        let fail = |reason: String| KernelError::CapabilityExecution { reason };
        if context.principal.principal_kind != "user" || context.principal.principal_id != self.owner_id.as_ref()
            || context.capability_id.as_ref() != MODULE_ID || !ACTIONS.contains(&context.action_id.as_ref()) {
            return Err(fail("Application management requires the local installation owner and an exact granted action".into()));
        }
        let schema = jsonschema::validator_for(&input_schema(context.action_id.as_ref())).map_err(|error| fail(error.to_string()))?;
        if !schema.is_valid(&input.0) { return Err(fail("Application management input does not match its granted action".into())); }
        if context.action_id.as_ref() == "app.hosting/import"
            && !Path::new(input.0["path"].as_str().expect("schema requires path")).is_absolute() {
            return Err(fail("import_app requires the project's absolute folder or ZIP path; relative paths are not resolved against the Agent workspace".into()));
        }
        if context.action_id.as_ref() == "app.hosting/import" && input.0.get("app_id").is_none() {
            if let Some(current) = self.current_application(&context).await.map_err(OwnerFailure::kernel)? {
                return Err(fail(format!("This workspace already belongs to application {}. Import external files into this same application by passing its app_id; do not create a duplicate application or import the current directory into itself.",current.id)));
            }
        }
        if action_effect(context.action_id.as_ref()) != EffectClass::WriteDurable {
            return self.invoke_owned(&context, input).await.map_err(OwnerFailure::kernel);
        }
        // The Host retains mutation and settlement together. Cancelling an
        // Agent waiter cannot erase an imported copy or its canonical outcome.
        let owner = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if !self.background.spawn(async move {
            let result = owner.invoke_managed(context, input).await;
            let _ = sender.send(result);
        }) {
            return Err(fail("Host is shutting down; application operation was not admitted".into()));
        }
        receiver.await.map_err(|_| fail("Application operation outcome is unknown; do not repeat it automatically".into()))?
    }
}

impl Owner {
    async fn current_application(&self, context: &CapabilityInvocationContext) -> Result<Option<nomifun_app_hosting::AppRecord>, OwnerFailure> {
        let fail = |reason: String| OwnerFailure { reason, outcome_known: true };
        let Some(workspace) = self.receipts.session_workspace(context).await.map_err(|error| fail(error.to_string()))? else { return Ok(None); };
        application_for_workspace(&self.hosting, &workspace).await
    }

    async fn invoke_managed(&self, context: CapabilityInvocationContext, input: StrictJsonValue) -> Result<StrictJsonValue, KernelError> {
        let fail = |error: nomifun_common::AppError| KernelError::CapabilityExecution { reason: error.to_string() };
        let receipt = self.receipts.begin_invocation(&context, &input.0, super::hosted_effect_receipts::Domain::AppHosting).await.map_err(fail)?;
        let _import_fence = if context.action_id.as_ref() == "app.hosting/import" {
            if let Some(app_id) = input.0.get("app_id").and_then(Value::as_str) {
                let guarded = async {
                    let owner = self.session_owner.upgrade().ok_or_else(|| nomifun_common::AppError::Conflict("Application project Session owner is unavailable".into()))?;
                    let target = self.hosting.get(app_id).await.map_err(|error| nomifun_common::AppError::Conflict(error.to_string()))?;
                    super::nomi_core_session::guard_agent_app_project_import(&owner, &self.workspace_owner, &context, receipt.exact_request(), &target).await
                }.await;
                match guarded {
                    Ok(fence) => Some(fence),
                    Err(error) => {
                        self.receipts.rejected(receipt, "APP_PROJECT_IMPORT_FENCED").await.map_err(fail)?;
                        return Err(fail(error));
                    }
                }
            } else { None }
        } else { None };
        match self.invoke_owned(&context, input).await {
            Ok(output) => { self.receipts.returned(receipt, &output.0).await.map_err(fail)?; Ok(output) }
            Err(error) => {
                if error.outcome_known {
                    // Failure after owner admission may leave a registered app
                    // or changed files. It is never a no-effect/retry promise.
                    self.receipts.failed(receipt, &json!({"owner_error":error.reason,"partial_effect_possible":true})).await.map_err(fail)?;
                    self.events.send_to_user(&self.owner_id, nomifun_api_types::WebSocketMessage::new("apps.changed", json!({})));
                }
                Err(error.kernel())
            }
        }
    }

    async fn invoke_owned(&self, context: &CapabilityInvocationContext, input: StrictJsonValue) -> Result<StrictJsonValue, OwnerFailure> {
        let fail = |reason: String| OwnerFailure { reason, outcome_known: true };
        let app_id = input.0.get("app_id").and_then(Value::as_str).unwrap_or_default();
        let manages_source = matches!(context.action_id.as_ref(), "app.hosting/update" | "app.hosting/preview" | "app.hosting/rebuild" | "app.hosting/start" | "app.hosting/stop")
            || context.action_id.as_ref() == "app.hosting/import" && !app_id.is_empty();
        let changes_mcp_source = manages_source
            && self.mcp.is_app_source(app_id).await;
        let mut source_warning = None;
        if context.action_id.as_ref() == "app.hosting/stop" && changes_mcp_source {
            self.mcp.remove_observation(app_id).map_err(|error| fail(error.to_string()))?;
            if let Err(error) = self.mcp.publish().await { source_warning = Some(error.to_string()); }
        }
        let mut result = match context.action_id.as_ref() {
            "app.hosting/guide" => json!({"instructions":DEVELOPMENT_GUIDE}),
            "app.hosting/current" => json!({"application":self.current_application(context).await?}),
            "app.hosting/list" => json!({"applications":self.hosting.list().await.map_err(OwnerFailure::hosting)?}),
            "app.hosting/status" => json!({"application":self.hosting.get(app_id).await.map_err(OwnerFailure::hosting)?}),
            "app.hosting/import" => {
                let source = input.0["path"].as_str().expect("schema requires path");
                let name = input.0.get("name").and_then(Value::as_str).map(str::to_owned);
                let kind = if input.0.get("import_kind").and_then(Value::as_str) == Some("artifact") { ImportKind::Artifact } else { ImportKind::Source };
                let overrides = input.0.get("settings").map(|settings| serde_json::from_value::<ImportSettings>(settings.clone()))
                    .transpose().map_err(|error| fail(error.to_string()))?;
                let mut app = if app_id.is_empty() { self.hosting.import(Path::new(source),name,kind).await }
                    else { self.hosting.import_into(app_id,Path::new(source),name,kind).await }.map_err(OwnerFailure::hosting)?;
                if changes_mcp_source { self.mcp.remove_observation(app_id).map_err(|error| fail(error.to_string()))?; }
                self.events.send_to_user(&self.owner_id, nomifun_api_types::WebSocketMessage::new("apps.changed", json!({"app_id":app.id})));
                if let Some(overrides) = overrides {
                    app = self.hosting.update_patch_details(&app.id,None,AppDetails::default(),overrides.into_patch(),false).await.map_err(|error| fail(format!(
                        "Application {} was imported into its managed copy, but its running settings were rejected: {error}. Its registration was preserved; correct the settings before starting it.",app.id)))?;
                }
                json!({"application":app})
            },
            "app.hosting/update" => {
                let name = input.0.get("name").and_then(Value::as_str).map(str::to_owned);
                let settings = input.0.get("settings").map(|overrides| serde_json::from_value::<ImportSettings>(overrides.clone()))
                    .transpose().map_err(|error| fail(error.to_string()))?.unwrap_or_default().into_patch();
                let details: AppDetails = serde_json::from_value(json!({
                    "description":input.0.get("description"), "tags":input.0.get("tags"), "import_kind":input.0.get("import_kind"),
                })).map_err(|error| fail(error.to_string()))?;
                let source_setting_present = settings.mcp_url.is_some();
                let detect_settings = input.0.get("detect_settings").and_then(Value::as_bool).unwrap_or(false);
                let app = self.hosting.update_patch_details(app_id,name,details,settings,detect_settings).await.map_err(OwnerFailure::hosting)?;
                if source_setting_present && changes_mcp_source { self.mcp.remove_observation(app_id).map_err(|error| fail(error.to_string()))?; }
                json!({"application":app})
            },
            "app.hosting/preview" | "app.hosting/rebuild" => {
                if changes_mcp_source {
                    self.mcp.remove_observation(app_id).map_err(|error| fail(error.to_string()))?;
                    if let Err(error) = self.mcp.publish().await { source_warning = Some(error.to_string()); }
                }
                let operation = if context.action_id.as_ref() == "app.hosting/preview" { AppOperation::Preview } else { AppOperation::Rebuild };
                json!({"application":self.hosting.operate(app_id,operation).await.map_err(OwnerFailure::hosting)?})
            },
            "app.hosting/start" => {
                let app = self.hosting.get(app_id).await.map_err(OwnerFailure::hosting)?;
                if changes_mcp_source && (app.state != nomifun_app_hosting::AppState::Running || app.preview) {
                    self.mcp.remove_observation(app_id).map_err(|error| fail(error.to_string()))?;
                    if let Err(error) = self.mcp.publish().await { source_warning = Some(error.to_string()); }
                }
                let operation = if app.prepared && !app.preview { AppOperation::Start } else { AppOperation::Use };
                json!({"application":self.hosting.operate(app_id,operation).await.map_err(OwnerFailure::hosting)?})
            },
            "app.hosting/stop" => json!({"application":self.hosting.operate(app_id,AppOperation::Stop).await.map_err(OwnerFailure::hosting)?}),
            _ => unreachable!("exact granted action checked"),
        };
        if let Some(warning) = source_warning { result["mcp_warning"] = json!(warning); }
        let current_mcp_source = manages_source
            && self.mcp.is_app_source(app_id).await;
        if matches!(context.action_id.as_ref(), "app.hosting/update" | "app.hosting/preview" | "app.hosting/rebuild" | "app.hosting/start") && current_mcp_source {
            if let Err(error) = self.mcp.refresh(app_id).await.and_then(|source| {
                if source.connection_status == "error" { Err(nomifun_common::AppError::Conflict(source.reason.unwrap_or_else(|| "Application MCP connection failed".into()))) } else { Ok(source) }
            }) {
                result["mcp_warning"] = json!(error.to_string());
            }
        }
        if context.action_id.as_ref() == "app.hosting/stop" && changes_mcp_source { self.mcp.remove_observation(app_id).map_err(|error| fail(error.to_string()))?; }
        if changes_mcp_source || current_mcp_source {
            if let Err(error) = self.mcp.publish().await { result["mcp_warning"] = json!(error.to_string()); }
        }
        if action_effect(context.action_id.as_ref()) == EffectClass::WriteDurable {
            self.events.send_to_user(&self.owner_id, nomifun_api_types::WebSocketMessage::new("apps.changed", json!({})));
        }
        Ok(StrictJsonValue(result))
    }
}

async fn application_for_workspace(hosting: &HostingOwner, workspace: &Path) -> Result<Option<nomifun_app_hosting::AppRecord>, OwnerFailure> {
    let fail = |reason: String| OwnerFailure { reason, outcome_known: true };
    let workspace = tokio::fs::canonicalize(workspace).await.map_err(|error| fail(format!("Current workspace is unavailable: {error}")))?;
    let mut current = None;
    for application in hosting.list().await.map_err(OwnerFailure::hosting)? {
        let Ok(directory) = tokio::fs::canonicalize(&application.directory).await else { continue; };
        if directory == workspace {
            if current.is_some() { return Err(fail("More than one application owns the current workspace".into())); }
            current = Some(application);
        }
    }
    Ok(current)
}

pub(super) struct SchemaResolver;
#[async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for SchemaResolver {
    async fn resolve(&self, capability: &ResolvedCapability, reference: &CanonicalSchemaRef) -> Result<StrictJsonValue, String> {
        if capability.capability.id.as_ref() == MODULE_ID {
            for action in ACTIONS {
                let schema = input_schema(action);
                if &schema_ref(action, "input", &schema) == reference { return Ok(StrictJsonValue(schema)); }
            }
        }
        Err("Unknown application hosting action schema".into())
    }
}

fn schema_ref(action: &str, direction: &str, schema: &Value) -> CanonicalSchemaRef {
    format!("schema://nomifun/{action}/{direction}@1#{}",digest_payload(schema).expect("static schema").as_ref()).into()
}

fn input_schema(action: &str) -> Value {
    match action {
        "app.hosting/guide" | "app.hosting/current" | "app.hosting/list" => json!({"type":"object","properties":{},"additionalProperties":false}),
        "app.hosting/import" => json!({"type":"object","additionalProperties":false,"required":["path"],"properties":{
            "app_id":{"type":"string","description":"Target application ID returned by current_app in the application workspace. Omit only when importing a new application from an ordinary project workspace.","minLength":36,"maxLength":36},
            "path":{"type":"string","description":"Absolute path of the existing project folder or ZIP. Relative paths are not resolved against the Agent workspace.","minLength":1,"maxLength":4096},"name":{"type":"string","minLength":1,"maxLength":128},
            "import_kind":{"type":"string","enum":["source","artifact"]},
            "settings":settings_schema()}}),
        "app.hosting/update" => json!({"type":"object","additionalProperties":false,"required":["app_id"],"properties":{
            "app_id":{"type":"string","minLength":36,"maxLength":36},
            "detect_settings":{"type":"boolean","description":"Recognize the real supported project files and fill missing ordinary runtime settings. This does not start the application. Inspect the returned settings, configure custom commands or URLs explicitly, then call preview_app or rebuild_app."},
            "name":{"type":"string","minLength":1,"maxLength":128},
            "description":{"type":"string","maxLength":4000},
            "tags":{"type":"array","maxItems":20,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":64}},
            "import_kind":{"type":"string","enum":["source","artifact"]},
            "settings":settings_schema()}}),
        _ => json!({"type":"object","additionalProperties":false,"required":["app_id"],"properties":{"app_id":{"type":"string","minLength":36,"maxLength":36}}}),
    }
}

fn settings_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "project_directory":{"type":"string","minLength":1,"maxLength":4096},
        "install_command":{"type":["string","null"],"maxLength":65536},
        "preview_command":{"type":["string","null"],"maxLength":65536},
        "build_command":{"type":["string","null"],"maxLength":65536},
        "output_directory":{"type":["string","null"],"maxLength":4096},
        "start_command":{"type":["string","null"],"maxLength":65536},
        "web_url":{"type":["string","null"],"maxLength":4096},
        "node_path":{"type":["string","null"],"maxLength":4096},
        "environment":{"type":"object","maxProperties":128,"additionalProperties":{"type":"string","maxLength":65536}},
        "mcp_url":{"type":["string","null"],"maxLength":4096}
    }})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_app_guide_is_read_only_and_not_repeated_in_tool_descriptions() {
        assert_eq!(action_effect("app.hosting/guide"), EffectClass::ReadLocal);
        assert!(jsonschema::validator_for(&input_schema("app.hosting/guide")).unwrap().is_valid(&json!({})));
        assert!(DESCRIPTION.contains("read_app_guide"));
        assert!(!DESCRIPTION.contains("NOMIFUN_LOCAL_TRUST"));
        assert!(!DESCRIPTION.contains("/api/remote/open"));
        assert!(DEVELOPMENT_GUIDE.contains("NOMIFUN_LOCAL_TRUST"));
        assert!(DEVELOPMENT_GUIDE.contains("/api/remote/open"));
        for action in ["app.hosting/guide", "app.hosting/current", "app.hosting/list"] {
            let schema = input_schema(action);
            assert!(schema["properties"].as_object().unwrap().is_empty(),
                "zero-argument actions still need an explicit canonical object schema");
            assert_eq!(schema["additionalProperties"],false);
        }
    }

    #[test]
    fn application_workspace_actions_are_exact_and_cannot_author_connection_or_history_refs() {
        assert_eq!(action_effect("app.hosting/current"), EffectClass::ReadSensitive);
        for action in ["app.hosting/update", "app.hosting/preview", "app.hosting/rebuild"] {
            assert_eq!(action_effect(action), EffectClass::WriteDurable);
            assert!(jsonschema::validator_for(&input_schema(action)).unwrap().is_valid(&json!({"app_id":uuid::Uuid::now_v7().to_string()})));
        }
        let schema = jsonschema::validator_for(&input_schema("app.hosting/update")).unwrap();
        let valid = json!({"app_id":uuid::Uuid::now_v7().to_string(),"name":"Todo","description":"A reminder application","tags":["Productivity"],
            "import_kind":"source","detect_settings":true,"settings":{"preview_command":"npm run dev","build_command":null,"start_command":"node server.cjs"}});
        assert!(schema.is_valid(&valid));
        for invalid_detection in [json!("true"),json!({"automatic":true}),json!(null)] {
            let mut invalid = valid.clone(); invalid["detect_settings"] = invalid_detection;
            assert!(!schema.is_valid(&invalid), "configuration detection is an explicit typed command flag");
        }
        for (field,value) in [("agent_id",json!("foreign-agent")),("agent_binding_id",json!("foreign-binding")),
            ("development_session_id",json!("foreign-session")),("exclusive_session_ids",json!(["foreign-session"]))] {
            let mut invalid = valid.clone(); invalid[field] = value.clone();
            assert!(!schema.is_valid(&invalid),"configuration tool cannot author {field}");
            let mut invalid = valid.clone(); invalid["settings"][field] = value;
            assert!(!schema.is_valid(&invalid),"configuration settings cannot author {field}");
        }
    }

    #[tokio::test]
    async fn current_application_matches_only_the_actual_managed_workspace() {
        let root = tempfile::tempdir().unwrap();
        let hosting = HostingOwner::new(root.path().join("apps"), nomifun_app_hosting::LocalConnection {
            host_url:"http://127.0.0.1:1".into(), local_trust:"isolated-test".into(),
        }).await.unwrap();
        let app = hosting.create("Current application".into(), nomifun_app_hosting::AppMetadata::default()).await.unwrap();
        let found = application_for_workspace(&hosting,&app.directory.join(".")).await.map_err(|error| error.reason).unwrap().unwrap();
        assert_eq!(found.id,app.id);
        assert!(application_for_workspace(&hosting,app.directory.parent().unwrap()).await.map_err(|error| error.reason).unwrap().is_none());
        let nested = app.directory.join("nested"); tokio::fs::create_dir(&nested).await.unwrap();
        assert!(application_for_workspace(&hosting,&nested).await.map_err(|error| error.reason).unwrap().is_none(),
            "a directory prefix must not claim that an unrelated workspace belongs to an application");
        hosting.shutdown().await.unwrap();
    }

    #[test]
    fn import_commands_can_override_detection_but_cannot_claim_connection_or_session_ownership() {
        let schema = jsonschema::validator_for(&input_schema("app.hosting/import")).unwrap();
        let request = json!({"path":"project","settings":{"install_command":null,"start_command":"python server.py","environment":{"PORT":"4400"}}});
        assert!(schema.is_valid(&request));
        let overrides: ImportSettings = serde_json::from_value(request["settings"].clone()).unwrap();
        let mut settings = nomifun_app_hosting::AppSettings { install_command:Some("npm install".into()),
            build_command:Some("npm run build".into()), ..Default::default() };
        overrides.into_patch().apply(&mut settings);
        assert_eq!(settings.install_command,None,"explicit null skips the detected install command");
        assert_eq!(settings.build_command.as_deref(),Some("npm run build"),"omitted settings preserve project detection");
        assert_eq!(settings.start_command.as_deref(),Some("python server.py"));
        for (field,value) in [("agent_binding_id",json!("foreign-binding")),("exclusive_session_ids",json!(["shared-session"])),("agent_id",json!("target-agent"))] {
            let mut invalid = request.clone(); invalid["settings"][field] = value;
            assert!(!schema.is_valid(&invalid),"management import cannot author {field}");
            assert!(serde_json::from_value::<ImportSettings>(invalid["settings"].clone()).is_err());
        }
    }
}
