//! Knowledge's non-Agent Browser Role operation. The application freezes the
//! installed Provider once; Kernel verifies it again for every source fetch.
use crate::headless_render::HeadlessRenderRuntime;
use nomifun_agent_contracts::*;
use nomifun_agent_domain_wave2::{
    Wave2HostPortError, Wave2OperationToolHostPort, Wave2OperationToolHostRequest,
    Wave2TypedCapabilityOperation,
};
use nomifun_agent_kernel::{
    KernelRegistry, RoleMemberAdmission, RoleMemberInvocationRequest, RoleToolOperationRequest,
};
use nomifun_common::AppError;
use nomifun_knowledge::source_url::{
    BrowserRenderContent, BrowserRenderContentPort, BrowserRenderContentRequest,
};
use std::{collections::BTreeSet, sync::Arc};

const MEMBER: &str = "browser.render_content";
const ACTION: &str = "browser.render_content.invoke";
fn principal(owner: &str) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "service".into(),
        principal_id: format!("knowledge-render:{owner}"),
    }
}
fn invalid() -> Wave2HostPortError {
    Wave2HostPortError::new(
        "BROWSER_RENDER_ADMISSION_REJECTED",
        "Invalid Knowledge browser operation authority",
    )
}

pub(crate) struct RenderRoleHost {
    runtime: Arc<HeadlessRenderRuntime>,
    principal: PrincipalRef,
}
impl RenderRoleHost {
    pub(crate) fn new(runtime: Arc<HeadlessRenderRuntime>, owner: &str) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            principal: principal(owner),
        })
    }
}
impl Wave2OperationToolHostPort for RenderRoleHost {
    fn invoke<'a>(
        &'a self,
        request: Wave2OperationToolHostRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<StrictJsonValue, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let context = &request.context;
            let provider = &context.provider_lock.provider;
            if context.principal != self.principal
                || context.role_id.as_ref() != nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID
                || context.state_scope_key.as_ref() != "knowledge:browser-render"
                || context.agent_session_id.is_some()
                || context.resolved_snapshot_ref.is_some()
                || context.member_id.as_ref() != MEMBER
                || request.action_id.as_ref() != ACTION
                || !context.resource_bindings.is_empty()
                || provider.role.key.role_id.as_ref()
                    != nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID
                || provider.role.key.contract_version.as_ref()
                    != nomifun_agent_domain_wave2::BROWSER_ROLE_CONTRACT_VERSION
                || provider.mount_id.as_ref() != nomifun_agent_domain_wave2::BROWSER_MOUNT_ID
                || provider.package.id.as_ref() != nomifun_agent_domain_wave2::BROWSER_PACKAGE_ID
            {
                return Err(invalid());
            }
            let Wave2TypedCapabilityOperation::BrowserRenderContent { input } = request.operation
            else {
                return Err(invalid());
            };
            nomifun_agent_domain_wave2::validate_action_input(MEMBER, &input).map_err(|_| {
                Wave2HostPortError::new("INVALID_PAYLOAD", "Invalid canonical render input")
            })?;
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                url: String,
            }
            let input: Input = serde_json::from_value(input.0).map_err(|_| {
                Wave2HostPortError::new("INVALID_PAYLOAD", "Expected a source URL only")
            })?;
            let url = url::Url::parse(&input.url)
                .map_err(|_| Wave2HostPortError::new("INVALID_PAYLOAD", "Invalid source URL"))?;
            if input.url.len() > 8192
                || !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(Wave2HostPortError::new(
                    "INVALID_PAYLOAD",
                    "Invalid source URL",
                ));
            }
            let output = self.runtime.render(url).await.map_err(|error| {
                Wave2HostPortError::new("BROWSER_RENDER_FAILED", error.to_string())
            })?;
            Ok(StrictJsonValue(
                serde_json::json!({"final_url":output.final_url,"html":output.html,"html_truncated":output.html_truncated}),
            ))
        })
    }
}

