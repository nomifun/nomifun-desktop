//! Production composition for [`crate::ChatModelBroker`].
//!
//! This crate intentionally does not depend on the application database or on
//! `nomifun-model-invoke`.  The former would put the model kernel above the
//! repository layer in the Cargo graph, while the latter currently exposes no
//! public chat-stream or resolved-call API.  The small ports below are the
//! dependency-direction boundary: the application wraps its existing provider,
//! connection, and model repositories once, and this module owns the exact
//! broker composition after that point.
//!
//! In particular, this module never reconstructs a route from a provider name,
//! never decrypts credentials into a wire request, and never turns the existing
//! non-chat `ModelInvokeService::invoke` API into a guessed chat response.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use thiserror::Error;

use crate::adapter::{
    AnthropicAdapter, BedrockAdapter, ChatProtocolAdapter, GeminiAdapter, OpenAiChatAdapter,
    OpenAiResponsesAdapter, ProviderTransport, ProviderWireRequest, ProviderWireStream,
    VertexAdapter,
};
use crate::broker::{
    BrokerRetryPolicy, ChatBrokerPort, ChatModelBroker, ChatModelStream,
};
use crate::contracts::{
    ChatModelError, ChatModelErrorCode, ChatModelRequest, ChatProtocol, ChatRetryDirective,
    ChatRouteSelection, ProviderCredentialRef, ProviderIdRef, ResolvedChatRoute,
    ResolvedChatRouteSet,
};
use crate::ports::{
    ChatCausalityGate, ChatRouteResolver, CredentialLease, CredentialTarget,
    ProviderCredentialStore,
};
use nomifun_agent_contracts::{ConnectionConfigRef, DigestHex};

/// Safe, fixed-shape failures returned by a repository bridge.
///
/// The enum deliberately has no free-form diagnostic field.  Repository
/// errors can contain SQL, URLs, or credential-adjacent values; none of those
/// values should become a broker error or a log field.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ProductionRepositoryError {
    #[error("required provider data is missing")]
    Missing,
    #[error("provider repository is unavailable")]
    Unavailable,
    #[error("provider repository returned invalid data")]
    InvalidData,
    #[error("provider repository denied the operation")]
    PermissionDenied,
}

/// The non-secret provider revision needed to fence a resolved route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRepositoryRecord {
    pub provider_id: ProviderIdRef,
    pub enabled: bool,
    pub config_revision_digest: DigestHex,
}

/// The non-secret connection identity bound to a resolved route.
///
/// Credential material is intentionally absent.  The connection bridge leases
/// an opaque [`CredentialLease`] separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionRepositoryRecord {
    pub provider_id: ProviderIdRef,
    pub connection_config_ref: ConnectionConfigRef,
    pub credential_ref: ProviderCredentialRef,
}

/// Adapter-facing provider repository.
///
/// An implementation normally wraps the existing provider repository and
/// computes the same canonical config digest that was captured in the model
/// route record.  A missing or disabled provider is not a fallback condition.
#[async_trait]
pub trait ProductionProviderRepository: Send + Sync {
    async fn find_provider(
        &self,
        provider_id: &ProviderIdRef,
    ) -> Result<Option<ProviderRepositoryRecord>, ProductionRepositoryError>;
}

/// Adapter-facing model repository.
///
/// The returned set is already the exact primary plus deterministic failover
/// order selected by the persisted model-route reference.  This port does not
/// permit name-based inference or an implicit provider preset.
#[async_trait]
pub trait ProductionModelRepository: Send + Sync {
    async fn resolve_chat_route(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<Option<ResolvedChatRouteSet>, ProductionRepositoryError>;
}

/// Adapter-facing connection repository and credential authority.
///
/// A host implementation can use its existing connection repository for both
/// operations.  `lease_credential` is the only place where the encryption key
/// crosses this module boundary; it must validate and decrypt internally, then
/// return only a non-empty opaque process-local handle.
#[async_trait]
pub trait ProductionConnectionRepository: Send + Sync {
    async fn find_connection(
        &self,
        route: &ResolvedChatRoute,
    ) -> Result<Option<ConnectionRepositoryRecord>, ProductionRepositoryError>;

