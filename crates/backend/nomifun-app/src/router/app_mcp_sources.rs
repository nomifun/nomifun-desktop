//! Read-only projection of hosted applications into the existing MCP owner.
//! App configuration owns the address; discovery is an in-memory observation.
//! Neither discovery nor MCP invocation starts or stops an application.
use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{extract::{Path, State}, routing::{get, post}, Json, Router};
use nomifun_api_types::{ApiResponse, McpConnectionTestErrorCode, McpConnectionTestResult, McpToolResponse};
use nomifun_common::{AppError, TimestampMs, now_ms};
use nomifun_db::{CreateMcpServerParams, DbError, IMcpServerRepository, UpdateMcpServerParams, models::McpServerRow};
use nomifun_app_hosting::{AppRecord, AppState, HostingOwner};
use serde::Serialize;
use serde_json::json;

#[derive(Clone)]
struct Observation {
    url: String,
    pid: Option<u32>,
    revision: TimestampMs,
    probe_succeeded: bool,
    probe_error: Option<String>,
    // One complete validated catalog, independent of the latest availability
    // probe. A failed readiness check is not a new tool contract.
    catalog: Option<Vec<McpToolResponse>>,
    last_connected: Option<TimestampMs>,
}

impl Observation {
    fn matches(&self, app: &AppRecord) -> bool {
        app.state == AppState::Running && app.pid == self.pid
            && app.settings.mcp_url.as_deref() == Some(self.url.as_str())
    }

    fn from_probe(app: &AppRecord, mut result: McpConnectionTestResult, previous: Option<Self>, revision: TimestampMs) -> Self {
        let previous = previous.filter(|observation| observation.matches(app));
        let preserve_catalog = !result.success && !result.needs_auth.unwrap_or(false)
            && matches!(result.code, Some(McpConnectionTestErrorCode::Timeout | McpConnectionTestErrorCode::ConnectionFailed));
        let (catalog, last_connected) = if result.success {
            (result.tools.take(), Some(revision))
        } else if preserve_catalog {
            previous.map_or((None, None), |observation| (observation.catalog, observation.last_connected))
        } else {
            (None, None)
        };
        let mut observation = Self { url: app.settings.mcp_url.clone().unwrap_or_default(), pid: app.pid,
            revision, probe_succeeded: result.success, probe_error: result.error, catalog, last_connected };
        if observation.probe_succeeded {
            // Validate before publication. Empty, malformed, or oversized new
            // catalogs explicitly replace the old contract with a rejection.
            let valid = AppMcpSources::row_with_observation(app, Some(&observation)).ok().flatten()
                .and_then(|row| super::nomi_core_mcp_catalog::server_tools(row).ok())
                .is_some_and(|tools| !tools.is_empty());
            if !valid {
                let count = observation.catalog.as_ref().map_or(0, Vec::len);
                observation.probe_succeeded = false;
                observation.catalog = None;
                observation.last_connected = None;
                observation.probe_error = Some(format!("Application MCP source {} returned {count} tools; its catalog is malformed, empty, or exceeds the current 256-tool source limit. Correct its schemas or expose fewer tools", app.id));
            }
        }
        observation
    }
}

pub(crate) struct AppMcpSources {
    hosting: Arc<HostingOwner>,
    ordinary: Arc<dyn IMcpServerRepository>,
    observations: std::sync::Mutex<BTreeMap<String, Observation>>,
    probes: std::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    publisher: std::sync::OnceLock<Arc<dyn nomifun_mcp::service::McpCatalogPublisher>>,
}

#[derive(Serialize)]
pub(crate) struct AppMcpSourceDto {
    pub app_id: String,
    pub name: String,
    pub url: String,
    pub state: AppState,
    pub connection_status: &'static str,
    pub tools: Vec<McpToolResponse>,
    pub reason: Option<String>,
}

impl AppMcpSources {
    pub(crate) fn new(hosting: Arc<HostingOwner>, ordinary: Arc<dyn IMcpServerRepository>) -> Arc<Self> {
        Arc::new(Self { hosting, ordinary, observations: Default::default(), probes: Default::default(), publisher: Default::default() })
    }

    pub(crate) fn set_publisher(&self, publisher: Arc<dyn nomifun_mcp::service::McpCatalogPublisher>) -> Result<(), AppError> {
        self.publisher.set(publisher).map_err(|_| AppError::Conflict("App MCP catalog publisher is already installed".into()))
    }

