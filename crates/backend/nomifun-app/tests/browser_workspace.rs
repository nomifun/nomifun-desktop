#![cfg(feature = "browser-use")]

#[path = "../src/browser_workspace_provider.rs"]
mod browser_workspace_provider;
use browser_workspace_provider as browser_provider;
pub use browser_workspace_provider::attached_provider::AttachedChromeProviderService;
#[path = "../src/router/browser_workspace.rs"]
mod browser_routes;

use async_trait::async_trait;
use nomifun_agent_contracts::{ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding};
use nomifun_browser_platform::{
    attached_browser::AttachedBrowserRuntimeError,
    bound_resource::BoundBrowserProviderResource,
    product::{BrowserSessionAuthority, BROWSER_RESOURCE_KIND},
    run_guard::{NativeInputGate, RunAdmissionError},
    runtime::{
        BrowserNativeSurfacePort, BrowserProfile, BrowserRuntime, BrowserRuntimeFactory,
        BrowserRuntimeSnapshot, BrowserTabCommand, CreateBrowserRuntime, WorkspaceError,
    },
    workspace::BrowserResourceService,
};
use std::{collections::{BTreeMap, BTreeSet}, sync::{Arc, Mutex}};


struct IdleUserClose(Arc<BrowserResourceService>);
#[async_trait]
impl browser_workspace_provider::BrowserUserClosePort for IdleUserClose {
    async fn close_user_workspace(&self, principal: &str, session: &str, generation: u64) -> Result<(), WorkspaceError> {
        self.0.close_idle(nomifun_browser_platform::workspace::managed_workspace_key(principal, session)?, generation).await
    }
}

#[derive(Default)]
struct Factory {
    created: Mutex<Vec<CreateBrowserRuntime>>,
}

struct Runtime(u64);

#[async_trait]
impl BrowserRuntimeFactory for Factory {
    async fn create(
        &self,
        request: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        self.created.lock().unwrap().push(request.clone());
        Ok(Arc::new(Runtime(request.runtime_generation)))
    }
}

#[async_trait]
impl NativeInputGate for Runtime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
}

#[async_trait]
impl BrowserRuntime for Runtime {
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { None }

    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        Ok(BrowserRuntimeSnapshot {
            downloads: Vec::new(),
            runtime_generation: self.0,
            revision: 1,
            active_tab_id: None,
            tabs: Vec::new(),
        })
    }

    async fn execute(
        &self,
        _: BrowserTabCommand,
        _: tokio_util::sync::CancellationToken,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        self.snapshot().await
    }

    async fn close(&self) -> Result<(), WorkspaceError> { Ok(()) }
}