    async fn lease_credential(
        &self,
        credential_ref: &ProviderCredentialRef,
        target: &CredentialTarget,
        encryption_key: &[u8; 32],
    ) -> Result<Option<CredentialLease>, ProductionRepositoryError>;
}

/// The three existing provider repository roles as a single explicit bundle.
#[derive(Clone)]
pub struct ProductionRepositorySet {
    pub provider_repository: Arc<dyn ProductionProviderRepository>,
    pub connection_repository: Arc<dyn ProductionConnectionRepository>,
    pub model_repository: Arc<dyn ProductionModelRepository>,
}

impl ProductionRepositorySet {
    pub fn new(
        provider_repository: Arc<dyn ProductionProviderRepository>,
        connection_repository: Arc<dyn ProductionConnectionRepository>,
        model_repository: Arc<dyn ProductionModelRepository>,
    ) -> Self {
        Self {
            provider_repository,
            connection_repository,
            model_repository,
        }
    }
}

/// Construction inputs for the production broker.
///
/// `model_invoke` is a deliberately narrow one-attempt adapter.  It is not a
/// second model router and it cannot select a provider or retry a request.
pub struct ProductionBrokerDependencies {
    pub repositories: ProductionRepositorySet,
    pub encryption_key: [u8; 32],
    pub causality_gate: Arc<dyn ChatCausalityGate>,
    pub model_invoke: Arc<dyn ChatModelInvokePort>,
    pub retry_policy: BrokerRetryPolicy,
}

impl ProductionBrokerDependencies {
    pub fn new(
        provider_repository: Arc<dyn ProductionProviderRepository>,
        connection_repository: Arc<dyn ProductionConnectionRepository>,
        model_repository: Arc<dyn ProductionModelRepository>,
        encryption_key: [u8; 32],
        causality_gate: Arc<dyn ChatCausalityGate>,
        model_invoke: Arc<dyn ChatModelInvokePort>,
    ) -> Self {
        Self {
            repositories: ProductionRepositorySet::new(
                provider_repository,
                connection_repository,
                model_repository,
            ),
            encryption_key,
            causality_gate,
            model_invoke,
            retry_policy: BrokerRetryPolicy::default(),
        }
    }

    pub fn with_retry_policy(mut self, retry_policy: BrokerRetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }
}

/// Construction failures that are safe to expose to the application.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProductionBrokerError {
    #[error("the model-invoke adapter owns retry; ChatModelBroker must be the sole retry owner")]
    RetryBoundary,
    #[error("ChatModelBroker construction failed: {0:?}")]
    Broker(ChatModelError),
}

/// One exact route resolver backed by the three repository ports.
pub struct ProductionRouteResolver {
    provider_repository: Arc<dyn ProductionProviderRepository>,
    connection_repository: Arc<dyn ProductionConnectionRepository>,
    model_repository: Arc<dyn ProductionModelRepository>,
}

impl ProductionRouteResolver {
    pub fn new(repositories: ProductionRepositorySet) -> Self {
        Self {
            provider_repository: repositories.provider_repository,
            connection_repository: repositories.connection_repository,
            model_repository: repositories.model_repository,
        }
    }

    async fn resolve_repository_route(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<ResolvedChatRouteSet, ChatModelError> {
        let routes = self
            .model_repository
            .resolve_chat_route(selection)
            .await
            .map_err(|error| repository_error("model route repository", error))?
            .ok_or_else(|| {
                ChatModelError::new(
                    ChatModelErrorCode::RouteNotFound,
                    "the requested chat model route is not configured",
                    ChatRetryDirective::Never,
                )
            })?;

        routes
            .validate_for(selection)
            .map_err(|error| ChatModelError::invalid_request(error.to_string()))?;

        // Validate every candidate before returning the set.  Silently
        // dropping a broken failover would turn a persisted route set into a
        // different route plan and make outages dependent on read timing.
        for route in routes.candidates() {
            let provider = self
                .provider_repository
                .find_provider(&route.provider_id)
                .await
                .map_err(|error| repository_error("provider repository", error))?
                .ok_or_else(|| {
                    ChatModelError::new(
                        ChatModelErrorCode::RouteNotFound,
                        "a resolved chat route references a missing provider",
                        ChatRetryDirective::Never,
                    )
                    .with_route(route.model_route_id.clone())
                })?;

            if provider.provider_id != route.provider_id {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::CredentialTargetMismatch,
                    "provider repository returned a different provider identity",
                    ChatRetryDirective::Never,
                )
                .with_route(route.model_route_id.clone()));
            }
            if !provider.enabled {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::RouteNotFound,
                    "a resolved chat route references a disabled provider",
                    ChatRetryDirective::Never,
                )
                .with_route(route.model_route_id.clone()));
            }
            if provider.config_revision_digest != route.config_revision_digest {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::RouteRevisionMismatch,
                    "resolved chat route configuration is stale",
                    ChatRetryDirective::Never,
                )
                .with_route(route.model_route_id.clone()));
            }

            let connection = self
                .connection_repository
                .find_connection(route)
                .await
                .map_err(|error| repository_error("connection repository", error))?
                .ok_or_else(|| {
                    ChatModelError::new(
                        ChatModelErrorCode::CredentialReferenceMissing,
                        "a resolved chat route references a missing connection",
                        ChatRetryDirective::Never,
                    )
                    .with_route(route.model_route_id.clone())
                })?;

            if connection.provider_id != route.provider_id
                || connection.connection_config_ref != route.connection_config_ref
                || connection.credential_ref != route.credential_ref
            {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::CredentialTargetMismatch,
                    "resolved chat route and connection identity differ",
                    ChatRetryDirective::Never,
                )
                .with_route(route.model_route_id.clone()));
            }
        }

        Ok(routes)
    }
}

