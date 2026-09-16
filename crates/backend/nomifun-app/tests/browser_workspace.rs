#![cfg(feature = "browser-use")]
#[path = "../src/browser_workspace_provider.rs"]
mod native_resolver;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use nomifun_browser_platform::{
    run_guard::{NativeInputGate, RunAdmissionError},
    runtime::*,
    workspace::BrowserWorkspaceService,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tower::ServiceExt;

struct Factory {
    fail_close: Arc<AtomicBool>,
    created: Arc<std::sync::Mutex<Vec<CreateBrowserRuntime>>>,
}
struct Runtime {
    generation: u64,
    fail_close: Arc<AtomicBool>,
}
#[async_trait]
impl BrowserRuntimeFactory for Factory {
    async fn create(
        &self,
        request: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        self.created.lock().unwrap().push(request.clone());
        Ok(Arc::new(Runtime {
            generation: request.runtime_generation,
            fail_close: self.fail_close.clone(),
        }))
    }
}
#[async_trait]
impl NativeInputGate for Runtime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        Ok(())
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        Ok(())
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        Ok(())
    }
}
#[async_trait]
impl BrowserRuntime for Runtime {
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> {
        None
    }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        Ok(BrowserRuntimeSnapshot {
            downloads: vec![],
            runtime_generation: self.generation,
            revision: 1,
            active_tab_id: None,
            tabs: vec![],
        })
    }
    async fn execute(
        &self,
        _: BrowserTabCommand,
        _: tokio_util::sync::CancellationToken,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        self.snapshot().await
    }
    async fn close(&self) -> Result<(), WorkspaceError> {
        if self.fail_close.load(Ordering::SeqCst) {
            Err(WorkspaceError::NativeCommandFailed)
        } else {
            Ok(())
        }
    }
}

fn request(method: &str, path: &str, trusted: bool, body: serde_json::Value) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if trusted {
        request = request.header("x-nomi-local-trust", "native-browser-api-test");
    }
    request.body(Body::from(body.to_string())).unwrap()
}

fn native_provider() -> nomifun_agent_contracts::ExactRoleProviderRef {
    let registration = nomifun_agent_domain_wave2::registrations()
        .unwrap()
        .into_iter()
        .find(|registration| {
            registration.metadata.manifest.payload.package_id.as_ref()
                == nomifun_agent_domain_wave2::BROWSER_PACKAGE_ID
        })
        .unwrap();
    let manifest = &registration.metadata.manifest.payload;
    let contribution = &manifest.contributions.role_providers[0];
    nomifun_agent_contracts::ExactRoleProviderRef {
        role: contribution.role.clone(),
        package: nomifun_agent_contracts::PackageRef {
            id: manifest.package_id.clone(),
            version: manifest.package_version.clone(),
        },
        mount_id: nomifun_agent_domain_wave2::BROWSER_MOUNT_ID.into(),
        contribution_digest: nomifun_agent_contracts::digest_payload(contribution).unwrap(),
    }
}

