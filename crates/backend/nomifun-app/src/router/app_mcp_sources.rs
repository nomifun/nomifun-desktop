//! Read-only projection of hosted applications into the existing MCP owner.
//! App configuration owns the address; discovery is an in-memory observation.
//! Neither discovery nor MCP invocation starts or stops an application.
use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{extract::{Path, State}, routing::{get, post}, Json, Router};
use nomifun_api_types::{ApiResponse, McpConnectionTestResult, McpToolResponse};
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
    result: McpConnectionTestResult,
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
            .filter(|observation| app.pid == observation.pid && app.settings.mcp_url.as_deref() == Some(observation.url.as_str())).cloned()
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
        let result = nomifun_mcp::McpConnectionTestService::new_dynamic().with_timeout(Duration::from_secs(3))
            .test_connection(&app.name, &nomifun_mcp::McpServerTransport::Http { url: url.clone(), headers: Default::default() }).await;
        // A late probe must never publish a deleted, stopped, or edited source.
        let current = self.hosting.get(id).await.map_err(|error| AppError::NotFound(error.to_string()))?;
        if current.state != AppState::Running || current.pid != app.pid || current.settings.mcp_url.as_ref() != Some(&url) {
            self.remove_observation(id)?;
            return Err(AppError::Conflict("Application MCP source changed during discovery; refresh it again".into()));
        }
        let mut observations = self.observations.lock().map_err(|_| AppError::Conflict("App MCP observations are unavailable".into()))?;
        let revision = observations.get(id).map_or(now_ms(), |previous| now_ms().max(previous.revision.saturating_add(1)));
        observations.insert(id.to_owned(), Observation { url: url.clone(), pid: current.pid, revision, result });
        drop(observations);
        // The existing Agent contract limits remain real. Reject this source
        // atomically with a local diagnostic instead of truncating its tools.
        if self.observation(&current).is_some_and(|value| value.result.success) {
            let valid = self.row(&current).ok().flatten()
                .and_then(|row| super::nomi_core_mcp_catalog::server_tools(row).ok())
                .is_some_and(|tools| !tools.is_empty());
            if !valid {
                let mut observations = self.observations.lock().map_err(|_| AppError::Conflict("App MCP observations are unavailable".into()))?;
                if let Some(observation) = observations.get_mut(id) {
                    let count = observation.result.tools.as_ref().map_or(0,Vec::len);
                    observation.result.success = false;
                    observation.result.tools = None;
                    observation.result.error = Some(format!("Application MCP source {id} returned {count} tools; its catalog is malformed, empty, or exceeds the current 256-tool source limit. Correct its schemas or expose fewer tools"));
                }
            }
        }
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
        let status = if app.state != AppState::Running { "stopped" }
            else if observation.as_ref().is_some_and(|value| value.result.success) { "connected" }
            else if observation.is_some() { "error" } else { "untested" };
        AppMcpSourceDto { app_id: app.id.clone(), name: app.name.clone(), url: url.into(), state: app.state,
            connection_status: status, tools: observation.as_ref().and_then(|value| value.result.tools.clone()).unwrap_or_default(),
            reason: observation.and_then(|value| value.result.error) }
    }

    fn row(&self, app: &AppRecord) -> Result<Option<McpServerRow>, DbError> {
        let Some(url) = &app.settings.mcp_url else { return Ok(None); };
        let observed = self.observation(app);
        let connected = observed.as_ref().is_some_and(|value| value.result.success);
        let tools = observed.as_ref().and_then(|value| value.result.tools.as_ref())
            .map(serde_json::to_string).transpose().map_err(|error| DbError::Conflict(error.to_string()))?;
        let revision = observed.as_ref().map_or(0, |value| value.revision);
        Ok(Some(McpServerRow { mcp_server_id: app.id.clone(), name: format!("App: {}", app.name),
            description: Some("Application-owned MCP source; configure its address in application settings".into()),
            enabled: app.state == AppState::Running && connected,
            transport_type: "http".into(), transport_config: json!({"url":url}).to_string(), tools,
            last_test_status: if connected { "connected" } else { "disconnected" }.into(),
            last_connected: connected.then_some(revision), original_json: None, builtin: true,
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