#[async_trait]
impl ChatRouteResolver for ProductionRouteResolver {
    async fn resolve(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<ResolvedChatRouteSet, ChatModelError> {
        self.resolve_repository_route(selection).await
    }
}

struct SecretKey([u8; 32]);

impl SecretKey {
    fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted encryption key]")
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// Credential authority for routes returned by [`ProductionRouteResolver`].
pub struct ProductionCredentialStore {
    connection_repository: Arc<dyn ProductionConnectionRepository>,
    encryption_key: SecretKey,
}

impl fmt::Debug for ProductionCredentialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductionCredentialStore")
            .field("connection_repository", &"[opaque]")
            .field("encryption_key", &self.encryption_key)
            .finish()
    }
}

impl ProductionCredentialStore {
    pub fn new(
        connection_repository: Arc<dyn ProductionConnectionRepository>,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            connection_repository,
            encryption_key: SecretKey::new(encryption_key),
        }
    }
}

#[async_trait]
impl ProviderCredentialStore for ProductionCredentialStore {
    async fn lease(
        &self,
        credential_ref: &ProviderCredentialRef,
        target: &CredentialTarget,
    ) -> Result<CredentialLease, ChatModelError> {
        let lease = self
            .connection_repository
            .lease_credential(credential_ref, target, self.encryption_key.as_bytes())
            .await
            .map_err(|error| repository_error("credential repository", error))?
            .ok_or_else(|| {
                ChatModelError::new(
                    ChatModelErrorCode::CredentialReferenceMissing,
                    "the requested provider credential is not available",
                    ChatRetryDirective::Never,
                )
            })?;

        if lease.credential_ref() != credential_ref
            || lease.target() != target
            || lease.opaque_handle().trim().is_empty()
        {
            return Err(ChatModelError::new(
                ChatModelErrorCode::CredentialTargetMismatch,
                "credential authority does not match the requested route target",
                ChatRetryDirective::Never,
            ));
        }
        Ok(lease)
    }
}

/// The only model-invoke surface needed by this broker.
///
/// An implementation must perform one provider attempt with the already
/// encoded request and opaque credential lease.  It must not resolve a second
/// route, rotate credentials, or retry.  The current public
/// `ModelInvokeService` does not implement this contract because its public
/// API is limited to non-chat `TaskRequest` values; an application adapter can
/// implement this port at its own composition boundary once it has a safe
/// internal chat executor.
#[async_trait]
pub trait ChatModelInvokePort: Send + Sync {
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError>;

    fn retry_count(&self) -> u8 {
        0
    }
}

/// A provider-neutral executor boundary for one already-encoded Chat attempt.
///
/// This alias is intentionally public so the application can install its
/// provider transport without making the broker depend on the legacy
/// `ModelInvokeService` or on a second routing layer.
pub trait ProviderWireExecutor: ChatModelInvokePort {}

impl<T> ProviderWireExecutor for T where T: ChatModelInvokePort + ?Sized {}

/// Compatibility alias for callers that name the bridge after the old
/// model-invoke service.
pub use ChatModelInvokePort as ModelInvokeChatPort;

/// A typed fail-closed adapter for hosts that have not installed a chat
/// executor yet.  It never fabricates a response.
pub struct UnavailableChatModelInvokePort;

#[async_trait]
impl ChatModelInvokePort for UnavailableChatModelInvokePort {
    async fn open_stream(
        &self,
        _request: ProviderWireRequest,
        _credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError> {
        Err(ChatModelError::new(
            ChatModelErrorCode::AdapterUnavailable,
            "the production chat model-invoke adapter is not configured",
            ChatRetryDirective::Never,
        ))
    }
}

/// One shared, single-attempt transport used by all six protocol adapters.
pub struct SixProtocolProviderTransport {
    model_invoke: Arc<dyn ChatModelInvokePort>,
}

impl SixProtocolProviderTransport {
    pub fn new(
        model_invoke: Arc<dyn ChatModelInvokePort>,
    ) -> Result<Self, ProductionBrokerError> {
        Self::try_new(model_invoke)
    }

