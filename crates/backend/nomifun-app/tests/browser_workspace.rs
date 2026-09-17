#![cfg(feature = "browser-use")]

#[path = "../src/browser_workspace_provider.rs"]
mod browser_provider;

use async_trait::async_trait;
use nomifun_agent_contracts::{ActionId, ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding};
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

#[test]
fn canonical_browser_rest_routes_use_the_provider_neutral_agent_session_binder() {
    let source = include_str!("../src/router/browser_workspace.rs");
    for handler in ["async fn snapshot(", "async fn ensure(", "async fn command("] {
        assert!(source.contains(handler), "missing canonical route {handler}");
    }
    assert!(source.contains("bind_authorized_resource("));
    assert!(source.contains("BoundBrowserProviderResource"));
    assert!(source.contains("BrowserProviderKind::Managed"));
    assert!(source.contains("BrowserProviderKind::AttachedChrome"));
    assert!(
        !source.contains("async fn ensure_resource("),
        "routes must not unconditionally materialize the managed provider"
    );
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
            ("persistence".to_owned(), "ephemeral".to_owned()),
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

fn request(owner: &str, session: &str) -> nomifun_ai_agent::factory::BrowserRuntimeRequest {
    nomifun_ai_agent::factory::BrowserRuntimeRequest {
        principal_id: owner.to_owned(),
        agent_session_id: session.to_owned(),
        action_allowlist: BTreeSet::from([
            ActionId::from("browser/observe"),
            ActionId::from("browser/navigate"),
        ]),
        provider: Some(provider()),
        resource_binding: Some(binding(owner)),
    }
}

#[tokio::test]
async fn managed_browser_resource_isolated_by_any_authorized_agent_session() {
    let owner = "0190f5fe-7c00-7a00-8000-000000000001";
    let regular = "0190f5fe-7c00-7a00-8000-000000000081";
    let delegated = "0190f5fe-7c00-7a00-8000-000000000082";
    let root = tempfile::tempdir().unwrap();
    let factory = Arc::new(Factory::default());
    let resources = Arc::new(BrowserResourceService::new(factory.clone()));
    let resolve = browser_provider::resolver(
        Some(resources.clone()),
        None,
        root.path().to_path_buf(),
        Arc::from(owner),
    );

    let regular_resource = resolve(request(owner, regular))
        .await
        .unwrap()
        .expect("regular AgentSession Browser Resource");
    let delegated_resource = resolve(request(owner, delegated))
        .await
        .unwrap()
        .expect("delegated AgentSession Browser Resource");
    let (
        BoundBrowserProviderResource::Managed(regular_resource),
        BoundBrowserProviderResource::Managed(delegated_resource),
    ) = (regular_resource, delegated_resource)
    else {
        panic!("managed provider expected")
    };
    assert_eq!(regular_resource.authority().agent_session_id(), regular);
    assert_eq!(delegated_resource.authority().agent_session_id(), delegated);
    assert_ne!(regular_resource.authority().key(), delegated_resource.authority().key());
    assert!(!Arc::ptr_eq(&regular_resource, &delegated_resource));

    let first_run = regular_resource.begin_run().await.unwrap();
    regular_resource
        .agent_command(
            &first_run,
            BrowserTabCommand::Create { url: "https://example.com/".into() },
        )
        .await
        .unwrap();
    regular_resource.finish_run(&first_run).await.unwrap();
    assert_eq!(factory.created.lock().unwrap().len(), 1);
    resources.shutdown().await.unwrap();
}

#[tokio::test]
async fn provider_and_resource_never_create_browser_action_authority() {
    let owner = "0190f5fe-7c00-7a00-8000-000000000001";
    let session = "0190f5fe-7c00-7a00-8000-000000000083";
    let root = tempfile::tempdir().unwrap();
    let resources = Arc::new(BrowserResourceService::new(Arc::new(Factory::default())));
    let resolve = browser_provider::resolver(
        Some(resources.clone()),
        None,
        root.path().to_path_buf(),
        Arc::from(owner),
    );

    let mut no_actions = request(owner, session);
    no_actions.action_allowlist.clear();
    assert!(resolve(no_actions).await.unwrap().is_none());

    let mut wrong_owner = request(owner, session);
    wrong_owner.principal_id = "0190f5fe-7c00-7a00-8000-000000000002".into();
    assert!(resolve(wrong_owner).await.is_err());

    let mut missing_provider = request(owner, session);
    missing_provider.provider = None;
    assert!(resolve(missing_provider).await.is_err());

    let mut missing_binding = request(owner, session);
    missing_binding.resource_binding = None;
    assert!(resolve(missing_binding).await.is_err());

    assert!(resources
        .get_for_agent_session(owner, session)
        .await
        .unwrap()
        .is_none());
    resources.shutdown().await.unwrap();
}

#[tokio::test]
async fn canonical_route_binder_selects_managed_and_attached_without_fallback() {
    let owner = "0190f5fe-7c00-7a00-8000-000000000001";
    let managed_session = "0190f5fe-7c00-7a00-8000-000000000085";
    let attached_session = "0190f5fe-7c00-7a00-8000-000000000086";
    let root = tempfile::tempdir().unwrap();
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
        true,
    )
    .await
    .unwrap();
    let managed_snapshot = managed.snapshot().await.unwrap();
    assert_eq!(
        managed_snapshot.provider_kind,
        nomifun_browser_platform::product::BrowserProviderKind::Managed
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
        false,
    )
    .await
    .unwrap();
    let attached_snapshot = attached_resource.snapshot().await.unwrap();
    assert_eq!(
        attached_snapshot.provider_kind,
        nomifun_browser_platform::product::BrowserProviderKind::AttachedChrome
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
fn browser_profile_uses_agent_session_and_binding_not_conversation_product_identity() {
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
    assert!(path.contains("browser-v3"));
    assert!(!path.contains("conversation"));
}