    pub(crate) async fn publish(&self) -> Result<(), AppError> {
        if let Some(publisher) = self.publisher.get() { publisher.refresh().await.map_err(AppError::from)?; }
        Ok(())
    }

    pub(crate) async fn is_app_source(&self, id: &str) -> bool {
        self.hosting.get(id).await.is_ok_and(|app| app.settings.mcp_url.is_some())
    }

    fn observation(&self, app: &AppRecord) -> Option<Observation> {
        if app.state != AppState::Running { return None; }
        self.observations.lock().ok()?.get(&app.id)
            .filter(|observation| observation.matches(app)).cloned()
    }

    pub(crate) async fn refresh(&self, id: &str) -> Result<AppMcpSourceDto, AppError> {
        let probe = self.probes.lock().map_err(|_| AppError::Conflict("App MCP source lock is unavailable".into()))?
            .entry(id.to_owned()).or_default().clone();
        let _guard = probe.lock().await;
        let app = self.hosting.get(id).await.map_err(|error| AppError::NotFound(error.to_string()))?;
        let url = app.settings.mcp_url.clone().ok_or_else(|| AppError::BadRequest("This application has no MCP address".into()))?;
        if app.state != AppState::Running {
            self.remove_observation(id)?;
            return Ok(self.dto(&app, &url));
        }
        let result = nomifun_mcp::McpConnectionTestService::new_dynamic()
            .with_setup_policy(nomifun_mcp::McpSetupPolicy::uniform(Duration::from_secs(3)))
            .test_connection(&app.name, &nomifun_mcp::McpServerTransport::Http { url: url.clone(), headers: Default::default() }).await;
        // A late probe must never publish a deleted, stopped, or edited source.
        let current = self.hosting.get(id).await.map_err(|error| AppError::NotFound(error.to_string()))?;
        if current.state != AppState::Running || current.pid != app.pid || current.settings.mcp_url.as_ref() != Some(&url) {
            self.remove_observation(id)?;
            return Err(AppError::Conflict("Application MCP source changed during discovery; refresh it again".into()));
        }
        let mut observations = self.observations.lock().map_err(|_| AppError::Conflict("App MCP observations are unavailable".into()))?;
        let previous = observations.remove(id);
        let revision = previous.as_ref().map_or(now_ms(), |previous| now_ms().max(previous.revision.saturating_add(1)));
        observations.insert(id.to_owned(), Observation::from_probe(&current, result, previous, revision));
        drop(observations);
        Ok(self.dto(&current, &url))
    }

    pub(crate) fn remove_observation(&self, id: &str) -> Result<(), AppError> {
        self.observations.lock().map_err(|_| AppError::Conflict("App MCP observations are unavailable".into()))?.remove(id);
        Ok(())
    }

    pub(crate) async fn sources(&self) -> Result<Vec<AppMcpSourceDto>, AppError> {
        let apps = self.hosting.list().await.map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(apps.iter().filter_map(|app| app.settings.mcp_url.as_ref().map(|url| self.dto(app, url))).collect())
    }

    fn dto(&self, app: &AppRecord, url: &str) -> AppMcpSourceDto {
        let observation = self.observation(app);
        Self::dto_with_observation(app, url, observation.as_ref())
    }

    fn dto_with_observation(app: &AppRecord, url: &str, observation: Option<&Observation>) -> AppMcpSourceDto {
        let observation = observation.filter(|observation| observation.matches(app));
        let status = if app.state != AppState::Running { "stopped" }
            else if observation.is_some_and(|value| value.probe_succeeded) { "connected" }
            else if observation.is_some() { "error" } else { "untested" };
        AppMcpSourceDto { app_id: app.id.clone(), name: app.name.clone(), url: url.into(), state: app.state,
            connection_status: status, tools: observation.and_then(|value| value.catalog.clone()).unwrap_or_default(),
            reason: observation.and_then(|value| value.probe_error.clone()) }
    }

    fn row(&self, app: &AppRecord) -> Result<Option<McpServerRow>, DbError> {
        let observed = self.observation(app);
        Self::row_with_observation(app, observed.as_ref())
    }