    pub fn try_new(
        model_invoke: Arc<dyn ChatModelInvokePort>,
    ) -> Result<Self, ProductionBrokerError> {
        if model_invoke.retry_count() != 0 {
            return Err(ProductionBrokerError::RetryBoundary);
        }
        Ok(Self { model_invoke })
    }

    pub const fn retry_count(&self) -> u8 {
        0
    }

    fn validate_wire_request(
        request: &ProviderWireRequest,
        credential: &CredentialLease,
    ) -> Result<(), ChatModelError> {
        let target = credential.target();
        if request.protocol != target.protocol
            || request.provider_id != target.provider_id
            || request.route_identity.route_id != target.model_route_id
            || request.route_identity.route_revision != target.model_route_revision
            || request.connection_config_ref != target.connection_config_ref
            || request.config_revision_digest != target.config_revision_digest
            || request.credential_ref != *credential.credential_ref()
            || credential.opaque_handle().trim().is_empty()
        {
            return Err(ChatModelError::new(
                ChatModelErrorCode::CredentialTargetMismatch,
                "provider request and credential authority do not match",
                ChatRetryDirective::Never,
            ));
        }
        if contains_sensitive_wire_key(&request.body) {
            return Err(ChatModelError::new(
                ChatModelErrorCode::ProtocolViolation,
                "provider wire request contains a credential field",
                ChatRetryDirective::Never,
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ProviderTransport for SixProtocolProviderTransport {
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError> {
        Self::validate_wire_request(&request, &credential)?;
        let stream = self
            .model_invoke
            .open_stream(request, credential)
            .await
            .map_err(sanitize_model_error)?;
        Ok(Box::pin(stream.map(|frame| frame.map_err(sanitize_model_error))))
    }
}

fn contains_sensitive_wire_key(value: &Value) -> bool {
    const SENSITIVE_KEYS: &[&str] = &[
        "apikey",
        "authorization",
        "accesstoken",
        "refreshtoken",
        "clientsecret",
        "privatekey",
        "credential",
        "credentialmaterial",
        "secretaccesskey",
        "accesskeyid",
        "sessiontoken",
    ];

    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized = key
                .chars()
                .filter(|character| *character != '_' && *character != '-')
                .collect::<String>()
                .to_ascii_lowercase();
            SENSITIVE_KEYS
                .iter()
                .any(|sensitive| normalized == *sensitive)
                || contains_sensitive_wire_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_sensitive_wire_key),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn sanitize_model_error(mut error: ChatModelError) -> ChatModelError {
    error.message = match error.code {
        ChatModelErrorCode::AdapterUnavailable => {
            "the provider chat adapter is unavailable".to_owned()
        }
        ChatModelErrorCode::AuthenticationFailed => {
            "provider authentication failed".to_owned()
        }
        ChatModelErrorCode::RateLimited => "provider rate limit rejected the attempt".to_owned(),
        ChatModelErrorCode::PromptTooLong => "provider rejected the prompt length".to_owned(),
        ChatModelErrorCode::UnsupportedFeature => {
            "provider does not support the requested chat features".to_owned()
        }
        ChatModelErrorCode::Cancelled => "provider chat attempt was cancelled".to_owned(),
        ChatModelErrorCode::CredentialReferenceMissing => {
            "provider credential is unavailable".to_owned()
        }
        ChatModelErrorCode::CredentialTargetMismatch => {
            "provider credential target was rejected".to_owned()
        }
        ChatModelErrorCode::CausalityRejected
        | ChatModelErrorCode::DuplicateOperation
        | ChatModelErrorCode::ShadowNotPrimary
        | ChatModelErrorCode::SessionTerminal
        | ChatModelErrorCode::RouteNotFound
        | ChatModelErrorCode::RouteRevisionMismatch
        | ChatModelErrorCode::InvalidRequest
        | ChatModelErrorCode::ProtocolViolation
        | ChatModelErrorCode::ProviderUnavailable
        | ChatModelErrorCode::Internal => "provider chat attempt failed".to_owned(),
        ChatModelErrorCode::StreamInterrupted => {
            "provider chat stream was interrupted".to_owned()
        }
    };
    error
}

fn repository_error(
    context: &'static str,
    error: ProductionRepositoryError,
) -> ChatModelError {
    let message = match error {
        ProductionRepositoryError::Missing => {
            format!("{context} did not contain the required record")
        }
        ProductionRepositoryError::Unavailable => format!("{context} is unavailable"),
        ProductionRepositoryError::InvalidData => {
            format!("{context} returned invalid data")
        }
        ProductionRepositoryError::PermissionDenied => {
            format!("{context} denied the operation")
        }
    };
    ChatModelError::new(
        ChatModelErrorCode::Internal,
        message,
        ChatRetryDirective::Never,
    )
}

fn six_protocol_adapters(
    transport: Arc<dyn ProviderTransport>,
) -> Vec<Arc<dyn ChatProtocolAdapter>> {
    vec![
        Arc::new(AnthropicAdapter::new(transport.clone())),
        Arc::new(OpenAiChatAdapter::new(transport.clone())),
        Arc::new(OpenAiResponsesAdapter::new(transport.clone())),
        Arc::new(GeminiAdapter::new(transport.clone())),
        Arc::new(BedrockAdapter::new(transport.clone())),
        Arc::new(VertexAdapter::new(transport)),
    ]
}

fn assemble_production_broker(
    dependencies: ProductionBrokerDependencies,
) -> Result<Arc<ChatModelBroker>, ProductionBrokerError> {
    let ProductionBrokerDependencies {
        repositories,
        encryption_key,
        causality_gate,
        model_invoke,
        retry_policy,
    } = dependencies;

    let route_resolver = Arc::new(ProductionRouteResolver::new(repositories.clone()));
    let credential_store = Arc::new(ProductionCredentialStore::new(
        repositories.connection_repository,
        encryption_key,
    ));
    let transport = Arc::new(SixProtocolProviderTransport::try_new(model_invoke)?);
    let transport: Arc<dyn ProviderTransport> = transport;
    let broker = ChatModelBroker::new(
        causality_gate,
        route_resolver,
        credential_store,
        six_protocol_adapters(transport),
        retry_policy,
    )
    .map_err(ProductionBrokerError::Broker)?;
    Ok(Arc::new(broker))
}

/// Concrete production broker constructor.
pub struct ProductionChatModelBroker {
    inner: Arc<ChatModelBroker>,
}

impl ProductionChatModelBroker {
    pub fn new(
        dependencies: ProductionBrokerDependencies,
    ) -> Result<Arc<Self>, ProductionBrokerError> {
        Ok(Arc::new(Self {
            inner: assemble_production_broker(dependencies)?,
        }))
    }

    pub fn adapter_protocols(&self) -> BTreeSet<ChatProtocol> {
        self.inner.adapter_protocols()
    }

    pub fn inner(&self) -> &Arc<ChatModelBroker> {
        &self.inner
    }

    pub fn into_port(self: Arc<Self>) -> Arc<dyn ChatBrokerPort> {
        self
    }
}

#[async_trait]
impl ChatBrokerPort for ProductionChatModelBroker {
    async fn open_chat_stream(
        &self,
        request: ChatModelRequest,
    ) -> Result<ChatModelStream, ChatModelError> {
        self.inner.open_stream(request).await
    }
}

/// Build a production broker as the narrow port consumed by AgentPlatform.
pub fn build_production_chat_model_broker(
    dependencies: ProductionBrokerDependencies,
) -> Result<Arc<dyn ChatBrokerPort>, ProductionBrokerError> {
    Ok(ProductionChatModelBroker::new(dependencies)?.into_port())
}

/// Convenience constructor with the three repository roles explicit.
pub fn build_production_chat_model_broker_from_repositories(
    provider_repository: Arc<dyn ProductionProviderRepository>,
    connection_repository: Arc<dyn ProductionConnectionRepository>,
    model_repository: Arc<dyn ProductionModelRepository>,
    encryption_key: [u8; 32],
    causality_gate: Arc<dyn ChatCausalityGate>,
    model_invoke: Arc<dyn ChatModelInvokePort>,
    retry_policy: BrokerRetryPolicy,
) -> Result<Arc<dyn ChatBrokerPort>, ProductionBrokerError> {
    build_production_chat_model_broker(
        ProductionBrokerDependencies::new(
            provider_repository,
            connection_repository,
            model_repository,
            encryption_key,
            causality_gate,
            model_invoke,
        )
        .with_retry_policy(retry_policy),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use futures::stream;
    use serde_json::json;

    use super::*;
    use crate::adapter::{ProviderTransport, ProviderWireFrame};
    use crate::contracts::ChatCausality;
    use crate::recorded::recorded_conformance_fixtures;

    struct AllowGate {
        calls: AtomicUsize,
        error: Option<ChatModelError>,
    }

    impl AllowGate {
        fn allow() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                error: None,
            })
        }

        fn reject(error: ChatModelError) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                error: Some(error),
            })
        }
    }