pub(crate) struct KnowledgeBrowserPort {
    kernel: Arc<KernelRegistry>,
    lock: ResolvedRoleProviderLock,
    principal: PrincipalRef,
}
impl KnowledgeBrowserPort {
    pub(crate) fn bind(kernel: Arc<KernelRegistry>, owner: &str) -> anyhow::Result<Arc<Self>> {
        let registry = kernel.snapshot()?;
        let provider = registry
            .role_provider(
                &nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into(),
                &nomifun_agent_domain_wave2::BROWSER_MOUNT_ID.into(),
            )
            .ok_or_else(|| anyhow::anyhow!("Installed Browser Provider is unavailable"))?;
        anyhow::ensure!(
            provider.source.source_kind == PluginSourceKind::Bundled
                && provider.contribution.members.contains_key(&MEMBER.into()),
            "Invalid installed Browser Provider"
        );
        anyhow::ensure!(
            registry.capability(&MEMBER.into()).is_some_and(|cap| cap
                .manifest
                .supports_consumer(CapabilityConsumer::Knowledge)),
            "Browser member does not support Knowledge"
        );
        let lock = ResolvedRoleProviderLock {
            provider: provider.provider.clone(),
            source: provider.source.clone(),
            supported_members: provider.contribution.members.keys().cloned().collect(),
        };
        Ok(Arc::new(Self {
            kernel,
            lock,
            principal: principal(owner),
        }))
    }
    fn request(
        &self,
        input: BrowserRenderContentRequest,
    ) -> Result<RoleToolOperationRequest, AppError> {
        let registry = self
            .kernel
            .snapshot()
            .map_err(|e| AppError::Conflict(e.to_string()))?;
        let id = format!("knowledge-render:{}", uuid::Uuid::now_v7());
        Ok(RoleToolOperationRequest {
            member: RoleMemberInvocationRequest {
                principal: self.principal.clone(),
                session_owner: self.principal.clone(),
                operation_id: id.clone().into(),
                correlation_id: id.clone().into(),
                capability_id: MEMBER.into(),
                resource_binding_ids: BTreeSet::new(),
                state_scope_key: "knowledge:browser-render".into(),
                admission: RoleMemberAdmission::Operation {
                    provider_lock: self.lock.clone(),
                    registry_generation: registry.generation,
                    registry_digest: registry.registry_digest.clone(),
                    resource_bindings: vec![],
                },
            },
            action_id: ACTION.into(),
            idempotency_key: id.into(),
            input: StrictJsonValue(serde_json::json!({"url":input.url})),
        })
    }
}
#[async_trait::async_trait]
impl BrowserRenderContentPort for KnowledgeBrowserPort {
    async fn render_content(
        &self,
        input: BrowserRenderContentRequest,
    ) -> Result<BrowserRenderContent, AppError> {
        let result = self
            .kernel
            .invoke_role_tool(self.request(input)?)
            .await
            .map_err(|error| {
                AppError::Conflict(format!("Canonical browser rendering failed: {error}"))
            })?;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Output {
            final_url: String,
            html: String,
            html_truncated: bool,
        }
        let output: Output = serde_json::from_value(result.0)
            .map_err(|_| AppError::Internal("Invalid browser render result".into()))?;
        Ok(BrowserRenderContent {
            final_url: output.final_url,
            html: output.html,
            html_truncated: output.html_truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    fn registry(
        runtime: Arc<HeadlessRenderRuntime>,
    ) -> (
        Arc<KernelRegistry>,
        Vec<nomifun_agent_kernel::PluginRegistration>,
    ) {
        let mut ports = nomifun_agent_domain_wave2::Wave2RoleHostPorts::with_actions(
            nomifun_agent_domain_wave2::unconfigured_host_port(),
        );
        ports.browser_operation_tools = RenderRoleHost::new(runtime, "test-owner");
        let registrations =
            nomifun_agent_domain_wave2::registrations_with_role_host_ports(ports).unwrap();
        let kernel = Arc::new(
            KernelRegistry::new(
                nomifun_agent_kernel::MaterializationPolicy::stable("1.0.0"),
                Arc::new(nomifun_agent_kernel::InMemoryPluginStatePersistence::new()),
            )
            .unwrap(),
        );
        kernel.replace_all(registrations.clone()).unwrap();
        (kernel, registrations)
    }
    #[tokio::test]
    async fn knowledge_dispatches_through_the_exact_non_agent_provider() {
        let (runtime, calls) = crate::headless_render::test_support::runtime();
        let (kernel, _) = registry(runtime.clone());
        let port = KnowledgeBrowserPort::bind(kernel, "test-owner").unwrap();
        let content = port
            .render_content(BrowserRenderContentRequest::new("https://example.com/"))
            .await
            .unwrap();
        assert_eq!(content.final_url, "https://example.com/");
        assert!(content.html.contains("selected Provider"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        runtime.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn wrong_principal_payload_and_action_fail_before_rendering() {
        let (runtime, calls) = crate::headless_render::test_support::runtime();
        let (kernel, _) = registry(runtime.clone());
        let port = KnowledgeBrowserPort::bind(kernel.clone(), "test-owner").unwrap();
        let mut wrong = port
            .request(BrowserRenderContentRequest::new("https://example.com/"))
            .unwrap();
        wrong.member.principal = principal("another-owner");
        wrong.member.session_owner = wrong.member.principal.clone();
        assert!(kernel.invoke_role_tool(wrong).await.is_err());
        let mut wrong = port
            .request(BrowserRenderContentRequest::new("https://example.com/"))
            .unwrap();
        wrong.action_id = "browser.navigate.invoke".into();
        assert!(kernel.invoke_role_tool(wrong).await.is_err());
        let mut wrong = port
            .request(BrowserRenderContentRequest::new("https://example.com/"))
            .unwrap();
        wrong.input.0["chrome_path"] = serde_json::json!("untrusted.exe");
        assert!(kernel.invoke_role_tool(wrong).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        runtime.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn changed_provider_source_is_not_silently_reselected() {
        let (runtime, calls) = crate::headless_render::test_support::runtime();
        let (kernel, mut registrations) = registry(runtime.clone());
        let port = KnowledgeBrowserPort::bind(kernel.clone(), "test-owner").unwrap();
        let provider = registrations
            .iter_mut()
            .find(|registration| {
                registration.metadata.mount_id.as_ref()
                    == nomifun_agent_domain_wave2::BROWSER_MOUNT_ID
            })
            .unwrap();
        let digest = digest_payload(&serde_json::json!({"changed":true})).unwrap();
        provider.metadata.source.source_digest = Some(digest.clone());
        provider.metadata.context.source.source_digest = Some(digest);
        kernel.replace_all(registrations).unwrap();
        assert!(
            port.render_content(BrowserRenderContentRequest::new("https://example.com/"))
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn application_composition_writes_a_rendered_knowledge_snapshot() {
        let (runtime, calls) = crate::headless_render::test_support::runtime();
        let root = tempfile::tempdir().unwrap();
        let services = crate::services::AppServices::try_from_config_with_host(
            nomifun_db::init_database_memory().await.unwrap(),
            &crate::AppConfig {
                data_dir: root.path().join("data"),
                work_dir: root.path().join("work"),
                auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
                local_trust_secret: Some(Arc::from("knowledge-render-fixture")),
                ..Default::default()
            },
            crate::DesktopHostServices {
                headless_render: Some(runtime),
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|error| panic!("isolated application setup: {error:#}"));
        let router = crate::compatibility::create_router(&services).await;
        let source=serde_json::from_value(serde_json::json!({"mode":"snapshot","entries":[{"url":"https://example.com/render-only","rendered":true}]})).unwrap();
        let base = services
            .knowledge_service
            .create_base("Rendered canonical fixture", "", None, Some(source))
            .await
            .unwrap();
        let fetched = base.source_fetch.as_ref().expect("source fetch summary");
        assert_eq!(fetched.failed, 0, "{:?}", fetched.errors);
        assert_eq!(fetched.fetched, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let files = services
            .knowledge_service
            .list_files(base.knowledge_base_id.as_ref())
            .await
            .unwrap();
        let snapshot = files
            .iter()
            .find(|file| file.rel_path.starts_with("snapshots/"))
            .expect("managed snapshot entry");
        let content = services
            .knowledge_service
            .read_file(base.knowledge_base_id.as_ref(), &snapshot.rel_path)
            .await
            .unwrap();
        assert!(
            content
                .content
                .contains("Rendered by the selected Provider")
        );
        drop(router);
        services.shutdown_nomi_core_host().await.unwrap();
        services.database.close().await;
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_SEARCH_CHROME; real Chromium, no public network"]
    async fn real_knowledge_operation_blocks_private_targets_without_http_fallback() {
        let chrome = std::path::PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").unwrap());
        let product = nomi_browser_engine::headless_page::probe_runtime(chrome.clone())
            .await
            .unwrap();
        let mut host = crate::DesktopHostServices::default();
        host.set_browser_release(chrome, product).await.unwrap();
        let trap = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/private", trap.local_addr().unwrap());
        let root = tempfile::tempdir().unwrap();
        let services = crate::services::AppServices::try_from_config_with_host(
            nomifun_db::init_database_memory().await.unwrap(),
            &crate::AppConfig {
                data_dir: root.path().join("data"),
                work_dir: root.path().join("work"),
                auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
                local_trust_secret: Some(Arc::from("knowledge-render-real")),
                ..Default::default()
            },
            host,
        )
        .await
        .unwrap_or_else(|error| panic!("real render setup: {error:#}"));
        let router = crate::compatibility::create_router(&services).await;
        let source = serde_json::from_value(
            serde_json::json!({"mode":"snapshot","entries":[{"url":url,"rendered":true}]}),
        )
        .unwrap();
        let base = services
            .knowledge_service
            .create_base("Blocked private render", "", None, Some(source))
            .await
            .unwrap();
        let summary = base.source_fetch.unwrap();
        assert_eq!(summary.fetched, 0);
        assert_eq!(summary.failed, 1);
        assert!(
            summary
                .errors
                .iter()
                .any(|error| error.contains("BROWSER_RENDER_FAILED")),
            "{:?}",
            summary.errors
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), trap.accept())
                .await
                .is_err()
        );
        drop(router);
        services.shutdown_nomi_core_host().await.unwrap();
        services.database.close().await;
    }
}