    fn row_with_observation(app: &AppRecord, observed: Option<&Observation>) -> Result<Option<McpServerRow>, DbError> {
        let Some(url) = &app.settings.mcp_url else { return Ok(None); };
        let observed = observed.filter(|observation| observation.matches(app));
        let connected = observed.is_some_and(|value| value.probe_succeeded);
        let tools = observed.and_then(|value| value.catalog.as_ref())
            .map(serde_json::to_string).transpose().map_err(|error| DbError::Conflict(error.to_string()))?;
        let revision = observed.map_or(0, |value| value.revision);
        Ok(Some(McpServerRow { mcp_server_id: app.id.clone(), name: format!("App: {}", app.name),
            description: Some("Application-owned MCP source; configure its address in application settings".into()),
            enabled: tools.is_some(),
            transport_type: "http".into(), transport_config: json!({"url":url}).to_string(), tools,
            last_test_status: if connected { "connected" } else if observed.is_some() { "error" } else { "disconnected" }.into(),
            last_connected: observed.and_then(|value| value.last_connected), original_json: None, builtin: true,
            deleted_at: None, created_at: 0, updated_at: revision }))
    }

    async fn editable(&self, id: &str) -> Result<(), DbError> {
        if self.is_app_source(id).await { return Err(DbError::Conflict("Application MCP sources are owned by application settings".into())); }
        Ok(())
    }
}

#[async_trait]
impl IMcpServerRepository for AppMcpSources {
    async fn list(&self) -> Result<Vec<McpServerRow>, DbError> {
        let mut rows = self.ordinary.list().await?;
        let apps = self.hosting.list().await.map_err(|error| DbError::Conflict(error.to_string()))?;
        for app in apps {
            if let Some(row) = self.row(&app)? {
                if rows.iter().any(|existing| existing.mcp_server_id == row.mcp_server_id) {
                    return Err(DbError::Conflict("Application and MCP configuration identities conflict".into()));
                }
                rows.push(row);
            }
        }
        Ok(rows)
    }
    async fn find_by_id(&self, id: &str) -> Result<Option<McpServerRow>, DbError> {
        if let Ok(app) = self.hosting.get(id).await { return self.row(&app); }
        self.ordinary.find_by_id(id).await
    }
    async fn find_by_name(&self, name: &str) -> Result<Option<McpServerRow>, DbError> {
        Ok(self.list().await?.into_iter().find(|row| row.name == name))
    }
    async fn find_by_id_any(&self, id: &str) -> Result<Option<McpServerRow>, DbError> {
        if let Ok(app) = self.hosting.get(id).await { return self.row(&app); }
        self.ordinary.find_by_id_any(id).await
    }
    async fn find_by_name_any(&self, name: &str) -> Result<Option<McpServerRow>, DbError> {
        if let Some(row) = self.ordinary.find_by_name_any(name).await? { return Ok(Some(row)); }
        let apps = self.hosting.list().await.map_err(|error| DbError::Conflict(error.to_string()))?;
        for app in apps {
            if format!("App: {}",app.name) == name { return self.row(&app); }
        }
        Ok(None)
    }
    async fn create(&self, params: CreateMcpServerParams<'_>) -> Result<McpServerRow, DbError> { self.ordinary.create(params).await }
    async fn update(&self, id: &str, params: UpdateMcpServerParams<'_>) -> Result<McpServerRow, DbError> { self.editable(id).await?; self.ordinary.update(id, params).await }
    async fn delete(&self, id: &str) -> Result<(), DbError> { self.editable(id).await?; self.ordinary.delete(id).await }
    async fn batch_upsert(&self, servers: &[CreateMcpServerParams<'_>]) -> Result<Vec<McpServerRow>, DbError> { self.ordinary.batch_upsert(servers).await }
    async fn update_status(&self, id: &str, status: &str, connected: Option<TimestampMs>) -> Result<(), DbError> { self.editable(id).await?; self.ordinary.update_status(id, status, connected).await }
    async fn update_tools(&self, id: &str, tools: Option<&str>) -> Result<(), DbError> { self.editable(id).await?; self.ordinary.update_tools(id, tools).await }
    async fn update_probe_if_revision(&self, id: &str, revision: TimestampMs, status: &str, connected: Option<TimestampMs>, tools: Option<&str>) -> Result<bool, DbError> { self.editable(id).await?; self.ordinary.update_probe_if_revision(id, revision, status, connected, tools).await }
}

#[derive(Clone)]
pub(crate) struct AppMcpRouterState {
    pub sources: Arc<AppMcpSources>,
    pub publisher: Arc<dyn nomifun_mcp::service::McpCatalogPublisher>,
}

pub(crate) fn routes(state: AppMcpRouterState) -> Router {
    Router::new().route("/api/app-hosting/mcp-sources", get(list_sources))
        .route("/api/app-hosting/mcp-sources/{id}/refresh", post(refresh_source)).with_state(state)
}

async fn list_sources(State(state): State<AppMcpRouterState>) -> Result<Json<ApiResponse<Vec<AppMcpSourceDto>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.sources.sources().await?)))
}