    #[async_trait]
    impl ChatCausalityGate for AllowGate {
        async fn authorize(&self, _causality: &ChatCausality) -> Result<(), ChatModelError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.error.clone().map_or(Ok(()), Err)
        }
    }

    struct StaticProviderRepository {
        record: ProviderRepositoryRecord,
    }

    #[async_trait]
    impl ProductionProviderRepository for StaticProviderRepository {
        async fn find_provider(
            &self,
            provider_id: &ProviderIdRef,
        ) -> Result<Option<ProviderRepositoryRecord>, ProductionRepositoryError> {
            Ok((provider_id == &self.record.provider_id).then(|| self.record.clone()))
        }
    }

    struct StaticModelRepository {
        routes: Option<ResolvedChatRouteSet>,
    }

    #[async_trait]
    impl ProductionModelRepository for StaticModelRepository {
        async fn resolve_chat_route(
            &self,
            _selection: &ChatRouteSelection,
        ) -> Result<Option<ResolvedChatRouteSet>, ProductionRepositoryError> {
            Ok(self.routes.clone())
        }
    }

    struct StaticConnectionRepository {
        record: ConnectionRepositoryRecord,
        mismatch_lease: bool,
    }

    #[async_trait]
    impl ProductionConnectionRepository for StaticConnectionRepository {
        async fn find_connection(
            &self,
            route: &ResolvedChatRoute,
        ) -> Result<Option<ConnectionRepositoryRecord>, ProductionRepositoryError> {
            Ok((route.provider_id == self.record.provider_id
                && route.connection_config_ref == self.record.connection_config_ref
                && route.credential_ref == self.record.credential_ref)
                .then(|| self.record.clone()))
        }