fn provider() -> nomifun_agent_contracts::ExactRoleProviderRef {
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

fn binding(owner: &str) -> TypedResourceBinding {
    TypedResourceBinding {
        binding_id: ResourceBindingId::from("browser:managed-browser"),
        resource_kind: ResourceKind::from(BROWSER_RESOURCE_KIND),
        resource_id: ResourceId::from("managed-browser"),
        owner_id: owner.to_owned(),
        operations: BTreeSet::from([
            "observe".to_owned(),
            "navigate".to_owned(),
            "act".to_owned(),
        ]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::from([
            ("provider_kind".to_owned(), "managed".to_owned()),
        ]),
    }
}

fn attached_binding(owner: &str) -> TypedResourceBinding {
    let mut binding = binding(owner);
    binding
        .typed_parameters
        .insert("provider_kind".to_owned(), "attached_chrome".to_owned());
    binding
}

struct SessionVerifier;

#[async_trait]
impl browser_provider::attached_provider::AttachedBrowserSessionVerifier for SessionVerifier {
    async fn verify(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), AttachedBrowserRuntimeError> {
        if principal_id == "0190f5fe-7c00-7a00-8000-000000000001"
            && agent_session_id.starts_with("0190f5fe")
        {
            Ok(())
        } else {
            Err(AttachedBrowserRuntimeError::ActionDenied)
        }
    }
}

#[tokio::test]
async fn canonical_route_binder_selects_managed_and_attached_without_fallback() {
    let owner = "0190f5fe-7c00-7a00-8000-000000000001";
    let managed_session = "0190f5fe-7c00-7a00-8000-000000000085";
    let attached_session = "0190f5fe-7c00-7a00-8000-000000000086";
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("storage-generation"), "0190f5fe-7c00-7a00-8000-000000000002").unwrap();
    let resources = Arc::new(BrowserResourceService::new(Arc::new(Factory::default())));
    let attached = browser_provider::attached_provider::AttachedChromeProviderService::new();
    attached
        .install_session_verifier(Arc::new(SessionVerifier))
        .unwrap();

    let managed_binding = binding(owner);
    let managed_authority = BrowserSessionAuthority::from_action_ids(
        owner,
        managed_session,
        ["browser/observe", "browser/navigate"],
        browser_provider::browser_resource_binding(
            managed_binding.clone(),
            browser_provider::provider_descriptor(&provider(), &managed_binding).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let managed = browser_provider::bind_authorized_resource(
        Some(resources.clone()),
        Some(attached.clone()),
        root.path(),
        managed_authority,
    )
    .await
    .unwrap();
    let managed_snapshot = managed.snapshot().await.unwrap();
    assert_eq!(
        managed_snapshot.provider_kind,
        nomifun_browser_platform::product::BrowserProviderKind::Managed
    );
    assert_eq!(
        managed_snapshot.allowed_actions,
        BTreeSet::from([
            "browser/navigate".to_owned(),
            "browser/observe".to_owned(),
        ])
    );
    assert!(matches!(managed, BoundBrowserProviderResource::Managed(_)));
    assert!(resources
        .get_for_agent_session(owner, managed_session)
        .await
        .unwrap()
        .is_some());

    let attached_binding = attached_binding(owner);
    let attached_authority = BrowserSessionAuthority::from_action_ids(
        owner,
        attached_session,
        ["browser/observe", "browser/navigate"],
        browser_provider::browser_resource_binding(
            attached_binding.clone(),
            browser_provider::provider_descriptor(&provider(), &attached_binding).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let attached_resource = browser_provider::bind_authorized_resource(
        Some(resources.clone()),
        Some(attached),
        root.path(),
        attached_authority,
    )
    .await
    .unwrap();
    let attached_snapshot = attached_resource.snapshot().await.unwrap();
    assert_eq!(
        attached_snapshot.provider_kind,
        nomifun_browser_platform::product::BrowserProviderKind::AttachedChrome
    );
    assert_eq!(
        attached_snapshot.allowed_actions,
        BTreeSet::from([
            "browser/navigate".to_owned(),
            "browser/observe".to_owned(),
        ])
    );
    assert!(attached_snapshot.runtime.is_none());
    assert!(matches!(
        attached_resource,
        BoundBrowserProviderResource::AttachedChrome(_)
    ));
    assert!(resources
        .get_for_agent_session(owner, attached_session)
        .await
        .unwrap()
        .is_none());
}

#[test]
fn browser_profile_uses_canonical_owner_and_session_identity() {
    let authority = browser_provider::browser_resource_binding(
        binding("0190f5fe-7c00-7a00-8000-000000000001"),
        browser_provider::provider_descriptor(
            &provider(),
            &binding("0190f5fe-7c00-7a00-8000-000000000001"),
        )
        .unwrap(),
    )
    .unwrap();
    let authority = nomifun_browser_platform::product::BrowserSessionAuthority::from_action_ids(
        "0190f5fe-7c00-7a00-8000-000000000001",
        "0190f5fe-7c00-7a00-8000-000000000084",
        ["browser/observe"],
        authority,
    )
    .unwrap();
    let profile = BrowserProfile::for_agent_session(
        std::path::Path::new("C:/nomifun-data"),
        &authority.key(),
        false,
    );
    let BrowserProfile::Persistent(path) = profile else { panic!("persistent profile") };
    let path = path.to_string_lossy();
    assert!(path.contains("browser-v4"));
    assert!(!path.contains("conversation"));
}

#[tokio::test]
async fn user_api_opens_without_agent_browser_grants_and_rejects_foreign_owners_and_preparation_races() {
    use axum::{body::Body, http::{Request, StatusCode}};
    use nomifun_agent_contracts::{AgentBindingValue, AgentPresetId, DigestHex, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef};
    use tower::ServiceExt;
    let db = nomifun_db::init_database_memory().await.unwrap();
    let sessions = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(db.pool().clone()).await.unwrap();
    let owner_id = "0190f5fe-7c00-7a00-8000-000000000001";
    let owner = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() };
    let binding = AgentBindingValue {
        preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("user-browser-no-grant"), revision: 1, revision_digest: DigestHex::from("a".repeat(64)) },
        resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: ResolvedSnapshotId::from("user-browser-no-grant"), snapshot_digest: DigestHex::from("b".repeat(64)) },
        typed_resource_bindings: vec![], binding_version: 1,
    };
    let opened = sessions.open(owner.clone(), binding.clone(), None, vec![], "user-browser-route", 1).await.unwrap();
    let session = opened.session.agent_session_id;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("storage-generation"), "0190f5fe-7c00-7a00-8000-000000000002").unwrap();
    let factory = Arc::new(Factory::default());
    let resources = Arc::new(BrowserResourceService::new(factory.clone()).with_profile_store(
        nomifun_browser_platform::runtime::BrowserProfileStore::new(root.path()).unwrap()));
    let router = browser_routes::routes(browser_routes::BrowserResourceApiState {
        resources: Some(resources.clone()), attached_chrome: None, sessions: sessions.clone(), data_dir: root.path().to_path_buf(),
        operation_locks: Arc::new(dashmap::DashMap::new()), close_owner: Arc::new(IdleUserClose(resources.clone())),
    });
    let base = format!("/api/agent-sessions/{}/browser", session.as_ref());
    let request = |id: &str, method: &str, path: String, body: &str| {
        let mut request = Request::builder().method(method).uri(path).header("content-type", "application/json").body(Body::from(body.to_owned())).unwrap();
        request.extensions_mut().insert(nomifun_auth::CurrentUser { id: nomifun_common::UserId::parse(id.to_owned()).unwrap(), username: "route-fixture".into() });
        request
    };
    let ensure = router.clone().oneshot(request(owner_id, "POST", base.clone(), "{}")).await.unwrap();
    assert_eq!(ensure.status(), StatusCode::OK);
    let payload: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(ensure.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(payload["data"]["browser_id"], "managed-browser");
    assert!(payload["data"].get("allowed_actions").is_none());
    assert!(payload["data"].get("resource_binding_id").is_none());
    let create = router.clone().oneshot(request(owner_id, "POST", format!("{base}/commands"), r#"{"command":"create","url":"https://example.test/"}"#)).await.unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    assert_eq!(factory.created.lock().unwrap().len(), 1, "the native runtime port was used without Agent Browser authority");
    let observed = sessions.get(&owner, &session).await.unwrap();
    assert_eq!(observed.session.agent_binding, binding, "user browsing never writes a canonical Agent grant or binding");
    assert!(sessions.active_capability_ids(&owner, &session).await.unwrap().is_empty());
    let foreign = router.clone().oneshot(request("0190f5fe-7c00-7a00-8000-000000000002", "POST", base.clone(), "{}")).await.unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    assert_eq!(factory.created.lock().unwrap().len(), 1);

    sessions.start_turn(&owner, &session, "preparing-no-browser", serde_json::json!({"content":"ordinary chat"})).await.unwrap();
    // The canonical Turn is active but its global native gate is not yet proven.
    let preparing = router.clone().oneshot(request(owner_id, "POST", base.clone(), "{}")).await.unwrap();
    assert_eq!(preparing.status(), StatusCode::CONFLICT);
    let blocked = router.clone().oneshot(request(owner_id, "POST", format!("{base}/commands"), r#"{"command":"create","url":"https://example.test/"}"#)).await.unwrap();
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    assert_eq!(factory.created.lock().unwrap().len(), 1, "preparation cannot create a user-ready native child");
    let snapshot = router.clone().oneshot(request(owner_id, "GET", base.clone(), "")).await.unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(snapshot.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(snapshot["data"]["run"]["input_state"], "agent_running");
    let user = resources.get_for_agent_session(owner_id, session.as_ref()).await.unwrap().unwrap();
    let guard = user.begin_run().await.unwrap();
    let already_locked = router.oneshot(request(owner_id, "POST", base, "{}")).await.unwrap();
    assert_eq!(already_locked.status(), StatusCode::OK, "a first pane may attach to a genuinely locked runtime during chat");
    let locked: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(already_locked.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(locked["data"]["run"]["input_state"], "agent_running");
    user.finish_run(&guard).await.unwrap();
    resources.shutdown().await.unwrap();
    db.close().await;
}