async fn refresh_source(State(state): State<AppMcpRouterState>, Path(id): Path<String>) -> Result<Json<ApiResponse<AppMcpSourceDto>>, AppError> {
    let result = state.sources.refresh(&id).await?;
    state.publisher.refresh().await.map_err(AppError::from)?;
    Ok(Json(ApiResponse::ok(result)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_control_plane::SessionCapabilitiesResolver;
    use nomifun_agent_contracts::UserId;
    use nomifun_app_hosting::{AppOperation, ImportKind, LocalConnection};

    fn hosted_record() -> AppRecord {
        AppRecord { id: "0190f5fe-7c00-7a00-8000-000000000001".into(), name: "Hosted tool".into(),
            description: String::new(), tags: Vec::new(), development_session_id: None,
            directory: "/fixture/hosted-tool".into(), import_kind: ImportKind::Source,
            settings: nomifun_app_hosting::AppSettings { mcp_url: Some("http://127.0.0.1:43111/mcp".into()), ..Default::default() },
            state: AppState::Running, operation: None, last_error: None, output: String::new(),
            pid: Some(71), preview: false, prepared: true }
    }

    fn tool(name: &str) -> McpToolResponse {
        McpToolResponse { name: name.into(), description: Some("Fixture tool".into()),
            input_schema: Some(json!({"type":"object","properties":{"value":{"type":"number"}}})) }
    }

    fn successful_probe(tools: Vec<McpToolResponse>) -> McpConnectionTestResult {
        McpConnectionTestResult { success: true, tools: Some(tools), error: None, code: None,
            details: None, needs_auth: None, auth_method: None, www_authenticate: None }
    }

    fn failed_probe(code: McpConnectionTestErrorCode) -> McpConnectionTestResult {
        McpConnectionTestResult { success: false, tools: None, error: Some("Fixture readiness failed".into()), code: Some(code),
            details: None, needs_auth: None, auth_method: None, www_authenticate: None }
    }

    fn observed_row(app: &AppRecord, observation: &Observation) -> McpServerRow {
        AppMcpSources::row_with_observation(app, Some(observation)).unwrap().unwrap()
    }

    #[test]
    fn hosted_transient_probe_failure_keeps_contract_without_claiming_online_status() {
        let app = hosted_record();
        for code in [McpConnectionTestErrorCode::Timeout, McpConnectionTestErrorCode::ConnectionFailed] {
            let good = Observation::from_probe(&app, successful_probe(vec![tool("echo")]), None, 10);
            let original = observed_row(&app, &good);
            let frozen = super::super::nomi_core_mcp_catalog::server_tools(original.clone()).unwrap().remove(0);
            let failed = Observation::from_probe(&app, failed_probe(code), Some(good), 11);
            let row = observed_row(&app, &failed);
            assert!(row.enabled, "a failed readiness probe must not withdraw the validated contract");
            assert_eq!(row.tools, original.tools);
            assert_eq!(row.last_connected, Some(10));
            assert_eq!(row.last_test_status, "error");
            let still_frozen = super::super::nomi_core_mcp_catalog::server_tools(row).unwrap().remove(0);
            assert_eq!(still_frozen.lock, frozen.lock);
            assert_eq!(still_frozen.connection_config_ref, frozen.connection_config_ref);
            let dto = AppMcpSources::dto_with_observation(&app, app.settings.mcp_url.as_deref().unwrap(), Some(&failed));
            assert_eq!(dto.connection_status, "error");
            assert_eq!(dto.reason.as_deref(), Some("Fixture readiness failed"));
            assert_eq!(dto.tools.len(), 1);
            let recovered = Observation::from_probe(&app, successful_probe(vec![tool("replacement")]), Some(failed), 12);
            let row = observed_row(&app, &recovered);
            assert!(row.enabled);
            assert_eq!(row.last_test_status, "connected");
            assert_eq!(recovered.catalog.unwrap()[0].name, "replacement");
        }
    }

    #[test]
    fn hosted_invalid_new_catalog_is_rejected_instead_of_resurrecting_last_good_tools() {
        let app = hosted_record();
        let mut malformed = tool("malformed");
        malformed.input_schema = Some(json!("not an object schema"));
        for tools in [vec![], vec![malformed], (0..257).map(|index| tool(&format!("tool{index}"))).collect()] {
            let good = Observation::from_probe(&app, successful_probe(vec![tool("echo")]), None, 10);
            let rejected = Observation::from_probe(&app, successful_probe(tools), Some(good), 11);
            assert!(!rejected.probe_succeeded);
            assert!(rejected.probe_error.as_deref().unwrap().contains("catalog is malformed, empty, or exceeds"));
            let row = observed_row(&app, &rejected);
            assert!(!row.enabled);
            assert!(row.tools.is_none());
            let later_timeout = Observation::from_probe(&app, failed_probe(McpConnectionTestErrorCode::Timeout), Some(rejected), 12);
            assert!(!observed_row(&app, &later_timeout).enabled);
            assert!(later_timeout.catalog.is_none());
        }
    }

    #[test]
    fn hosted_catalog_cannot_cross_stop_process_or_endpoint_changes() {
        let app = hosted_record();
        for change in ["stopped", "pid", "url"] {
            let good = Observation::from_probe(&app, successful_probe(vec![tool("echo")]), None, 10);
            let mut changed = app.clone();
            match change {
                "stopped" => changed.state = AppState::Stopped,
                "pid" => changed.pid = Some(72),
                _ => changed.settings.mcp_url = Some("http://127.0.0.1:43112/mcp".into()),
            }
            let row = observed_row(&changed, &good);
            assert!(!row.enabled, "{change} must invalidate the previous contract");
            assert!(row.tools.is_none());
            let failed = Observation::from_probe(&changed, failed_probe(McpConnectionTestErrorCode::Timeout), Some(good), 11);
            assert!(!observed_row(&changed, &failed).enabled);
            assert!(failed.catalog.is_none());
        }
        let good = Observation::from_probe(&app, successful_probe(vec![tool("echo")]), None, 10);
        let mut authentication = failed_probe(McpConnectionTestErrorCode::ConnectionFailed);
        authentication.needs_auth = Some(true);
        let rejected = Observation::from_probe(&app, authentication, Some(good), 11);
        assert!(!observed_row(&app, &rejected).enabled, "authentication rejection is not a transient readiness failure");
    }

    #[tokio::test]
    async fn real_hosted_mcp_is_discovered_selected_and_called_without_another_process_owner() {
        assert!(std::process::Command::new("node").arg("--version").output().is_ok_and(|output| output.status.success()), "this integration test requires the user's system Node");
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        std::fs::create_dir_all(&original).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        std::fs::write(original.join("mcp.cjs"), r#"
const http = require('node:http');
http.createServer((req,res)=>{
  if (req.method==='DELETE') { res.writeHead(204); res.end(); return; }
  let body=''; req.on('data',part=>body+=part); req.on('end',()=>{
    const request=JSON.parse(body || '{}');
    if (request.id===undefined) { res.writeHead(202); res.end(); return; }
    let result;
    if (request.method==='initialize') result={protocolVersion:request.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'hosted-app',version:'1'}};
    else if (request.method==='tools/list') result={tools:[{name:'double',description:'Double a supplied number',inputSchema:{type:'object',required:['value'],additionalProperties:false,properties:{value:{type:'number'}}}}]};
    else if (request.method==='tools/call') result={content:[{type:'text',text:String(request.params.arguments.value*2)}]};
    else { res.writeHead(400); res.end(); return; }
    res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({jsonrpc:'2.0',id:request.id,result}));
  });
}).listen(Number(process.env.PORT),'127.0.0.1');
"#).unwrap();
        let hosting = HostingOwner::new(root.path().join("managed"), LocalConnection {
            host_url: "http://127.0.0.1:1".into(), local_trust: "test-local-owner".into(),
        }).await.unwrap();
        let app = hosting.import(&original, Some("Number tool".into()), ImportKind::Source).await.unwrap();
        let mut settings = app.settings;
        settings.install_command = None; settings.build_command = None; settings.preview_command = None;
        settings.start_command = Some("node mcp.cjs".into());
        settings.environment.insert("PORT".into(), port.to_string());
        settings.mcp_url = Some(format!("http://127.0.0.1:{port}/mcp"));
        hosting.update(&app.id, app.name, settings).await.unwrap();
        let database = nomifun_db::init_database_memory().await.unwrap();
        let ordinary = Arc::new(nomifun_db::SqliteMcpServerRepository::new(database.pool().clone()));
        let sources = AppMcpSources::new(hosting.clone(), ordinary.clone());
        assert_eq!(sources.sources().await.unwrap()[0].connection_status, "stopped");
        assert!(!sources.find_by_id(&app.id).await.unwrap().unwrap().enabled);
        hosting.operate(&app.id, AppOperation::Use).await.unwrap();
        let pid = hosting.get(&app.id).await.unwrap().pid;
        let mut source = sources.refresh(&app.id).await.unwrap();
        for _ in 0..10 {
            if source.connection_status == "connected" { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
            source = sources.refresh(&app.id).await.unwrap();
        }
        assert_eq!(source.connection_status, "connected", "{:?}", source.reason);
        assert_eq!(source.tools.len(), 1);
        assert_eq!(hosting.get(&app.id).await.unwrap().pid, pid, "MCP discovery must never launch a second application");
        assert!(ordinary.list().await.unwrap().is_empty(), "an App address/catalog must not be copied into a writable MCP table");
        assert!(sources.update(&app.id, UpdateMcpServerParams::default()).await.is_err());

        let paths = nomifun_skill_library::SkillPaths {
            data_dir: root.path().join("skills-data"), user_skills_dir: root.path().join("user-skills"),
            cron_skills_dir: root.path().join("cron-skills"), builtin_skills_dir: root.path().join("builtin-skills"),
            builtin_rules_dir: root.path().join("builtin-rules"),
        };
        let owner: UserId = "owner".into();
        let resolver = super::super::session_capabilities::GlobalSessionCapabilities {
            owner: owner.clone(), skills: Arc::new(paths), mcp: sources.clone(), app_mcp: sources.clone(),
        };
        assert!(resolver.resolve(&owner, None).await.unwrap().mcp_server_ids.is_empty(), "a new application must not enter every ordinary Agent Session");
        let selected = nomifun_api_types::SessionCapabilitySelectionDto { skill_names: vec![], mcp_server_ids: vec![app.id.clone()] };
        assert_eq!(resolver.resolve(&owner, Some(&selected)).await.unwrap().mcp_server_ids, std::collections::BTreeSet::from([app.id.clone()]));
        let row = sources.find_by_id(&app.id).await.unwrap().unwrap();
        let descriptor = super::super::nomi_core_mcp_catalog::server_tools(row.clone()).unwrap().remove(0);
        let transport = nomifun_mcp::McpServerTransport::from_db(&row.transport_type, &row.transport_config).unwrap();
        let connection = descriptor.connection_config_ref.as_ref().to_owned();
        let result = nomifun_mcp::McpOwner::new_dynamic(Arc::new(nomifun_mcp::AnonymousMcpCredentialAuthority))
            .invoke(nomifun_mcp::McpToolInvocationRequest {
                principal_kind: "user".into(), principal_id: "owner".into(), operation_id: "app-real-call".into(),
                server: nomifun_mcp::McpServerBinding { server_id: app.id.clone(), server_owner_id: "owner".into(), enabled: true,
                    connection_config_ref: connection.clone(), resource_binding_id: "app-binding".into(), resource_kind: "mcp_server".into(),
                    resource_id: app.id.clone(), resource_owner_id: "owner".into(), granted_operations: std::collections::BTreeSet::from(["connect".into(),"invoke".into()]),
                    resource_connection_config_ref: Some(connection), transport },
                tool: nomifun_mcp::McpToolBinding::new(app.id.clone(),descriptor.lock.canonical_tool_key.as_ref(),descriptor.lock.schema_digest.as_ref(),descriptor.input_schema,"double").unwrap(),
                arguments: json!({"value":21}),
            }).await.unwrap();
        assert_eq!(result.result["content"][0]["text"], "42", "must receive the actual hosted service's MCP response");
        assert_eq!(hosting.get(&app.id).await.unwrap().pid, pid);
        hosting.operate(&app.id, AppOperation::Stop).await.unwrap();
        assert_eq!(sources.sources().await.unwrap()[0].connection_status, "stopped");
        assert!(resolver.resolve(&owner, Some(&selected)).await.is_err(), "a selected stopped source is unavailable without auto-start");
        hosting.delete(&app.id).await.unwrap();
        assert!(sources.find_by_id(&app.id).await.unwrap().is_none());
        assert!(original.join("mcp.cjs").exists());
        hosting.shutdown().await.unwrap();
    }
}