        async fn lease_credential(
            &self,
            credential_ref: &ProviderCredentialRef,
            target: &CredentialTarget,
            _encryption_key: &[u8; 32],
        ) -> Result<Option<CredentialLease>, ProductionRepositoryError> {
            let target = if self.mismatch_lease {
                CredentialTarget {
                    provider_id: ProviderIdRef::from("different-provider"),
                    ..target.clone()
                }
            } else {
                target.clone()
            };
            Ok(Some(CredentialLease::new(
                credential_ref.clone(),
                target,
                "opaque-test-handle",
            )))
        }
    }

    enum InvokeScript {
        Error(ChatModelError),
        Frames(Vec<ProviderWireFrame>),
    }

    struct ScriptedModelInvoke {
        calls: AtomicUsize,
        scripts: Mutex<VecDeque<InvokeScript>>,
        requests: Mutex<Vec<ProviderWireRequest>>,
    }

    impl ScriptedModelInvoke {
        fn new(scripts: impl IntoIterator<Item = InvokeScript>) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                scripts: Mutex::new(scripts.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }
    }

    #[async_trait]
    impl ChatModelInvokePort for ScriptedModelInvoke {
        async fn open_stream(
            &self,
            request: ProviderWireRequest,
            credential: CredentialLease,
        ) -> Result<ProviderWireStream, ChatModelError> {
            assert_eq!(credential.credential_ref(), &request.credential_ref);
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.requests
                .lock()
                .expect("request lock")
                .push(request);
            match self
                .scripts
                .lock()
                .expect("script lock")
                .pop_front()
                .unwrap_or_else(|| {
                    InvokeScript::Error(ChatModelError::provider_unavailable(
                        "script exhausted",
                    ))
                }) {
                InvokeScript::Error(error) => Err(error),
                InvokeScript::Frames(frames) => Ok(Box::pin(stream::iter(
                    frames.into_iter().map(Ok),
                ))),
            }
        }
    }

    fn fixture() -> crate::recorded::RecordedConformanceFixture {
        recorded_conformance_fixtures()
            .into_iter()
            .next()
            .expect("recorded fixture")
    }

    fn repository_set(
        route: &ResolvedChatRoute,
        routes: Option<ResolvedChatRouteSet>,
        mismatch_lease: bool,
    ) -> ProductionRepositorySet {
        ProductionRepositorySet::new(
            Arc::new(StaticProviderRepository {
                record: ProviderRepositoryRecord {
                    provider_id: route.provider_id.clone(),
                    enabled: true,
                    config_revision_digest: route.config_revision_digest.clone(),
                },
            }),
            Arc::new(StaticConnectionRepository {
                record: ConnectionRepositoryRecord {
                    provider_id: route.provider_id.clone(),
                    connection_config_ref: route.connection_config_ref.clone(),
                    credential_ref: route.credential_ref.clone(),
                },
                mismatch_lease,
            }),
            Arc::new(StaticModelRepository { routes }),
        )
    }