#[tokio::test]
async fn native_browser_http_authority_and_predelete_cleanup_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let db = nomifun_db::init_database_memory().await.unwrap();
    use nomifun_db::IClientPreferenceRepository;
    let preferences=nomifun_db::SqliteClientPreferenceRepository::new(db.pool().clone());
    let retired_preferences=[
        ("agent.browserUse.displayMode","\"external\""),
        ("agent.browserUse.displayModeVersion","2"),
        ("browser.resourcePolicy","invalid retired policy"),
    ];
    preferences.upsert_batch(&retired_preferences).await.unwrap();
    let mut services = nomifun_app::compatibility::AppServices::from_config(
        db,
        &nomifun_app::AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from("native-browser-api-test")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let keys=retired_preferences.iter().map(|(key,_)|*key).collect::<Vec<_>>();
    let stored=preferences.get_by_keys(&keys).await.unwrap();
    for (key,value) in retired_preferences {
        assert_eq!(stored.iter().find(|row|row.key==key).unwrap().value,value,"startup must not migrate retired browser settings");
    }
    assert!(!services.data_dir.join("browser-v2/headless/profiles").exists(),
        "startup must not construct or recover the retired Hub profile root");
    let fail_close = Arc::new(AtomicBool::new(false));
    let workspaces = Arc::new(BrowserWorkspaceService::new(Arc::new(Factory {
        fail_close: fail_close.clone(),
        created: Default::default(),
    })));
    services.browser_workspaces = Some(workspaces.clone());
    let id = "0190f5fe-7c00-7a00-8000-000000000081";
    nomifun_db::sqlx::query("INSERT INTO conversations (conversation_id,user_id,name,type,extra,status,created_at,updated_at) VALUES (?,?,'browser fixture','nomi','{}','pending',1,1)")
        .bind(id).bind(services.authoritative_user_id.as_ref()).execute(services.database.pool()).await.unwrap();
    let router = nomifun_app::compatibility::create_router(&services).await;
    for (method,path) in [
        ("GET","/api/browser/login/status"),("POST","/api/browser/login/open"),("POST","/api/browser/login/close"),
        ("GET","/api/browser/overview"),("GET","/api/browser/lanes"),
        ("POST","/api/browser/lanes/retired/close"),("POST","/api/browser/lanes/retired/foreground"),
        ("POST","/api/browser/lanes/retired/background"),("POST","/api/browser/conversations/retired/close"),
        ("POST","/api/browser/close-all"),("GET","/api/browser/resource-policy"),("PUT","/api/browser/resource-policy"),
        ("GET","/api/browser/display-mode"),("PUT","/api/browser/display-mode"),
    ] {
        let response=router.clone().oneshot(request(method,path,true,serde_json::json!({}))).await.unwrap();
        assert_eq!(response.status(),StatusCode::NOT_FOUND,"retired browser management API must not remain callable: {path}");
    }
    let response=router.clone().oneshot(request("GET","/api/capabilities",true,serde_json::json!({}))).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    let body=axum::body::to_bytes(response.into_body(),512*1024).await.unwrap();
    let catalog:serde_json::Value=serde_json::from_slice(&body).unwrap();
    let catalog=catalog["data"].as_array().expect("capability catalog");
    let local=catalog.iter().find(|item|item["capability"]["id"]=="nomi_local_websearch").expect("local search must appear in the workbench catalog");
    assert_eq!(local["capability"]["version"],"1.0.0");
    assert_eq!(local["materialization_state"],"unavailable","native Workspace availability must not imply verified local-search runtime availability");
    assert_eq!(local["required_resource_kinds"],serde_json::json!([]));
    assert_eq!(local["conflicting_capabilities"],serde_json::json!([]));
    assert!(catalog.iter().any(|item|item["capability"]["id"]=="web.search"));
    let path = format!("/api/conversations/{id}/browser");
    let response = router
        .clone()
        .oneshot(request("POST", &path, false, serde_json::json!({})))
        .await
        .unwrap();
    assert!(matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));
    let response = router
        .clone()
        .oneshot(request("POST", &path, true, serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16_384)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["run"]["input_state"], "user_ready");
    assert_eq!(body["data"]["runtime"], serde_json::Value::Null);
    let key = BrowserWorkspaceKey {
        user_id: services.authoritative_user_id.to_string(),
        conversation_id: id.into(),
    };
    let workspace = workspaces.get(&key).await.unwrap();
    let resolve = native_resolver::resolver(
        Some(workspaces.clone()),
        services.database.pool().clone(),
        services.execution_conversation_boundary.clone(),
        services.data_dir.clone(),
        services.authoritative_user_id.clone(),
    );
    let without_native = native_resolver::resolver(
        None,
        services.database.pool().clone(),
        services.execution_conversation_boundary.clone(),
        services.data_dir.clone(),
        services.authoritative_user_id.clone(),
    );
    let provider = native_provider();
    let workspace_request =
        |user_id: String| nomifun_ai_agent::factory::BrowserRuntimeRequest {
            user_id,
            conversation_id: id.into(),
            temporary: true,
            selected: true,
            provider: Some(provider.clone()),
        };
    nomifun_db::sqlx::query("UPDATE conversations SET source='telegram' WHERE conversation_id=?")
        .bind(id)
        .execute(services.database.pool())
        .await
        .unwrap();
    assert!(resolve(workspace_request(key.user_id.clone())).await.is_err(),
        "channel Agents must not use the interactive workspace");
    assert!(without_native(workspace_request(key.user_id.clone())).await.is_err());
    nomifun_db::sqlx::query("UPDATE conversations SET source='nomifun' WHERE conversation_id=?")
        .bind(id)
        .execute(services.database.pool())
        .await
        .unwrap();
    assert!(
        Arc::ptr_eq(
            &resolve(workspace_request(key.user_id.clone()))
                .await
                .unwrap()
                .expect("expected the owned native workspace"),
            &workspace
        ),
        "Agent and UI must resolve the same workspace"
    );
    assert!(
        resolve(workspace_request("another-user".into()))
            .await
            .is_err()
    );
    assert!(without_native(workspace_request(key.user_id.clone())).await.is_err(),
        "selected Browser capability requires a native host");
    let unselected = nomifun_ai_agent::factory::BrowserRuntimeRequest {
        user_id: key.user_id.clone(),
        conversation_id: id.into(),
        temporary: true,
        selected: false,
        provider: None,
    };
    assert!(resolve(unselected).await.unwrap().is_none(),
        "ordinary chat must not create a hidden Browser workspace");
    let mut missing_provider = workspace_request(key.user_id.clone());
    missing_provider.provider = None;
    assert!(
        matches!(
            resolve(missing_provider).await,
            Err(nomifun_common::AppError::UnprocessableEntity(_))
        ),
        "a selected Browser capability requires its exact v2 Provider"
    );
    let mut selected = workspace_request(key.user_id.clone());
    let mut old_provider = provider.clone();
    old_provider.role.key.contract_version = "1.0.0".into();
    selected.provider = Some(old_provider);
    assert!(
        matches!(
            resolve(selected).await,
            Err(nomifun_common::AppError::UnprocessableEntity(_))
        ),
        "v1 Providers must not bind native workspaces"
    );
    let selected = workspace_request(key.user_id.clone());
    assert!(Arc::ptr_eq(
        &resolve(selected)
            .await
            .unwrap()
            .expect("expected the owned native workspace"),
        &workspace
    ));
    let mut changed = workspace_request(key.user_id.clone());
    let mut changed_provider = provider.clone();
    changed_provider.contribution_digest = "changed-contribution".into();
    changed.provider = Some(changed_provider);
    assert!(
        matches!(
            resolve(changed).await,
            Err(nomifun_common::AppError::Conflict(_))
        ),
        "exact contribution changes must not reuse the previous Provider's workspace"
    );
    assert!(
        Arc::ptr_eq(
            &resolve(workspace_request(key.user_id.clone()))
                .await
                .unwrap()
                .expect("expected the owned native workspace"),
            &workspace
        ),
        "user reopen must preserve the bound Provider"
    );
    let run = workspace.begin_run().await.unwrap();
    let commands = format!("{path}/commands");
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &commands,
            true,
            serde_json::json!({"command":"create","url":"http://localhost:3000"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), 16_384)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "BROWSER_USER_INPUT_LOCKED");
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &commands,
            true,
            serde_json::json!({"command":"unlock"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    workspace.finish_run(&run).await.unwrap();
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &commands,
            true,
            serde_json::json!({"command":"create","url":"http://localhost:3000"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    fail_close.store(true, Ordering::SeqCst);
    let generation=workspace.runtime_generation();
    let close_body=serde_json::json!({"runtime_generation":generation});
    let message_id=nomifun_common::generate_id();
    nomifun_db::sqlx::query("INSERT INTO messages (message_id,conversation_id,msg_id,type,content,position,status,created_at) VALUES (?,?,?,'text','{\"content\":\"keep browser conversation history\"}','right','finish',1000)")
        .bind(&message_id).bind(id).bind(&message_id).execute(services.database.pool()).await.unwrap();
    let before:(String,String,Option<String>,i64)=nomifun_db::sqlx::query_as("SELECT name,extra,status,updated_at FROM conversations WHERE conversation_id=?")
        .bind(id).fetch_one(services.database.pool()).await.unwrap();
    let denied=router.clone().oneshot(request("DELETE",&path,false,close_body.clone())).await.unwrap();
    assert!(matches!(denied.status(),StatusCode::UNAUTHORIZED|StatusCode::FORBIDDEN));
    let stale=router.clone().oneshot(request("DELETE",&path,true,serde_json::json!({"runtime_generation":generation+1}))).await.unwrap();
    assert_eq!(stale.status(),StatusCode::CONFLICT);
    let run=workspace.begin_run().await.unwrap();
    let busy=router.clone().oneshot(request("DELETE",&path,true,close_body.clone())).await.unwrap();
    assert_eq!(busy.status(),StatusCode::CONFLICT);
    assert!(workspace.agent_command(&run,BrowserTabCommand::Create {url:"http://localhost:3000".into()}).await.is_ok());
    workspace.finish_run(&run).await.unwrap();
    let failed=router.clone().oneshot(request("DELETE",&path,true,close_body.clone())).await.unwrap();
    assert_eq!(failed.status(),StatusCode::CONFLICT);
    assert!(Arc::ptr_eq(&workspace,&workspaces.get(&key).await.unwrap()));
    fail_close.store(false,Ordering::SeqCst);
    let closed=router.clone().oneshot(request("DELETE",&path,true,close_body.clone())).await.unwrap();
    assert_eq!(closed.status(),StatusCode::OK);
    assert!(workspaces.get(&key).await.is_none());
    let retried=router.clone().oneshot(request("DELETE",&path,true,close_body.clone())).await.unwrap();
    assert_eq!(retried.status(),StatusCode::OK,"already-closed retry must be idempotent");
    let after:(String,String,Option<String>,i64)=nomifun_db::sqlx::query_as("SELECT name,extra,status,updated_at FROM conversations WHERE conversation_id=?")
        .bind(id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(before,after,"browser rebuild must not reset the conversation aggregate");
    let content:String=nomifun_db::sqlx::query_scalar("SELECT content FROM messages WHERE message_id=?").bind(&message_id).fetch_one(services.database.pool()).await.unwrap();
    assert!(content.contains("keep browser conversation history"));
    let recreated=router.clone().oneshot(request("POST",&commands,true,serde_json::json!({"command":"create","url":"http://localhost:3000"}))).await.unwrap();
    assert_eq!(recreated.status(),StatusCode::OK);
    let replacement=workspaces.get(&key).await.unwrap();
    assert!(replacement.runtime_generation()>generation);
    let stale=router.clone().oneshot(request("DELETE",&path,true,close_body)).await.unwrap();
    assert_eq!(stale.status(),StatusCode::CONFLICT);
    assert!(Arc::ptr_eq(&replacement,&workspaces.get(&key).await.unwrap()));
    fail_close.store(true,Ordering::SeqCst);
    let delete_path = format!("/api/conversations/{id}");
    let response = router
        .clone()
        .oneshot(request(
            "DELETE",
            &delete_path,
            true,
            serde_json::Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let remaining: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT count(*) FROM conversations WHERE conversation_id=?",
    )
    .bind(id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(
        remaining, 1,
        "failed native cleanup must preserve the conversation"
    );
    fail_close.store(false, Ordering::SeqCst);
    let response = router
        .clone()
        .oneshot(request(
            "DELETE",
            &delete_path,
            true,
            serde_json::Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(workspaces.get(&key).await.is_none());
    let response = router
        .oneshot(request("GET", &path, true, serde_json::Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn user_and_agent_select_the_same_conversation_profile_without_project_sharing() {
    let root = tempfile::tempdir().unwrap();
    let db = nomifun_db::init_database_memory().await.unwrap();
    let mut services = nomifun_app::compatibility::AppServices::from_config(db,
        &nomifun_app::AppConfig {
            data_dir: root.path().join("data"), work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from("native-browser-api-test")),
            ..Default::default()
        }).await.unwrap();
    let created = Arc::new(std::sync::Mutex::new(Vec::new()));
    let workspaces = Arc::new(BrowserWorkspaceService::new(Arc::new(Factory {
        fail_close: Arc::new(AtomicBool::new(false)), created: created.clone(),
    })));
    services.browser_workspaces = Some(workspaces.clone());
    let resolve = native_resolver::resolver(Some(workspaces.clone()), services.database.pool().clone(),
        services.execution_conversation_boundary.clone(), services.data_dir.clone(), services.authoritative_user_id.clone());
    let router = nomifun_app::compatibility::create_router(&services).await;
    let ids = ["0190f5fe-7c00-7a00-8000-000000000091", "0190f5fe-7c00-7a00-8000-000000000092", "0190f5fe-7c00-7a00-8000-000000000093"];
    // This path deliberately does not exist: login identity must not depend on
    // filesystem canonicalization, or on two conversations choosing one project.
    let project = root.path().join("project-not-created");
    for (index, id) in ids.iter().enumerate() {
        let temporary = index == 2;
        let extra = if temporary { serde_json::json!({"workspace":project,"temp_workspace_id":id}) }
            else { serde_json::json!({"workspace":project}) };
        nomifun_db::sqlx::query("INSERT INTO conversations (conversation_id,user_id,name,type,extra,status,created_at,updated_at) VALUES (?,?,'profile fixture','nomi',?,'pending',1,1)")
            .bind(id).bind(services.authoritative_user_id.as_ref()).bind(extra.to_string())
            .execute(services.database.pool()).await.unwrap();
        let key = BrowserWorkspaceKey { user_id: services.authoritative_user_id.to_string(), conversation_id: (*id).into() };
        let path = format!("/api/conversations/{id}/browser/commands");
        // Materialize through both independent admission paths, with a close
        // between them so reuse cannot hide a disagreement in Profile selection.
        for agent_first in [false, true] {
            let workspace = if agent_first {
                let target = resolve(nomifun_ai_agent::factory::BrowserRuntimeRequest {
                    user_id: key.user_id.clone(), conversation_id: key.conversation_id.clone(), temporary,
                    selected: true, provider: Some(native_provider()),
                }).await.unwrap();
                let workspace = target.expect("native workspace");
                let run = workspace.begin_run().await.unwrap();
                workspace.agent_command(&run, BrowserTabCommand::Create {url:"http://localhost:3000".into()}).await.unwrap();
                workspace.finish_run(&run).await.unwrap();
                workspace
            } else {
                let response = router.clone().oneshot(request("POST", &path, true,
                    serde_json::json!({"command":"create","url":"http://localhost:3000"}))).await.unwrap();
                let status = response.status();
                let body = axum::body::to_bytes(response.into_body(), 16_384).await.unwrap();
                assert_eq!(status, StatusCode::OK, "conversation {id}: {}", String::from_utf8_lossy(&body));
                workspaces.get(&key).await.unwrap()
            };
            let actual = created.lock().unwrap().last().unwrap().clone();
            assert_eq!(actual.key, key);
            assert_eq!(actual.profile, BrowserProfile::for_conversation(&services.data_dir, &key, temporary));
            workspaces.close_idle(key.clone(), workspace.runtime_generation()).await.unwrap();
        }
    }
    {
        let records = created.lock().unwrap();
        assert_eq!(records.len(), 6);
        assert_eq!(records[0].profile, records[1].profile);
        assert_eq!(records[2].profile, records[3].profile);
        assert_ne!(records[0].profile, records[2].profile);
        assert_eq!(records[4].profile, BrowserProfile::Ephemeral);
        assert_eq!(records[5].profile, BrowserProfile::Ephemeral);
    }
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