    fn dependencies(
        fixture: &crate::recorded::RecordedConformanceFixture,
        invoke: Arc<dyn ChatModelInvokePort>,
        mismatch_lease: bool,
    ) -> ProductionBrokerDependencies {
        let route = &fixture.route;
        let repositories = repository_set(
            route,
            Some(ResolvedChatRouteSet {
                primary: route.clone(),
                failovers: Vec::new(),
            }),
            mismatch_lease,
        );
        ProductionBrokerDependencies {
            repositories,
            encryption_key: [0x5a; 32],
            causality_gate: AllowGate::allow(),
            model_invoke: invoke,
            retry_policy: BrokerRetryPolicy {
                max_total_attempts: 1,
                max_attempts_per_route: 1,
            },
        }
    }

    #[tokio::test]
    async fn production_factory_composes_exact_six_adapters_and_streams() {
        let fixture = fixture();
        let invoke = ScriptedModelInvoke::new([InvokeScript::Frames(
            fixture.wire_events.clone(),
        )]);
        let broker = ProductionChatModelBroker::new(dependencies(
            &fixture,
            invoke.clone(),
            false,
        ))
        .expect("production broker");

        assert_eq!(
            broker.adapter_protocols(),
            ChatProtocol::ALL.into_iter().collect::<BTreeSet<_>>()
        );
        let events = broker
            .open_chat_stream(fixture.request)
            .await
            .expect("open stream")
            .collect::<Vec<_>>()
            .await;
        assert!(events.iter().all(Result::is_ok));
        assert!(events.last().is_some_and(|item| {
            item.as_ref()
                .is_ok_and(|event| event.event.is_terminal())
        }));
        assert_eq!(invoke.calls(), 1);
    }

    #[tokio::test]
    async fn missing_route_fails_closed_before_model_invoke() {
        let fixture = fixture();
        let invoke = ScriptedModelInvoke::new([]);
        let mut dependencies = dependencies(&fixture, invoke.clone(), false);
        dependencies.repositories = repository_set(&fixture.route, None, false);
        let gate = AllowGate::allow();
        dependencies.causality_gate = gate.clone();
        let broker = ProductionChatModelBroker::new(dependencies).expect("production broker");

        let error = match broker.open_chat_stream(fixture.request).await {
            Ok(_) => panic!("missing route"),
            Err(error) => error,
        };
        assert_eq!(error.code, ChatModelErrorCode::RouteNotFound);
        assert_eq!(invoke.calls(), 0);
        // Route resolution deliberately precedes causality admission. A
        // missing immutable route must not consume the operation claim.
        assert_eq!(gate.calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn causality_rejection_stops_before_route_or_model_invoke() {
        let fixture = fixture();
        let invoke = ScriptedModelInvoke::new([]);
        let mut dependencies = dependencies(&fixture, invoke.clone(), false);
        dependencies.causality_gate = AllowGate::reject(ChatModelError::new(
            ChatModelErrorCode::ShadowNotPrimary,
            "shadow request rejected",
            ChatRetryDirective::Never,
        ));

        let error = match ProductionChatModelBroker::new(dependencies)
            .expect("production broker")
            .open_chat_stream(fixture.request)
            .await
        {
            Ok(_) => panic!("causality rejection"),
            Err(error) => error,
        };
        assert_eq!(error.code, ChatModelErrorCode::ShadowNotPrimary);
        assert_eq!(invoke.calls(), 0);
    }

    #[tokio::test]
    async fn mismatched_credential_lease_fails_closed_before_model_invoke() {
        let fixture = fixture();
        let invoke = ScriptedModelInvoke::new([]);
        let broker = ProductionChatModelBroker::new(dependencies(
            &fixture,
            invoke.clone(),
            true,
        ))
        .expect("production broker");

        let events = broker
            .open_chat_stream(fixture.request)
            .await
            .expect("stream is created before per-attempt lease")
            .collect::<Vec<_>>()
            .await;
        assert_eq!(
            events
                .last()
                .expect("terminal broker error")
                .as_ref()
                .expect_err("mismatched lease")
                .code,
            ChatModelErrorCode::CredentialTargetMismatch
        );
        assert_eq!(invoke.calls(), 0);
    }

    #[tokio::test]
    async fn retry_is_owned_by_the_broker_and_adapter_errors_are_redacted() {
        let fixture = fixture();
        let invoke = ScriptedModelInvoke::new([
            InvokeScript::Error(ChatModelError::new(
                ChatModelErrorCode::RateLimited,
                "provider leaked super-secret-value",
                ChatRetryDirective::RetrySameRoute,
            )),
            InvokeScript::Frames(fixture.wire_events.clone()),
        ]);
        let mut dependencies = dependencies(&fixture, invoke.clone(), false);
        dependencies.retry_policy = BrokerRetryPolicy {
            max_total_attempts: 2,
            max_attempts_per_route: 2,
        };
        let broker = ProductionChatModelBroker::new(dependencies).expect("production broker");
        let events = broker
            .open_chat_stream(fixture.request)
            .await
            .expect("stream")
            .collect::<Vec<_>>()
            .await;

        assert!(events.iter().all(Result::is_ok));
        assert_eq!(invoke.calls(), 2);
        assert!(events.iter().filter_map(|item| item.as_ref().ok()).all(|event| {
            event.total_attempt == 2 && event.route_attempt == 2
        }));
        assert!(
            !events
                .iter()
                .filter_map(|item| item.as_ref().err())
                .any(|error| error.message.contains("super-secret-value"))
        );
    }

    #[tokio::test]
    async fn transport_rejects_wire_credential_fields_without_calling_executor() {
        let fixture = fixture();
        let invoke = ScriptedModelInvoke::new([]);
        let transport =
            SixProtocolProviderTransport::new(invoke.clone()).expect("single-attempt transport");
        let route = &fixture.route;
        let credential = CredentialLease::new(
            route.credential_ref.clone(),
            CredentialTarget::for_route(route),
            "opaque-test-handle",
        );
        let request = ProviderWireRequest {
            protocol: route.protocol,
            provider_id: route.provider_id.clone(),
            model: route.model.clone(),
            route_identity: fixture.request.route.clone(),
            connection_config_ref: route.connection_config_ref.clone(),
            config_revision_digest: route.config_revision_digest.clone(),
            credential_ref: route.credential_ref.clone(),
            body: json!({"api_key": "super-secret-value"}),
        };

        let error = match transport.open_stream(request, credential).await {
            Ok(_) => panic!("wire credential field"),
            Err(error) => error,
        };
        assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
        assert_eq!(invoke.calls(), 0);
    }

    #[test]
    fn transport_credential_filter_rejects_provider_specific_secret_names() {
        for key in [
            "secret_access_key",
            "SecretAccessKey",
            "session-token",
            "access_key_id",
            "client-secret",
            "private_key",
        ] {
            assert!(
                contains_sensitive_wire_key(&json!({key: "must-not-wire"})),
                "credential key {key:?} was not rejected"
            );
        }
    }

    struct RetryingModelInvoke;

    #[async_trait]
    impl ChatModelInvokePort for RetryingModelInvoke {
        async fn open_stream(
            &self,
            _request: ProviderWireRequest,
            _credential: CredentialLease,
        ) -> Result<ProviderWireStream, ChatModelError> {
            unreachable!("construction must reject autonomous retry")
        }

        fn retry_count(&self) -> u8 {
            1
        }
    }

    #[test]
    fn autonomous_model_invoke_retry_is_rejected_at_construction() {
        let error = match SixProtocolProviderTransport::new(Arc::new(RetryingModelInvoke)) {
            Ok(_) => panic!("retrying adapter must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, ProductionBrokerError::RetryBoundary);
    }

    #[tokio::test]
    async fn unavailable_model_invoke_port_is_typed_and_not_a_fake_response() {
        let fixture = fixture();
        let invoke = Arc::new(UnavailableChatModelInvokePort);
        let broker = ProductionChatModelBroker::new(dependencies(
            &fixture,
            invoke,
            false,
        ))
        .expect("production broker");
        let events = broker
            .open_chat_stream(fixture.request)
            .await
            .expect("stream")
            .collect::<Vec<_>>()
            .await;
        let error = events
            .last()
            .expect("unavailable adapter error")
            .as_ref()
            .expect_err("must fail closed");
        assert_eq!(error.code, ChatModelErrorCode::AdapterUnavailable);
        assert!(error.message.contains("adapter"));
    }

    #[test]
    fn credential_store_debug_does_not_include_key_bytes() {
        let fixture = fixture();
        let route = fixture.route.clone();
        let repositories = repository_set(
            &route,
            Some(ResolvedChatRouteSet {
                primary: route.clone(),
                failovers: Vec::new(),
            }),
            false,
        );
        let store = ProductionCredentialStore::new(
            repositories.connection_repository,
            [0xab; 32],
        );
        let debug = format!("{store:?}");
        assert!(!debug.contains("ab"));
        assert!(debug.contains("redacted"));
    }

}
