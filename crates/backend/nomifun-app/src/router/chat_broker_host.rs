//! App-side composition for the production v4 chat broker.
//!
//! The v4 database owns the route record. Provider/model/connection details
//! are read from the existing provider database through explicit cross-database
//! inputs. This module never derives a `ModelRouteId`, a credential reference,
//! or a provider revision digest from another value.

#![forbid(unsafe_code)]
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use nomifun_agent_contracts::{
    AgentPresetRevisionPayload, ChatRouteCandidate, ChatRouteFeature, ChatRouteProtocol,
    ChatRouteRecord as CanonicalChatRouteRecord, ConnectionConfigRef, DigestHex, digest_payload,
    validate_chat_route_records,
};
use nomifun_chat_model_broker::{
    BrokerRetryPolicy, ChatBrokerPort, ChatCapabilityObserver, ChatCausalityGate, ChatModelError,
    ChatModelErrorCode, ChatModelFeature, ChatModelInvokePort, ChatProtocol,
    ChatRetryDirective, ChatRouteSelection,
    CredentialLease, CredentialTarget, ProductionBrokerDependencies, ProductionBrokerError,
    ProductionConnectionRepository as ProductionConnectionRepositoryPort,
    ProductionModelRepository as ProductionModelRepositoryPort,
    ProductionProviderRepository as ProductionProviderRepositoryPort,
    ProductionRepositoryError, ProviderCredentialRef, ProviderIdRef,
    ProviderRepositoryRecord, ConnectionRepositoryRecord, ResolvedChatRoute,
    ResolvedChatRouteSet, ProviderWireFrame, ProviderWireRequest, ProviderWireStream,
    build_production_chat_model_broker,
};
use nomifun_db::{
    IProviderModelCapabilityRepository, SqlitePool, SqliteProviderModelCapabilityRepository,
};
use nomifun_db::sqlx::{self, Row};
use nomifun_api_types::{CapabilityHealth, ModelTask, ModelTechnicalCapability};
use nomifun_model_invoke::{
    AuthMaterial, AuthScheme, OpaqueCredentialLease, OpaqueCredentialResolver,
    InvokeError, InvokeErrorKind, SingleAttemptFraming, SingleAttemptHttpExecutor,
    SingleAttemptRequest, expand_protocol_endpoint_template, join_endpoint,
    protocol_descriptor, protocol_task_descriptor, validate_credentialed_target_url,
    validate_provider_params_for_protocol,
};
use serde::Serialize;
use serde_json::Value;
use std::fmt;

const PROVIDER_CHAT_MODEL_TASK: &str = "chat";

fn decode_agent_preset_revision_payload(
    json: &str,
) -> Result<AgentPresetRevisionPayload, ChatBrokerHostError> {
    let payload: AgentPresetRevisionPayload = serde_json::from_str(json).map_err(|error| {
        ChatBrokerHostError::InvalidRouteRecord(format!(
            "agent preset revision payload is invalid: {error}"
        ))
    })?;
    validate_chat_route_records(&payload).map_err(|error| {
        ChatBrokerHostError::InvalidRouteRecord(format!(
            "{}: {}",
            error.code.as_ref(),
            error.message
        ))
    })?;
    Ok(payload)
}

fn resolve_chat_route_record(
    record: &CanonicalChatRouteRecord,
    selection: &ChatRouteSelection,
) -> Result<ResolvedChatRouteSet, ChatBrokerHostError> {
    record
        .validate_for(selection)
        .map_err(|error| ChatBrokerHostError::InvalidRouteRecord(error.to_string()))?;
    let primary = convert_chat_route_candidate(&record.primary)?;
    let failovers = record
        .failovers
        .iter()
        .map(convert_chat_route_candidate)
        .collect::<Result<Vec<_>, _>>()?;
    let routes = ResolvedChatRouteSet { primary, failovers };
    routes
        .validate_for(selection)
        .map_err(|error| ChatBrokerHostError::InvalidRouteRecord(error.to_string()))?;
    Ok(routes)
}

fn convert_chat_route_candidate(
    candidate: &ChatRouteCandidate,
) -> Result<ResolvedChatRoute, ChatBrokerHostError> {
    let protocol = match candidate.protocol {
        ChatRouteProtocol::Anthropic => ChatProtocol::Anthropic,
        ChatRouteProtocol::OpenaiChat => ChatProtocol::OpenaiChat,
        ChatRouteProtocol::OpenaiResponses => ChatProtocol::OpenaiResponses,
        ChatRouteProtocol::Gemini => ChatProtocol::Gemini,
        ChatRouteProtocol::Bedrock => ChatProtocol::Bedrock,
        ChatRouteProtocol::Vertex => ChatProtocol::Vertex,
    };
    let features = candidate.features.iter().map(convert_route_feature).collect();
    let activation_features = candidate
        .activation_features
        .iter()
        .map(convert_route_feature)
        .collect();
    let route = ResolvedChatRoute {
        model_route_id: candidate.model_route_id.clone(),
        model_route_revision: candidate.model_route_revision,
        provider_id: ProviderIdRef::from(candidate.provider_id.clone()),
        model: candidate.model.clone(),
        protocol,
        connection_config_ref: candidate.connection_config_ref.clone(),
        config_revision_digest: candidate.config_revision_digest.clone(),
        credential_ref: ProviderCredentialRef::from(candidate.credential_ref.clone()),
        features,
        activation_features,
    };
    route
        .validate()
        .map_err(|error| ChatBrokerHostError::InvalidRouteRecord(error.to_string()))?;
    Ok(route)
}

fn convert_route_feature(feature: &ChatRouteFeature) -> ChatModelFeature {
    match feature {
        ChatRouteFeature::TextInput => ChatModelFeature::TextInput,
        ChatRouteFeature::ImageInput => ChatModelFeature::ImageInput,
        ChatRouteFeature::AudioInput => ChatModelFeature::AudioInput,
        ChatRouteFeature::TextOutput => ChatModelFeature::TextOutput,
        ChatRouteFeature::AudioOutput => ChatModelFeature::AudioOutput,
        ChatRouteFeature::ToolCalls => ChatModelFeature::ToolCalls,
        ChatRouteFeature::Reasoning => ChatModelFeature::Reasoning,
        ChatRouteFeature::ReasoningSignature => ChatModelFeature::ReasoningSignature,
        ChatRouteFeature::PromptCache => ChatModelFeature::PromptCache,
        ChatRouteFeature::StructuredOutput => ChatModelFeature::StructuredOutput,
        ChatRouteFeature::ProviderRoundState => ChatModelFeature::ProviderRoundState,
        ChatRouteFeature::NativeResponsesItems => ChatModelFeature::NativeResponsesItems,
        ChatRouteFeature::Streaming => ChatModelFeature::Streaming,
        ChatRouteFeature::WebSearch => ChatModelFeature::WebSearch,
    }
}

/// Detailed, safe errors returned by the app-side route and lease boundary.
#[derive(Debug, PartialEq, Eq)]
pub enum ChatBrokerHostError {
    InvalidRouteRecord(String),
    RouteRecordMissing {
        task: String,
        route_id: String,
        route_revision: u64,
    },
    RouteDatabaseUnavailable,
    ProviderDatabaseUnavailable,
    ModelBindingMissing,
    ModelCapabilityMismatch,
    ConnectionMissing,
    DefaultConnectionUnsupported,
    CredentialRegistry,
}

impl fmt::Display for ChatBrokerHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRouteRecord(error) => {
                write!(formatter, "v4 chat route record is invalid: {error}")
            }
            Self::RouteRecordMissing {
                task,
                route_id,
                route_revision,
            } => write!(
                formatter,
                "v4 chat route record is missing for task {task:?}, route {route_id:?}@{route_revision}"
            ),
            Self::RouteDatabaseUnavailable => {
                formatter.write_str("the v4 route-record storage is unavailable")
            }
            Self::ProviderDatabaseUnavailable => {
                formatter.write_str("the provider database is unavailable")
            }
            Self::ModelBindingMissing => {
                formatter.write_str("provider model binding is missing or disabled")
            }
            Self::ModelCapabilityMismatch => {
                formatter.write_str("provider model capability does not match the route protocol")
            }
            Self::ConnectionMissing => formatter.write_str("named provider connection is missing"),
            Self::DefaultConnectionUnsupported => formatter.write_str(
                "default provider connections are not representable by the v4 route record",
            ),
            Self::CredentialRegistry => {
                formatter.write_str("credential lease registry rejected the credential")
            }
        }
    }
}

impl std::error::Error for ChatBrokerHostError {}

impl From<ChatBrokerHostError> for ProductionRepositoryError {
    fn from(error: ChatBrokerHostError) -> Self {
        match error {
            ChatBrokerHostError::RouteDatabaseUnavailable
            | ChatBrokerHostError::ProviderDatabaseUnavailable => {
                ProductionRepositoryError::Unavailable
            }
            ChatBrokerHostError::InvalidRouteRecord(_)
            | ChatBrokerHostError::RouteRecordMissing { .. }
            | ChatBrokerHostError::ModelBindingMissing
            | ChatBrokerHostError::ModelCapabilityMismatch
            | ChatBrokerHostError::ConnectionMissing
            | ChatBrokerHostError::DefaultConnectionUnsupported
            | ChatBrokerHostError::CredentialRegistry => ProductionRepositoryError::InvalidData,
        }
    }
}

/// DB-backed provider repository. It reads only provider identity, enabled
/// state, and the content digest of the complete invocation graph.
#[derive(Clone)]
pub struct ProductionProviderRepository {
    pool: SqlitePool,
}

impl ProductionProviderRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProductionProviderRepositoryPort for ProductionProviderRepository {
    async fn find_provider(
        &self,
        provider_id: &ProviderIdRef,
    ) -> Result<Option<ProviderRepositoryRecord>, ProductionRepositoryError> {
        let row = sqlx::query(
            "SELECT provider_id, enabled \
             FROM providers WHERE provider_id = ?",
        )
        .bind(provider_id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ProductionRepositoryError::Unavailable)?;
        let Some(row) = row else {
            return Ok(None);
        };

        let stored_provider_id: String = row
            .try_get("provider_id")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        if stored_provider_id != provider_id.as_ref() {
            return Err(ProductionRepositoryError::InvalidData);
        }
        let enabled: i64 = row
            .try_get("enabled")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let computed_digest = provider_config_digest(&self.pool, provider_id).await?;
        Ok(Some(ProviderRepositoryRecord {
            provider_id: provider_id.clone(),
            enabled: enabled != 0,
            config_revision_digest: computed_digest,
        }))
    }
}

/// Compute the invocation-graph digest for one provider revision.
///
/// The legacy catalog stores a monotonic integer `config_revision`, while the
/// v4 broker contract carries a content digest.  The integer is only a CAS
/// fence; it is not itself a digest and must never be encoded or padded into
/// one.  This explicit, versioned input contains every persisted field that
/// can change the Chat wire request or its credential target, while excluding
/// health observations and display-only timestamps.
pub(crate) async fn provider_config_digest(
    pool: &SqlitePool,
    provider_id: &ProviderIdRef,
) -> Result<DigestHex, ProductionRepositoryError> {
    let provider = sqlx::query_as::<_, nomifun_db::models::Provider>(
        "SELECT * FROM providers WHERE provider_id = ?",
    )
    .bind(provider_id.as_ref())
    .fetch_optional(pool)
    .await
    .map_err(|_| ProductionRepositoryError::Unavailable)?
    .ok_or(ProductionRepositoryError::Missing)?;

    let mut models = sqlx::query_as::<_, nomifun_db::models::ProviderModelRow>(
        "SELECT * FROM provider_models WHERE provider_id = ? ORDER BY model ASC",
    )
    .bind(provider_id.as_ref())
    .fetch_all(pool)
    .await
    .map_err(|_| ProductionRepositoryError::Unavailable)?;

    let mut capabilities =
        sqlx::query_as::<_, nomifun_db::models::ProviderModelCapabilityRow>(
            "SELECT * FROM provider_model_capabilities \
             WHERE provider_id = ? ORDER BY model ASC, task ASC",
        )
        .bind(provider_id.as_ref())
        .fetch_all(pool)
        .await
        .map_err(|_| ProductionRepositoryError::Unavailable)?;

    let mut connections = sqlx::query_as::<_, nomifun_db::models::ProviderConnectionRow>(
        "SELECT * FROM provider_connections WHERE provider_id = ? ORDER BY role ASC",
    )
    .bind(provider_id.as_ref())
    .fetch_all(pool)
    .await
    .map_err(|_| ProductionRepositoryError::Unavailable)?;

    models.sort_by(|left, right| left.model.cmp(&right.model));
    capabilities.sort_by(|left, right| {
        left.model
            .cmp(&right.model)
            .then(left.task.cmp(&right.task))
    });
    connections.sort_by(|left, right| {
        left.role
            .cmp(&right.role)
            .then(left.connection_id.cmp(&right.connection_id))
    });

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct DigestInput {
        schema: &'static str,
        provider: ProviderDigestProvider,
        models: Vec<ProviderDigestModel>,
        capabilities: Vec<ProviderDigestCapability>,
        connections: Vec<ProviderDigestConnection>,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct ProviderDigestProvider {
        provider_id: String,
        platform: String,
        base_url: String,
        auth_scheme: String,
        credentials_encrypted: String,
        enabled: bool,
        bedrock_config: Option<String>,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct ProviderDigestModel {
        provider_id: String,
        model: String,
        enabled: bool,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct ProviderDigestCapability {
        provider_id: String,
        model: String,
        task: String,
        traits: String,
        protocol: String,
        connection_role: String,
        base_url_override: Option<String>,
        endpoint: Option<String>,
        poll_endpoint: Option<String>,
        content_endpoint: Option<String>,
        realtime_endpoint: Option<String>,
        allow_cross_origin_credentials: bool,
        provider_params: String,
        context_limit: Option<i64>,
        output_limit: Option<i64>,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct ProviderDigestConnection {
        provider_id: String,
        connection_id: String,
        role: String,
        base_url: String,
        auth_scheme: String,
        credentials_encrypted: String,
        extra: String,
    }

    let input = DigestInput {
        schema: "nomifun.provider-invocation-graph.v1",
        provider: ProviderDigestProvider {
            provider_id: provider.provider_id,
            platform: provider.platform,
            base_url: provider.base_url,
            auth_scheme: provider.auth_scheme,
            credentials_encrypted: provider.credentials_encrypted,
            enabled: provider.enabled,
            bedrock_config: provider.bedrock_config,
        },
        models: models
            .into_iter()
            .map(|model| ProviderDigestModel {
                provider_id: model.provider_id,
                model: model.model,
                enabled: model.enabled,
            })
            .collect(),
        capabilities: capabilities
            .into_iter()
            .map(|capability| ProviderDigestCapability {
                provider_id: capability.provider_id,
                model: capability.model,
                task: capability.task,
                traits: capability.traits,
                protocol: capability.protocol,
                connection_role: capability.connection_role,
                base_url_override: capability.base_url_override,
                endpoint: capability.endpoint,
                poll_endpoint: capability.poll_endpoint,
                content_endpoint: capability.content_endpoint,
                realtime_endpoint: capability.realtime_endpoint,
                allow_cross_origin_credentials: capability.allow_cross_origin_credentials,
                provider_params: capability.provider_params,
                context_limit: capability.context_limit,
                output_limit: capability.output_limit,
            })
            .collect(),
        connections: connections
            .into_iter()
            .map(|connection| ProviderDigestConnection {
                provider_id: connection.provider_id,
                connection_id: connection.connection_id,
                role: connection.role,
                base_url: connection.base_url,
                auth_scheme: connection.auth_scheme,
                credentials_encrypted: connection.credentials_encrypted,
                extra: connection.extra,
            })
            .collect(),
    };

    digest_payload(&input)
        .map_err(|_| ProductionRepositoryError::InvalidData)
}

#[derive(Clone, Debug)]
struct CachedTechnicalCapabilities {
    config_revision: i64,
    unsupported: BTreeSet<ModelTechnicalCapability>,
}

/// Process-local projection of the durable negative-observation set. A
/// provider configuration revision change invalidates an entry automatically;
/// conclusive failures update DB first and this cache second.
#[derive(Clone, Default)]
struct ModelTechnicalCapabilityCache {
    entries: Arc<RwLock<BTreeMap<(String, String), CachedTechnicalCapabilities>>>,
}

impl ModelTechnicalCapabilityCache {
    async fn unsupported_for(
        &self,
        pool: &SqlitePool,
        provider_id: &str,
        model: &str,
    ) -> Result<(i64, BTreeSet<ModelTechnicalCapability>), ProductionRepositoryError> {
        let row = sqlx::query(
            "SELECT provider.config_revision, capability.health \
             FROM providers provider \
             JOIN provider_model_capabilities capability \
               ON capability.provider_id = provider.provider_id \
             WHERE capability.provider_id = ? AND capability.model = ? \
               AND capability.task = 'chat'",
        )
        .bind(provider_id)
        .bind(model)
        .fetch_optional(pool)
        .await
        .map_err(|_| ProductionRepositoryError::Unavailable)?
        .ok_or(ProductionRepositoryError::Missing)?;
        let config_revision: i64 = row
            .try_get("config_revision")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let key = (provider_id.to_owned(), model.to_owned());
        if let Some(cached) = self
            .entries
            .read()
            .map_err(|_| ProductionRepositoryError::Unavailable)?
            .get(&key)
            .filter(|cached| cached.config_revision == config_revision)
            .cloned()
        {
            return Ok((config_revision, cached.unsupported));
        }
        let health: Option<String> = row
            .try_get("health")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let unsupported = health
            .as_deref()
            .map(serde_json::from_str::<CapabilityHealth>)
            .transpose()
            .map_err(|_| ProductionRepositoryError::InvalidData)?
            .map(|health| {
                health
                    .unsupported_technical_capabilities
                    .into_iter()
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        self.entries
            .write()
            .map_err(|_| ProductionRepositoryError::Unavailable)?
            .insert(
                key,
                CachedTechnicalCapabilities {
                    config_revision,
                    unsupported: unsupported.clone(),
                },
            );
        Ok((config_revision, unsupported))
    }

    fn remember_unsupported(
        &self,
        provider_id: &str,
        model: &str,
        config_revision: i64,
        capability: ModelTechnicalCapability,
    ) -> Result<(), ProductionRepositoryError> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| ProductionRepositoryError::Unavailable)?;
        let entry = entries
            .entry((provider_id.to_owned(), model.to_owned()))
            .or_insert_with(|| CachedTechnicalCapabilities {
                config_revision,
                unsupported: BTreeSet::new(),
            });
        if entry.config_revision != config_revision {
            *entry = CachedTechnicalCapabilities {
                config_revision,
                unsupported: BTreeSet::new(),
            };
        }
        entry.unsupported.insert(capability);
        Ok(())
    }
}

/// DB-backed v4 route repository plus provider-model identity checks.
#[derive(Clone)]
pub struct ProductionModelRepository {
    pool: SqlitePool,
    technical_capabilities: ModelTechnicalCapabilityCache,
}

impl ProductionModelRepository {
    #[cfg(test)]
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            technical_capabilities: ModelTechnicalCapabilityCache::default(),
        }
    }

    fn with_technical_capabilities(
        pool: SqlitePool,
        technical_capabilities: ModelTechnicalCapabilityCache,
    ) -> Self {
        Self {
            pool,
            technical_capabilities,
        }
    }

    async fn apply_technical_capabilities(
        &self,
        mut routes: ResolvedChatRouteSet,
    ) -> Result<ResolvedChatRouteSet, ProductionRepositoryError> {
        for route in std::iter::once(&mut routes.primary).chain(routes.failovers.iter_mut()) {
            let (_, unsupported) = self
                .technical_capabilities
                .unsupported_for(&self.pool, route.provider_id.as_ref(), &route.model)
                .await?;
            for feature in [
                ChatModelFeature::ToolCalls,
                ChatModelFeature::Reasoning,
                ChatModelFeature::Streaming,
            ] {
                route.features.insert(feature);
            }
            if route.protocol == ChatProtocol::OpenaiResponses {
                route.features.extend([
                    ChatModelFeature::AudioOutput,
                    ChatModelFeature::ProviderRoundState,
                ]);
            }
            for capability in unsupported {
                route.features.remove(&match capability {
                    ModelTechnicalCapability::FunctionCalling => ChatModelFeature::ToolCalls,
                    ModelTechnicalCapability::Reasoning => ChatModelFeature::Reasoning,
                    ModelTechnicalCapability::Streaming => ChatModelFeature::Streaming,
                });
            }
        }
        Ok(routes)
    }

    async fn load_route_record(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<Option<ResolvedChatRouteSet>, ChatBrokerHostError> {
        selection
            .validate()
            .map_err(|error| ChatBrokerHostError::InvalidRouteRecord(error.to_string()))?;
        let Some(record) = self.load_route_record_for_revision(selection).await? else {
            return Ok(None);
        };
        let routes = resolve_chat_route_record(&record, selection)?;
        Ok(Some(routes))
    }

    async fn load_route_record_for_revision(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<Option<CanonicalChatRouteRecord>, ChatBrokerHostError> {
        let payload_json: Option<String> = sqlx::query_scalar(
            "SELECT payload_json FROM agent_preset_revisions WHERE revision_id = ?",
        )
        .bind(&selection.preset_revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ChatBrokerHostError::RouteDatabaseUnavailable)?;
        let Some(payload_json) = payload_json else {
            return Ok(None);
        };
        let payload = decode_agent_preset_revision_payload(&payload_json)?;
        Ok(payload.chat_route_records.get(&selection.model_task).cloned())
    }

    #[cfg(test)]
    pub async fn resolve_route_record(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<ResolvedChatRouteSet, ChatBrokerHostError> {
        self.load_route_record(selection)
            .await?
            .ok_or_else(|| ChatBrokerHostError::RouteRecordMissing {
                task: selection.model_task.clone(),
                route_id: selection.route_id.as_ref().to_owned(),
                route_revision: selection.route_revision,
            })
    }

    async fn validate_model_bindings(
        &self,
        routes: &ResolvedChatRouteSet,
    ) -> Result<(), ChatBrokerHostError> {
        for route in routes.candidates() {
            let model_enabled: Option<i64> = sqlx::query_scalar(
                "SELECT enabled FROM provider_models \
                 WHERE provider_id = ? AND model = ?",
            )
            .bind(route.provider_id.as_ref())
            .bind(&route.model)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| ChatBrokerHostError::ProviderDatabaseUnavailable)?;
            if model_enabled != Some(1) {
                return Err(ChatBrokerHostError::ModelBindingMissing);
            }

            let capability_protocol: Option<String> = sqlx::query_scalar(
                "SELECT protocol FROM provider_model_capabilities \
                 WHERE provider_id = ? AND model = ? AND task = ?",
            )
            .bind(route.provider_id.as_ref())
            .bind(&route.model)
            .bind(PROVIDER_CHAT_MODEL_TASK)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| ChatBrokerHostError::ProviderDatabaseUnavailable)?;
            let Some(capability_protocol) = capability_protocol else {
                return Err(ChatBrokerHostError::ModelBindingMissing);
            };
            if capability_protocol != protocol_id(route.protocol) {
                return Err(ChatBrokerHostError::ModelCapabilityMismatch);
            }
        }
        Ok(())
    }

    /// Resolve the transport target for one already-selected route. This is
    /// an exact identity lookup, not a second provider/model selector: every
    /// route identity from the Broker wire request must match the persisted
    /// v4 record before any endpoint or connection data is accepted.
    async fn resolve_attempt_target(
        &self,
        request: &ProviderWireRequest,
    ) -> Result<ProviderAttemptTarget, ProductionRepositoryError> {
        let selection = request.route_identity.clone();
        let record = self
            .load_route_record_for_revision(&selection)
            .await
            .map_err(ProductionRepositoryError::from)?
            .ok_or(ProductionRepositoryError::Missing)?;
        let primary_identity = record
            .identity_for(
                selection.preset_revision_id.clone(),
                selection.model_task.clone(),
            )
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let routes = resolve_chat_route_record(&record, &primary_identity)
            .map_err(ProductionRepositoryError::from)?;
        self.validate_model_bindings(&routes)
            .await
            .map_err(ProductionRepositoryError::from)?;
        let routes = self.apply_technical_capabilities(routes).await?;
        let route = routes
            .candidates()
            .find(|route| {
                route.provider_id == request.provider_id
                    && route.model == request.model
                    && route.protocol == request.protocol
                    && route.model_route_id == selection.route_id
                    && route.model_route_revision == selection.route_revision
                    && route.connection_config_ref == request.connection_config_ref
                    && route.config_revision_digest == request.config_revision_digest
                    && route.credential_ref == request.credential_ref
                    && route.features == request.route_features
            })
            .cloned()
            .ok_or(ProductionRepositoryError::Missing)?;

        let capability = sqlx::query(
            "SELECT protocol, connection_role, endpoint, base_url_override, \
                    allow_cross_origin_credentials, provider_params, output_limit \
             FROM provider_model_capabilities \
             WHERE provider_id = ? AND model = ? AND task = ?",
        )
        .bind(route.provider_id.as_ref())
        .bind(&route.model)
        .bind(PROVIDER_CHAT_MODEL_TASK)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ProductionRepositoryError::Unavailable)?
        .ok_or(ProductionRepositoryError::Missing)?;
        let capability_protocol: String = capability
            .try_get("protocol")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        if capability_protocol != protocol_id(route.protocol) {
            return Err(ProductionRepositoryError::InvalidData);
        }
        let connection_role: String = capability
            .try_get("connection_role")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let endpoint: Option<String> = capability
            .try_get("endpoint")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let base_url_override: Option<String> = capability
            .try_get("base_url_override")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let allow_cross_origin_credentials: bool = capability
            .try_get("allow_cross_origin_credentials")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let provider_params_raw: String = capability
            .try_get("provider_params")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let provider_params: Value = serde_json::from_str(&provider_params_raw)
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        validate_provider_params_for_protocol(
            &capability_protocol,
            ModelTask::Chat,
            &provider_params,
        )
        .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let output_limit: Option<i64> = capability
            .try_get("output_limit")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        if output_limit.is_some_and(|limit| limit <= 0) {
            return Err(ProductionRepositoryError::InvalidData);
        }

        let provider = sqlx::query(
            "SELECT base_url, bedrock_config FROM providers WHERE provider_id = ?",
        )
        .bind(route.provider_id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ProductionRepositoryError::Unavailable)?
        .ok_or(ProductionRepositoryError::Missing)?;
        let provider_base_url: String = provider
            .try_get("base_url")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let bedrock_config: Option<String> = provider
            .try_get("bedrock_config")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let current_digest = provider_config_digest(&self.pool, &route.provider_id).await?;
        if current_digest != route.config_revision_digest {
            return Err(ProductionRepositoryError::InvalidData);
        }

        let (base_url, connection_id) = if connection_role == "default" {
            (provider_base_url, "default".to_owned())
        } else {
            let connection = sqlx::query(
                "SELECT connection_id, base_url \
                 FROM provider_connections \
                 WHERE provider_id = ? AND role = ?",
            )
            .bind(route.provider_id.as_ref())
            .bind(&connection_role)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| ProductionRepositoryError::Unavailable)?
            .ok_or(ProductionRepositoryError::Missing)?;
            (
                connection
                    .try_get("base_url")
                    .map_err(|_| ProductionRepositoryError::InvalidData)?,
                connection
                    .try_get("connection_id")
                    .map_err(|_| ProductionRepositoryError::InvalidData)?,
            )
        };
        if connection_id != route.connection_config_ref.as_ref() {
            return Err(ProductionRepositoryError::InvalidData);
        }

        let framing = if route.protocol == ChatProtocol::Bedrock {
            SingleAttemptFraming::AwsEventStream
        } else {
            SingleAttemptFraming::Sse
        };
        let region = bedrock_config
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|value| value.get("region").and_then(Value::as_str).map(str::to_owned));
        let endpoint = match route.protocol {
            ChatProtocol::Bedrock => {
                let region = region
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or(ProductionRepositoryError::InvalidData)?;
                format!(
                    "https://bedrock-runtime.{region}.amazonaws.com/model/{}/invoke-with-response-stream",
                    route.model
                )
            }
            _ => {
                let endpoint = endpoint
                    .or_else(|| {
                        protocol_task_descriptor(&capability_protocol, ModelTask::Chat)
                            .and_then(|descriptor| {
                                descriptor
                                    .endpoints
                                    .into_iter()
                                    .find(|endpoint| {
                                        endpoint.purpose
                                            == nomifun_model_invoke::ProtocolEndpointPurpose::Submit
                                    })
                                    .map(|endpoint| endpoint.default_value)
                            })
                    })
                    .ok_or(ProductionRepositoryError::InvalidData)?;
                let endpoint = expand_protocol_endpoint_template(
                    &capability_protocol,
                    ModelTask::Chat,
                    "endpoint",
                    &endpoint,
                    &route.model,
                )
                .map_err(|_| ProductionRepositoryError::InvalidData)?;
                let effective_base_url = base_url_override
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(&base_url);
                validate_credentialed_target_url(
                    effective_base_url,
                    allow_cross_origin_credentials,
                    &endpoint,
                    "endpoint",
                    nomifun_model_invoke::ProtocolTransportKind::Http,
                    true,
                )
                .map_err(|_| ProductionRepositoryError::InvalidData)?;
                if reqwest::Url::parse(&endpoint).is_ok() {
                    endpoint
                } else {
                    join_endpoint(effective_base_url, &endpoint)
                }
            }
        };
        Ok(ProviderAttemptTarget {
            url: endpoint,
            framing,
            region,
            provider_params,
            output_limit,
        })
    }
}

#[derive(Clone, Debug)]
struct ProviderAttemptTarget {
    url: String,
    framing: SingleAttemptFraming,
    region: Option<String>,
    provider_params: Value,
    output_limit: Option<i64>,
}

#[async_trait]
impl ProductionModelRepositoryPort for ProductionModelRepository {
    async fn resolve_chat_route(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<Option<ResolvedChatRouteSet>, ProductionRepositoryError> {
        let Some(routes) = self
            .load_route_record(selection)
            .await
            .map_err(ProductionRepositoryError::from)?
        else {
            return Ok(None);
        };
        self.validate_model_bindings(&routes)
            .await
            .map_err(ProductionRepositoryError::from)?;
        Ok(Some(self.apply_technical_capabilities(routes).await?))
    }
}

#[derive(Clone)]
struct ProductionChatCapabilityObserver {
    pool: SqlitePool,
    technical_capabilities: ModelTechnicalCapabilityCache,
}

impl ProductionChatCapabilityObserver {
    fn new(
        pool: SqlitePool,
        technical_capabilities: ModelTechnicalCapabilityCache,
    ) -> Self {
        Self {
            pool,
            technical_capabilities,
        }
    }
}

fn technical_capability_for_feature(
    feature: ChatModelFeature,
) -> Option<ModelTechnicalCapability> {
    match feature {
        ChatModelFeature::ToolCalls => Some(ModelTechnicalCapability::FunctionCalling),
        ChatModelFeature::Reasoning => Some(ModelTechnicalCapability::Reasoning),
        ChatModelFeature::Streaming => Some(ModelTechnicalCapability::Streaming),
        _ => None,
    }
}

fn technical_capability_key(capability: ModelTechnicalCapability) -> &'static str {
    match capability {
        ModelTechnicalCapability::FunctionCalling => "function_calling",
        ModelTechnicalCapability::Reasoning => "reasoning",
        ModelTechnicalCapability::Streaming => "streaming",
    }
}

#[async_trait]
impl ChatCapabilityObserver for ProductionChatCapabilityObserver {
    async fn record_unsupported(
        &self,
        route: &ResolvedChatRoute,
        feature: ChatModelFeature,
    ) {
        let Some(capability) = technical_capability_for_feature(feature) else {
            return;
        };
        // Capture the monotonic revision before recomputing the route digest.
        // If configuration changes at either side of the digest read, the
        // digest comparison or the repository CAS below rejects the late
        // provider response.
        let revision: i64 = match sqlx::query_scalar(
            "SELECT config_revision FROM providers WHERE provider_id = ?",
        )
        .bind(route.provider_id.as_ref())
        .fetch_one(&self.pool)
        .await
        {
            Ok(revision) => revision,
            Err(error) => {
                tracing::warn!(
                    provider_id = route.provider_id.as_ref(),
                    model = route.model,
                    ?feature,
                    ?error,
                    "could not read provider revision before recording unsupported capability"
                );
                return;
            }
        };
        let current_digest = match provider_config_digest(&self.pool, &route.provider_id).await {
            Ok(digest) => digest,
            Err(error) => {
                tracing::warn!(
                    provider_id = route.provider_id.as_ref(),
                    model = route.model,
                    ?feature,
                    ?error,
                    "could not verify provider digest before recording unsupported capability"
                );
                return;
            }
        };
        if current_digest != route.config_revision_digest {
            return;
        }
        let repository = SqliteProviderModelCapabilityRepository::new(self.pool.clone());
        match repository
            .mark_technical_capability_unsupported(
                route.provider_id.as_ref(),
                revision,
                &route.model,
                PROVIDER_CHAT_MODEL_TASK,
                technical_capability_key(capability),
            )
            .await
        {
            Ok(true) => {
                if let Err(error) = self.technical_capabilities.remember_unsupported(
                    route.provider_id.as_ref(),
                    &route.model,
                    revision,
                    capability,
                ) {
                    tracing::warn!(
                        provider_id = route.provider_id.as_ref(),
                        model = route.model,
                        ?feature,
                        ?error,
                        "durable capability downgrade succeeded but cache update failed"
                    );
                }
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    provider_id = route.provider_id.as_ref(),
                    model = route.model,
                    ?feature,
                    ?error,
                    "could not persist unsupported technical capability"
                );
            }
        }
    }
}

fn protocol_id(protocol: ChatProtocol) -> &'static str {
    match protocol {
        ChatProtocol::Anthropic => "anthropic.messages",
        ChatProtocol::OpenaiChat => "openai.chat_text",
        ChatProtocol::OpenaiResponses => "openai.responses",
        ChatProtocol::Gemini => "gemini.generate_text",
        ChatProtocol::Bedrock => "bedrock.anthropic_messages",
        ChatProtocol::Vertex => "vertex.anthropic_messages",
    }
}

fn auth_scheme_key(scheme: &AuthScheme) -> String {
    match scheme {
        AuthScheme::Bearer => "bearer".to_owned(),
        AuthScheme::TokenHeader => "token".to_owned(),
        AuthScheme::HeaderKey(name) => format!("header_key:{}", name.to_ascii_lowercase()),
        AuthScheme::QueryKey(name) => format!("query_key:{}", name.to_ascii_lowercase()),
        AuthScheme::MultiHeader(_) => "volc_voice".to_owned(),
        AuthScheme::Bedrock => "bedrock".to_owned(),
    }
}

/// Keep the production broker's connection boundary aligned with the
/// model-invoke manifest. The manifest, rather than the transport vocabulary,
/// owns which schemes a chat protocol may use.
fn validate_protocol_auth(
    protocol: &str,
    scheme: &AuthScheme,
) -> Result<(), ChatBrokerHostError> {
    let descriptor = protocol_descriptor(protocol).ok_or_else(|| {
        ChatBrokerHostError::InvalidRouteRecord(format!(
            "unknown or chat-incompatible protocol {protocol:?}"
        ))
    })?;
    if !descriptor.supported_tasks.contains(&ModelTask::Chat) {
        return Err(ChatBrokerHostError::InvalidRouteRecord(format!(
            "protocol {protocol:?} is not a Chat protocol"
        )));
    }

    let actual = auth_scheme_key(scheme);
    let allowed = descriptor.allowed_auth_schemes.iter().any(|candidate| {
        let candidate = candidate.trim().to_ascii_lowercase();
        candidate == actual
            || (candidate == "header_key:<name>" && actual.starts_with("header_key:"))
            || (candidate == "query_key:<param>" && actual.starts_with("query_key:"))
    });
    if allowed {
        Ok(())
    } else {
        Err(ChatBrokerHostError::InvalidRouteRecord(format!(
            "protocol {protocol:?} does not accept connection auth scheme {actual:?}"
        )))
    }
}

fn register_route_credential(
    credentials: &ConnectionCredentialLeaseRegistry,
    route: &ResolvedChatRoute,
    auth_scheme: String,
    encrypted_material: String,
) -> Result<(), ProductionRepositoryError> {
    // Validate before taking the registry write path. A bad protocol/scheme
    // pair must not leave a credential entry that a later route can observe.
    let scheme =
        AuthScheme::parse(&auth_scheme).map_err(|_| ProductionRepositoryError::InvalidData)?;
    validate_protocol_auth(protocol_id(route.protocol), &scheme)
        .map_err(ProductionRepositoryError::from)?;
    credentials
        .register_encrypted(
            route.credential_ref.clone(),
            route.provider_id.clone(),
            route.connection_config_ref.clone(),
            auth_scheme,
            encrypted_material,
        )
        .map_err(ProductionRepositoryError::from)
}

/// An encrypted credential registry. Route records provide the credential
/// reference explicitly; the registry never derives it from a connection ID.
#[derive(Clone, Default)]
pub struct ConnectionCredentialLeaseRegistry {
    credentials: Arc<RwLock<BTreeMap<ProviderCredentialRef, RegisteredCredential>>>,
    leases: Arc<RwLock<BTreeMap<String, RegisteredLease>>>,
}

#[derive(Clone)]
struct RegisteredCredential {
    provider_id: ProviderIdRef,
    connection_config_ref: ConnectionConfigRef,
    auth_scheme: String,
    encrypted_material: String,
}

#[derive(Clone)]
struct RegisteredLease {
    auth_scheme: String,
    /// Decrypted credentials are wiped when the lease is released or dropped.
    material: zeroize::Zeroizing<String>,
}

/// Scope the decrypted lease to the provider-opening future. Broker
/// cancellation drops this guard even while route lookup or HTTP opening is
/// pending; early validation errors must release it as well.
struct AttemptCredentialGuard {
    registry: ConnectionCredentialLeaseRegistry,
    handle: String,
}

impl Drop for AttemptCredentialGuard {
    fn drop(&mut self) {
        self.registry.release(&self.handle);
    }
}

impl ConnectionCredentialLeaseRegistry {
    fn register_encrypted(
        &self,
        credential_ref: ProviderCredentialRef,
        provider_id: ProviderIdRef,
        connection_config_ref: ConnectionConfigRef,
        auth_scheme: String,
        encrypted_material: String,
    ) -> Result<(), ChatBrokerHostError> {
        if encrypted_material.trim().is_empty() {
            return Err(ChatBrokerHostError::CredentialRegistry);
        }
        let mut credentials = self
            .credentials
            .write()
            .map_err(|_| ChatBrokerHostError::CredentialRegistry)?;
        let next = RegisteredCredential {
            provider_id,
            connection_config_ref,
            auth_scheme,
            encrypted_material,
        };
        if let Some(existing) = credentials.get(&credential_ref) {
            if existing.provider_id != next.provider_id
                || existing.connection_config_ref != next.connection_config_ref
                || existing.auth_scheme != next.auth_scheme
                || existing.encrypted_material != next.encrypted_material
            {
                return Err(ChatBrokerHostError::CredentialRegistry);
            }
        } else {
            credentials.insert(credential_ref, next);
        }
        Ok(())
    }

    async fn lease(
        &self,
        credential_ref: &ProviderCredentialRef,
        target: &CredentialTarget,
        encryption_key: &[u8; 32],
    ) -> Result<Option<CredentialLease>, ProductionRepositoryError> {
        let registered = {
            let credentials = self
                .credentials
                .read()
                .map_err(|_| ProductionRepositoryError::Unavailable)?;
            credentials.get(credential_ref).cloned()
        };
        let Some(registered) = registered else {
            return Ok(None);
        };
        if registered.provider_id != target.provider_id
            || registered.connection_config_ref != target.connection_config_ref
        {
            return Err(ProductionRepositoryError::InvalidData);
        }
        let material = nomifun_common::decrypt_string(
            &registered.encrypted_material,
            encryption_key,
        )
        .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let handle = format!("chat-credential-{}", uuid::Uuid::now_v7());
        let lease = CredentialLease::new(credential_ref.clone(), target.clone(), &handle);
        self.leases
            .write()
            .map_err(|_| ProductionRepositoryError::Unavailable)?
            .insert(
                handle,
                RegisteredLease {
                    auth_scheme: registered.auth_scheme,
                    material: zeroize::Zeroizing::new(material),
                },
            );
        Ok(Some(lease))
    }

    fn release(&self, handle: &str) {
        if let Ok(mut leases) = self.leases.write() {
            leases.remove(handle);
        }
    }

    fn auth_material_for_handle(
        &self,
        handle: &str,
    ) -> Result<AuthMaterial, InvokeError> {
        let leases = self
            .leases
            .read()
            .map_err(|_| InvokeError::config("chat credential lease registry is unavailable"))?;
        let registered = leases
            .get(handle)
            .ok_or_else(|| InvokeError::config("chat credential lease is missing"))?;
        let credentials = serde_json::from_str(registered.material.as_str())
            .map_err(|_| InvokeError::config("chat credential payload is invalid"))?;
        let material = AuthMaterial {
            scheme: AuthScheme::parse(&registered.auth_scheme)?,
            credentials,
        };
        material.validate_credentials()?;
        Ok(material)
    }
}

#[derive(Clone)]
struct RegistryCredentialResolver {
    registry: ConnectionCredentialLeaseRegistry,
}

#[async_trait]
impl OpaqueCredentialResolver for RegistryCredentialResolver {
    async fn resolve(
        &self,
        lease: &OpaqueCredentialLease,
    ) -> Result<AuthMaterial, InvokeError> {
        self.registry.auth_material_for_handle(lease.handle())
    }
}

/// App composition adapter that turns one Broker wire request into one
/// already-resolved provider HTTP attempt. It owns no route selection,
/// credential rotation, retry, or failover policy.
pub struct ProductionChatModelInvoke {
    routes: Arc<ProductionModelRepository>,
    credentials: ConnectionCredentialLeaseRegistry,
    executor: SingleAttemptHttpExecutor,
}

impl ProductionChatModelInvoke {
    pub fn new(
        routes: Arc<ProductionModelRepository>,
        credentials: ConnectionCredentialLeaseRegistry,
        http: reqwest::Client,
    ) -> Self {
        let resolver = Arc::new(RegistryCredentialResolver {
            registry: credentials.clone(),
        });
        Self {
            routes,
            credentials: credentials.clone(),
            executor: SingleAttemptHttpExecutor::new(http, resolver),
        }
    }
}

#[async_trait]
impl ChatModelInvokePort for ProductionChatModelInvoke {
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError> {
        let credential_handle = credential.opaque_handle().to_owned();
        let credential_guard = AttemptCredentialGuard {
            registry: self.credentials.clone(),
            handle: credential_handle.clone(),
        };
        let mut target = self
            .routes
            .resolve_attempt_target(&request)
            .await
            .map_err(|error| {
                let mut mapped = ChatModelError::new(
                    ChatModelErrorCode::AdapterUnavailable,
                    "the exact provider transport target is unavailable",
                    ChatRetryDirective::Never,
                )
                .with_route(request.route_identity.route_id.clone());
                mapped.provider_status = Some(repository_error_status(error));
                mapped
            })?;
        let lease = OpaqueCredentialLease::new(&credential_handle)
            .map_err(invoke_error_to_chat_error)?;
        let mut body = merge_chat_provider_params(
            request.body,
            &target.provider_params,
            request.protocol,
            target.output_limit,
        )?;
        if !request.route_features.contains(&ChatModelFeature::Streaming) {
            configure_non_streaming_attempt(
                request.protocol,
                &mut target.url,
                &mut target.framing,
                &mut body,
            )?;
        }
        let result = self
            .executor
            .open_stream(SingleAttemptRequest {
                protocol: protocol_id(request.protocol).to_owned(),
                url: target.url,
                model: request.model,
                body,
                credential: lease,
                timeout: Duration::from_secs(120),
                idle_timeout: Duration::from_secs(120),
                framing: target.framing,
                region: target.region,
            })
            .await;
        // The HTTP executor has already attached the credential and returned
        // a response body stream; retain no decrypted material while the
        // provider stream is being consumed.
        drop(credential_guard);
        let stream = result.map_err(invoke_error_to_chat_error)?;
        Ok(Box::pin(stream.map(|frame| {
            frame
                .map(|frame| ProviderWireFrame {
                    event: frame.event,
                    data: frame.data,
                })
                .map_err(invoke_error_to_chat_error)
        })))
    }
}

fn configure_non_streaming_attempt(
    protocol: ChatProtocol,
    url: &mut String,
    framing: &mut SingleAttemptFraming,
    body: &mut Value,
) -> Result<(), ChatModelError> {
    let object = body.as_object_mut().ok_or_else(|| {
        ChatModelError::new(
            ChatModelErrorCode::ProtocolViolation,
            "provider chat request body is not a JSON object",
            ChatRetryDirective::Never,
        )
    })?;
    match protocol {
        ChatProtocol::OpenaiChat => {
            object.insert("stream".into(), Value::Bool(false));
            object.remove("stream_options");
        }
        ChatProtocol::OpenaiResponses | ChatProtocol::Anthropic | ChatProtocol::Vertex => {
            object.insert("stream".into(), Value::Bool(false));
        }
        ChatProtocol::Gemini => {
            let mut parsed = reqwest::Url::parse(url).map_err(|_| {
                ChatModelError::new(
                    ChatModelErrorCode::AdapterUnavailable,
                    "configured Gemini endpoint is invalid",
                    ChatRetryDirective::Never,
                )
            })?;
            let path = parsed.path().replace(":streamGenerateContent", ":generateContent");
            parsed.set_path(&path);
            let query = parsed
                .query_pairs()
                .filter(|(key, value)| !(key == "alt" && value == "sse"))
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>();
            parsed.set_query(None);
            if !query.is_empty() {
                parsed.query_pairs_mut().extend_pairs(query);
            }
            *url = parsed.into();
        }
        ChatProtocol::Bedrock => {
            *url = url.replace("/invoke-with-response-stream", "/invoke");
        }
    }
    // The executor's bounded JSON framing turns one response object into one
    // broker frame; no SSE/AWS event-stream parser is used in this mode.
    *framing = SingleAttemptFraming::Sse;
    Ok(())
}

fn merge_chat_provider_params(
    mut body: Value,
    configured: &Value,
    protocol: ChatProtocol,
    output_limit: Option<i64>,
) -> Result<Value, ChatModelError> {
    let configured = configured.as_object().ok_or_else(|| {
        ChatModelError::new(
            ChatModelErrorCode::InvalidRequest,
            "provider chat parameters are not a JSON object",
            ChatRetryDirective::Never,
        )
    })?;
    let body_object = body.as_object_mut().ok_or_else(|| {
        ChatModelError::new(
            ChatModelErrorCode::ProtocolViolation,
            "provider chat request body is not a JSON object",
            ChatRetryDirective::Never,
        )
    })?;
    let reasoning_effort = configured
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::to_owned);

    for (key, value) in configured {
        // Provider defaults may tune sampling, but cannot add model-owned
        // messages/tools/thinking after the Broker's feature admission, or
        // reinsert model/stream fields removed by the cloud wire encoder.
        if protocol.uses_anthropic_messages() && matches!(key.as_str(),
            "model" | "stream" | "messages" | "system" | "tools" | "tool_choice"
            | "thinking" | "anthropic_version") {
            continue;
        }
        if matches!(
            key.as_str(),
            "max_tokens_field"
                | "chain_rounds"
                | "require_reasoning_content"
                | "reasoning_effort"
        ) {
            continue;
        }
        merge_missing_json_value(body_object, key, value);
    }

    let configured_ceiling_key = configured
        .get("max_tokens_field")
        .and_then(Value::as_str)
        .filter(|key| !key.trim().is_empty());
    let ceiling = output_limit
        .map(|limit| {
            u32::try_from(limit).map_err(|_| {
                ChatModelError::new(
                    ChatModelErrorCode::InvalidRequest,
                    "provider output ceiling exceeds the supported range",
                    ChatRetryDirective::Never,
                )
            })
        })
        .transpose()?;
    match protocol {
        ChatProtocol::Gemini => {
            if let Some(ceiling) = ceiling {
                let generation = body_object
                    .entry("generationConfig".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let generation = generation.as_object_mut().ok_or_else(|| {
                    ChatModelError::protocol_violation(
                        "provider generationConfig is not a JSON object",
                    )
                })?;
                cap_json_number(generation, "maxOutputTokens", ceiling);
            }
        }
        ChatProtocol::OpenaiResponses => {
            if let Some(ceiling) = ceiling {
                cap_json_number(body_object, "max_output_tokens", ceiling);
            }
        }
        ChatProtocol::OpenaiChat => {
            if let Some(ceiling) = ceiling {
                let key = configured_ceiling_key.unwrap_or("max_tokens");
                for default_key in [
                    "max_tokens",
                    "max_completion_tokens",
                    "max_output_tokens",
                ] {
                    if default_key != key {
                        body_object.remove(default_key);
                    }
                }
                cap_json_number(body_object, key, ceiling);
            }
        }
        ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => {
            if let Some(ceiling) = ceiling {
                cap_json_number(body_object, "max_tokens", ceiling);
            }
            let max_tokens = body_object.get("max_tokens").and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .ok_or_else(|| ChatModelError::invalid_request("Messages output ceiling must be positive"))?;
            if let Some(thinking) = body_object.get("thinking") {
                let budget = thinking.get("budget_tokens").and_then(Value::as_u64);
                if thinking.get("type").and_then(Value::as_str) != Some("enabled")
                    || !budget.is_some_and(|budget| budget >= 1024 && budget < max_tokens) {
                    return Err(ChatModelError::invalid_request(
                        "Messages thinking budget must remain below the effective provider output ceiling",
                    ));
                }
            }
        }
    }
    if let Some(reasoning_effort) = reasoning_effort {
        match protocol {
            ChatProtocol::OpenaiChat => {
                body_object
                    .entry("reasoning_effort")
                    .or_insert_with(|| Value::String(reasoning_effort));
            }
            ChatProtocol::OpenaiResponses => {
                let reasoning = body_object
                    .entry("reasoning".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let reasoning = reasoning.as_object_mut().ok_or_else(|| {
                    ChatModelError::invalid_request(
                        "OpenAI Responses reasoning configuration must be an object",
                    )
                })?;
                reasoning
                    .entry("effort")
                    .or_insert_with(|| Value::String(reasoning_effort));
            }
            ChatProtocol::Gemini => {
                let generation = body_object
                    .entry("generationConfig".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let generation = generation.as_object_mut().ok_or_else(|| {
                    ChatModelError::invalid_request(
                        "Gemini generationConfig must be an object",
                    )
                })?;
                let thinking = generation
                    .entry("thinkingConfig".to_owned())
                    .or_insert_with(|| Value::Object(Default::default()));
                let thinking = thinking.as_object_mut().ok_or_else(|| {
                    ChatModelError::invalid_request(
                        "Gemini thinkingConfig must be an object",
                    )
                })?;
                thinking
                    .entry("thinkingLevel")
                    .or_insert_with(|| Value::String(reasoning_effort));
            }
            ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => {
                return Err(ChatModelError::invalid_request(
                    "configured reasoning effort is unsupported by this Chat protocol",
                ));
            }
        }
    }
    Ok(body)
}

fn merge_missing_json_value(
    target: &mut serde_json::Map<String, Value>,
    key: &str,
    incoming: &Value,
) {
    match target.get_mut(key) {
        Some(Value::Object(existing)) => {
            if let Value::Object(incoming) = incoming {
                for (nested_key, nested_value) in incoming {
                    if let Some(Value::Object(existing_nested)) = existing.get_mut(nested_key)
                        && let Value::Object(incoming_nested) = nested_value
                    {
                        merge_json_object_missing(existing_nested, incoming_nested);
                    } else {
                        existing
                            .entry(nested_key.clone())
                            .or_insert_with(|| nested_value.clone());
                    }
                }
            }
        }
        Some(_) => {}
        None => {
            target.insert(key.to_owned(), incoming.clone());
        }
    }
}

fn merge_json_object_missing(
    target: &mut serde_json::Map<String, Value>,
    incoming: &serde_json::Map<String, Value>,
) {
    for (key, value) in incoming {
        if let Some(Value::Object(existing)) = target.get_mut(key)
            && let Value::Object(incoming) = value
        {
            merge_json_object_missing(existing, incoming);
        } else {
            target.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn cap_json_number(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    ceiling: u32,
) {
    let ceiling = u64::from(ceiling);
    match object.get(key).and_then(Value::as_u64) {
        Some(current) if current > ceiling => {
            object.insert(key.to_owned(), Value::from(ceiling));
        }
        None => {
            object.insert(key.to_owned(), Value::from(ceiling));
        }
        _ => {}
    }
}




fn repository_error_status(error: ProductionRepositoryError) -> u16 {
    match error {
        ProductionRepositoryError::Missing => 404,
        ProductionRepositoryError::Unavailable => 503,
        ProductionRepositoryError::InvalidData => 422,
        ProductionRepositoryError::PermissionDenied => 403,
    }
}

fn invoke_error_to_chat_error(error: InvokeError) -> ChatModelError {
    if error.is_context_length_rejected() {
        let mut mapped = ChatModelError::new(
            ChatModelErrorCode::PromptTooLong,
            "provider rejected the input context length",
            ChatRetryDirective::Never,
        );
        mapped.provider_status = error.http_status;
        return mapped;
    }
    let unsupported_feature = error.unsupported_technical_capability.map(|capability| match capability {
        nomifun_api_types::ModelTechnicalCapability::FunctionCalling => ChatModelFeature::ToolCalls,
        nomifun_api_types::ModelTechnicalCapability::Reasoning => ChatModelFeature::Reasoning,
        nomifun_api_types::ModelTechnicalCapability::Streaming => ChatModelFeature::Streaming,
    });
    let retry = if unsupported_feature.is_some() {
        ChatRetryDirective::Failover
    } else {
        match error.kind {
            InvokeErrorKind::Auth
            | InvokeErrorKind::RateLimited
            | InvokeErrorKind::ProviderError
            | InvokeErrorKind::Network
            | InvokeErrorKind::Timeout => ChatRetryDirective::Failover,
            InvokeErrorKind::Config
            | InvokeErrorKind::InvalidParams
            | InvokeErrorKind::UnsupportedTask
            | InvokeErrorKind::NoAdapter
            | InvokeErrorKind::MissingConnection
            | InvokeErrorKind::ParseError
            | InvokeErrorKind::NonApiResponse
            | InvokeErrorKind::NotPollable
            | InvokeErrorKind::ContentPolicy
            | InvokeErrorKind::QuotaExhausted
            | InvokeErrorKind::JobFailed => ChatRetryDirective::Never,
        }
    };
    let code = if unsupported_feature.is_some() {
        ChatModelErrorCode::UnsupportedFeature
    } else {
        match error.kind {
        InvokeErrorKind::Auth => ChatModelErrorCode::AuthenticationFailed,
        InvokeErrorKind::RateLimited => ChatModelErrorCode::RateLimited,
        InvokeErrorKind::InvalidParams | InvokeErrorKind::UnsupportedTask => {
            ChatModelErrorCode::InvalidRequest
        }
        InvokeErrorKind::NoAdapter | InvokeErrorKind::MissingConnection => {
            ChatModelErrorCode::AdapterUnavailable
        }
        InvokeErrorKind::Config => ChatModelErrorCode::AdapterUnavailable,
        InvokeErrorKind::ProviderError
        | InvokeErrorKind::Network
        | InvokeErrorKind::Timeout
        | InvokeErrorKind::QuotaExhausted
        | InvokeErrorKind::JobFailed => ChatModelErrorCode::ProviderUnavailable,
        InvokeErrorKind::ParseError | InvokeErrorKind::NonApiResponse => {
            ChatModelErrorCode::ProtocolViolation
        }
        InvokeErrorKind::NotPollable | InvokeErrorKind::ContentPolicy => {
            ChatModelErrorCode::UnsupportedFeature
        }
        }
    };
    let mut mapped = ChatModelError::new(code, "provider chat attempt failed", retry);
    mapped.retry_after_ms = error.retry_after_ms;
    mapped.provider_status = error.http_status;
    mapped.unsupported_feature = unsupported_feature;
    mapped
}

/// DB-backed connection repository. Named routes use the immutable
/// `provider_connections.connection_id`; the explicit `"default"` reference
/// uses the provider row as its canonical default connection.
#[derive(Clone)]
pub struct ProductionConnectionRepository {
    pool: SqlitePool,
    credentials: ConnectionCredentialLeaseRegistry,
}

impl ProductionConnectionRepository {
    pub fn new(pool: SqlitePool, credentials: ConnectionCredentialLeaseRegistry) -> Self {
        Self { pool, credentials }
    }

    pub fn credentials(&self) -> &ConnectionCredentialLeaseRegistry {
        &self.credentials
    }
}

#[async_trait]
impl ProductionConnectionRepositoryPort for ProductionConnectionRepository {
    async fn find_connection(
        &self,
        route: &ResolvedChatRoute,
    ) -> Result<Option<ConnectionRepositoryRecord>, ProductionRepositoryError> {
        if route.connection_config_ref.as_ref() == "default" {
            let row = sqlx::query(
                "SELECT provider_id, auth_scheme, credentials_encrypted \
                 FROM providers WHERE provider_id = ?",
            )
            .bind(route.provider_id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| ProductionRepositoryError::Unavailable)?;
            let Some(row) = row else {
                return Ok(None);
            };
            let provider_id: String = row
                .try_get("provider_id")
                .map_err(|_| ProductionRepositoryError::InvalidData)?;
            let auth_scheme: String = row
                .try_get("auth_scheme")
                .map_err(|_| ProductionRepositoryError::InvalidData)?;
            let encrypted_material: String = row
                .try_get("credentials_encrypted")
                .map_err(|_| ProductionRepositoryError::InvalidData)?;
            if provider_id != route.provider_id.as_ref() {
                return Err(ProductionRepositoryError::InvalidData);
            }
            register_route_credential(
                &self.credentials,
                route,
                auth_scheme,
                encrypted_material,
            )?;
            return Ok(Some(ConnectionRepositoryRecord {
                provider_id: route.provider_id.clone(),
                connection_config_ref: route.connection_config_ref.clone(),
                credential_ref: route.credential_ref.clone(),
            }));
        }
        let row = sqlx::query(
            "SELECT provider_id, connection_id, auth_scheme, credentials_encrypted \
             FROM provider_connections WHERE connection_id = ?",
        )
        .bind(route.connection_config_ref.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ProductionRepositoryError::Unavailable)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let provider_id: String = row
            .try_get("provider_id")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let connection_id: String = row
            .try_get("connection_id")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let encrypted_material: String = row
            .try_get("credentials_encrypted")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        let auth_scheme: String = row
            .try_get("auth_scheme")
            .map_err(|_| ProductionRepositoryError::InvalidData)?;
        if provider_id != route.provider_id.as_ref()
            || connection_id != route.connection_config_ref.as_ref()
        {
            return Err(ProductionRepositoryError::InvalidData);
        }
        register_route_credential(
            &self.credentials,
            route,
            auth_scheme,
            encrypted_material,
        )?;
        Ok(Some(ConnectionRepositoryRecord {
            provider_id: route.provider_id.clone(),
            connection_config_ref: route.connection_config_ref.clone(),
            credential_ref: route.credential_ref.clone(),
        }))
    }

    async fn lease_credential(
        &self,
        credential_ref: &ProviderCredentialRef,
        target: &CredentialTarget,
        encryption_key: &[u8; 32],
    ) -> Result<Option<CredentialLease>, ProductionRepositoryError> {
        self.credentials
            .lease(credential_ref, target, encryption_key)
            .await
    }
}

/// Complete app-side repository composition. The host supplies the
/// single-attempt model-invoke port and the real causality gate at build time.
#[derive(Clone)]
pub struct ChatBrokerHostComposition {
    provider_repository: Arc<ProductionProviderRepository>,
    model_repository: Arc<ProductionModelRepository>,
    connection_repository: Arc<ProductionConnectionRepository>,
    capability_observer: Arc<ProductionChatCapabilityObserver>,
    encryption_key: [u8; 32],
}

impl ChatBrokerHostComposition {
    /// Default product route storage in the canonical main SQLite.
    pub fn for_nomi_core(pool: SqlitePool, encryption_key: [u8; 32]) -> Self {
        let technical_capabilities = ModelTechnicalCapabilityCache::default();
        Self {
            provider_repository: Arc::new(ProductionProviderRepository::new(pool.clone())),
            model_repository: Arc::new(ProductionModelRepository::with_technical_capabilities(
                pool.clone(),
                technical_capabilities.clone(),
            )),
            connection_repository: Arc::new(ProductionConnectionRepository::new(
                pool.clone(),
                ConnectionCredentialLeaseRegistry::default(),
            )),
            capability_observer: Arc::new(ProductionChatCapabilityObserver::new(
                pool,
                technical_capabilities,
            )),
            encryption_key,
        }
    }
    pub fn build_broker(
        &self,
        causality_gate: Arc<dyn ChatCausalityGate>,
        model_invoke: Arc<dyn ChatModelInvokePort>,
        retry_policy: BrokerRetryPolicy,
    ) -> Result<Arc<dyn ChatBrokerPort>, ProductionBrokerError> {
        let mut dependencies = ProductionBrokerDependencies::new(
            self.provider_repository.clone(),
            self.connection_repository.clone(),
            self.model_repository.clone(),
            self.encryption_key,
            causality_gate,
            model_invoke,
        );
        dependencies.retry_policy = retry_policy;
        dependencies.capability_observer = self.capability_observer.clone();
        build_production_chat_model_broker(dependencies)
    }

    pub fn credential_registry(&self) -> &ConnectionCredentialLeaseRegistry {
        self.connection_repository.credentials()
    }

    pub fn build_model_invoke(
        &self,
        http: reqwest::Client,
    ) -> Arc<dyn ChatModelInvokePort> {
        Arc::new(ProductionChatModelInvoke::new(
            self.model_repository.clone(),
            self.credential_registry().clone(),
            http,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use nomifun_agent_contracts::{
        AgentPresetRevisionPayload, ChatRouteCandidate, ChatRouteFeature, ChatRouteIdentity,
        ChatRouteProtocol, ChatRouteRecord, ChatRouteRecordSchema, ChatRouteTask,
    };
    use serde_json::json;

    #[tokio::test]
    async fn cancelling_provider_open_releases_only_its_credential_lease() {
        let registry = ConnectionCredentialLeaseRegistry::default();
        for handle in ["cancelled-attempt", "other-attempt"] {
            registry.leases.write().unwrap().insert(handle.to_owned(), RegisteredLease {
                auth_scheme: "bearer".to_owned(),
                material: zeroize::Zeroizing::new("{}".to_owned()),
            });
        }
        let (started, ready) = tokio::sync::oneshot::channel();
        let task_registry = registry.clone();
        let task = tokio::spawn(async move {
            let _guard = AttemptCredentialGuard {
                registry: task_registry,
                handle: "cancelled-attempt".to_owned(),
            };
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let leases = registry.leases.read().unwrap();
        assert!(!leases.contains_key("cancelled-attempt"));
        assert!(leases.contains_key("other-attempt"));
    }

    fn selection() -> ChatRouteSelection {
        ChatRouteIdentity::new(
            "preset@1",
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
            "opaque-route".into(),
            7,
        )
    }

    fn candidate() -> ChatRouteCandidate {
        ChatRouteCandidate {
            model_route_id: "opaque-route".into(),
            model_route_revision: 7,
            provider_id: "provider-1".to_owned(),
            model: "model-1".to_owned(),
            protocol: ChatRouteProtocol::OpenaiChat,
            connection_config_ref: "connection-1".into(),
            config_revision_digest: "a".repeat(64).into(),
            credential_ref: "credential-1".to_owned(),
            features: BTreeSet::from([
                ChatRouteFeature::TextInput,
                ChatRouteFeature::TextOutput,
            ]),
            activation_features: BTreeSet::new(),
        }
    }

    fn revision_payload(record: ChatRouteRecord) -> AgentPresetRevisionPayload {
        AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::from([(
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT.to_owned(),
                record.primary.model_route_id.clone(),
            )]),
            chat_route_records: BTreeMap::from([(
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT.to_owned(),
                record,
            )]),
            enabled_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Route lookup test agent".to_owned(),
            instructions: "Exercise the persisted canonical route.".to_owned(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        }
    }

    #[test]
    fn route_record_is_explicit_and_does_not_decode_route_id() {
        let record = ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: candidate(),
            failovers: Vec::new(),
        };
        let routes = resolve_chat_route_record(
            &record,
            &selection(),
        )
        .unwrap();
        assert_eq!(routes.primary.model_route_id.as_ref(), "opaque-route");
    }

    #[test]
    fn legacy_string_route_payload_fails_closed() {
        let mut payload = serde_json::to_value(revision_payload(ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: candidate(),
            failovers: Vec::new(),
        }))
        .unwrap();
        payload["chat_route_records"][nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT] =
            json!("opaque-route");
        let error = decode_agent_preset_revision_payload(&payload.to_string()).unwrap_err();
        assert!(matches!(
            error,
            ChatBrokerHostError::InvalidRouteRecord(_)
        ));
    }

    #[test]
    fn unknown_route_record_fields_fail_closed() {
        let mut payload = serde_json::to_value(revision_payload(ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: candidate(),
            failovers: Vec::new(),
        }))
        .unwrap();
        payload["chat_route_records"][nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT]
            ["derived_provider"] = json!("must-not-be-accepted");
        let error = decode_agent_preset_revision_payload(&payload.to_string()).unwrap_err();
        assert!(matches!(
            error,
            ChatBrokerHostError::InvalidRouteRecord(_)
        ));
    }

    #[test]
    fn provider_params_are_merged_without_overriding_typed_fields_and_ceiling_is_capped() {
        let body = json!({
            "model": "typed-model",
            "temperature": 0.7,
            "generationConfig": {
                "maxOutputTokens": 2048
            }
        });
        let configured = json!({
            "temperature": 0.2,
            "candidateCount": 2,
            "generationConfig": {
                "candidateCount": 3
            }
        });

        let merged = merge_chat_provider_params(
            body,
            &configured,
            ChatProtocol::Gemini,
            Some(1024),
        )
        .unwrap();

        assert_eq!(merged["temperature"], 0.7);
        assert_eq!(merged["candidateCount"], 2);
        assert_eq!(merged["generationConfig"]["candidateCount"], 3);
        assert_eq!(merged["generationConfig"]["maxOutputTokens"], 1024);
    }

    #[test]
    fn provider_control_fields_do_not_leak_into_the_wire_body() {
        let merged = merge_chat_provider_params(
            json!({"max_completion_tokens": 4096}),
            &json!({
                "max_tokens_field": "max_completion_tokens",
                "require_reasoning_content": false,
                "temperature": 0.25
            }),
            ChatProtocol::OpenaiChat,
            Some(512),
        )
        .unwrap();

        assert_eq!(merged["max_completion_tokens"], 512);
        assert_eq!(merged["temperature"], 0.25);
        assert!(merged.get("max_tokens_field").is_none());
        assert!(merged.get("require_reasoning_content").is_none());
    }

    #[test]
    fn model_reasoning_effort_maps_to_each_supported_chat_wire_shape_as_a_default() {
        let openai_chat = merge_chat_provider_params(
            json!({"reasoning_effort":"low"}),
            &json!({"reasoning_effort":"high"}),
            ChatProtocol::OpenaiChat,
            None,
        )
        .unwrap();
        assert_eq!(openai_chat["reasoning_effort"], "low");

        let responses = merge_chat_provider_params(
            json!({"reasoning":{"summary":"concise","effort":"low"}}),
            &json!({"reasoning_effort":"medium"}),
            ChatProtocol::OpenaiResponses,
            None,
        )
        .unwrap();
        assert_eq!(responses["reasoning"]["effort"], "low");
        assert_eq!(responses["reasoning"]["summary"], "concise");

        let gemini = merge_chat_provider_params(
            json!({"generationConfig":{"temperature":0.2,"thinkingConfig":{"thinkingLevel":"low"}}}),
            &json!({"reasoning_effort":"high"}),
            ChatProtocol::Gemini,
            None,
        )
        .unwrap();
        assert_eq!(gemini["generationConfig"]["thinkingConfig"]["thinkingLevel"], "low");
        assert_eq!(gemini["generationConfig"]["temperature"], 0.2);

        let defaults = [
            (
                ChatProtocol::OpenaiChat,
                json!({}),
                vec!["reasoning_effort"],
            ),
            (
                ChatProtocol::OpenaiResponses,
                json!({}),
                vec!["reasoning", "effort"],
            ),
            (
                ChatProtocol::Gemini,
                json!({}),
                vec!["generationConfig", "thinkingConfig", "thinkingLevel"],
            ),
        ];
        for (protocol, body, path) in defaults {
            let merged = merge_chat_provider_params(
                body,
                &json!({"reasoning_effort":"high"}),
                protocol,
                None,
            )
            .unwrap();
            let value = path
                .iter()
                .fold(&merged, |value, key| &value[*key]);
            assert_eq!(value, "high");
        }
    }

    #[test]
    fn model_reasoning_effort_never_leaks_as_an_unsupported_messages_field() {
        let error = merge_chat_provider_params(
            json!({"max_tokens":4096}),
            &json!({"reasoning_effort":"high"}),
            ChatProtocol::Anthropic,
            Some(4096),
        )
        .unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
        assert!(error.message.contains("reasoning effort"));
    }

    #[test]
    fn explicit_http_capability_rejection_is_failover_eligible_and_typed() {
        let mut source = InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "provider returned a bounded machine-readable error",
        )
        .with_http_status(400);
        source.unsupported_technical_capability =
            Some(ModelTechnicalCapability::FunctionCalling);
        let mapped = invoke_error_to_chat_error(source);
        assert_eq!(mapped.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(mapped.retry, ChatRetryDirective::Failover);
        assert_eq!(mapped.unsupported_feature, Some(ChatModelFeature::ToolCalls));
        assert_eq!(mapped.provider_status, Some(400));
    }

    #[test]
    fn observed_non_streaming_routes_use_bounded_json_transport() {
        let mut openai_url = "https://example.invalid/v1/chat/completions".to_owned();
        let mut openai_framing = SingleAttemptFraming::Sse;
        let mut openai_body = json!({
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        configure_non_streaming_attempt(
            ChatProtocol::OpenaiChat,
            &mut openai_url,
            &mut openai_framing,
            &mut openai_body,
        )
        .unwrap();
        assert_eq!(openai_body["stream"], false);
        assert!(openai_body.get("stream_options").is_none());

        let mut gemini_url =
            "https://example.invalid/v1beta/models/model:streamGenerateContent?alt=sse".to_owned();
        let mut gemini_framing = SingleAttemptFraming::Sse;
        let mut gemini_body = json!({"contents": []});
        configure_non_streaming_attempt(
            ChatProtocol::Gemini,
            &mut gemini_url,
            &mut gemini_framing,
            &mut gemini_body,
        )
        .unwrap();
        assert_eq!(
            gemini_url,
            "https://example.invalid/v1beta/models/model:generateContent"
        );

        let mut bedrock_url =
            "https://bedrock.example/model/model/invoke-with-response-stream".to_owned();
        let mut bedrock_framing = SingleAttemptFraming::AwsEventStream;
        let mut bedrock_body = json!({});
        configure_non_streaming_attempt(
            ChatProtocol::Bedrock,
            &mut bedrock_url,
            &mut bedrock_framing,
            &mut bedrock_body,
        )
        .unwrap();
        assert_eq!(bedrock_url, "https://bedrock.example/model/model/invoke");
        assert_eq!(bedrock_framing, SingleAttemptFraming::Sse);
    }

    #[test]
    fn chat_protocol_auth_validation_uses_the_manifest_contract() {
        assert!(validate_protocol_auth("openai.chat_text", &AuthScheme::Bearer).is_ok());
        assert!(
            validate_protocol_auth(
                "openai.chat_text",
                &AuthScheme::HeaderKey("x-api-key".into())
            )
            .is_err()
        );
        assert!(
            validate_protocol_auth(
                "anthropic.messages",
                &AuthScheme::HeaderKey("X-API-KEY".into())
            )
            .is_ok()
        );
        assert!(
            validate_protocol_auth("gemini.generate_text", &AuthScheme::Bearer).is_err()
        );
        assert!(
            validate_protocol_auth("bedrock.anthropic_messages", &AuthScheme::Bedrock).is_ok()
        );
        assert!(
            validate_protocol_auth("bedrock.anthropic_messages", &AuthScheme::Bearer).is_err()
        );
        assert!(
            validate_protocol_auth("vertex.anthropic_messages", &AuthScheme::Bedrock).is_err()
        );
    }

    #[test]
    fn invalid_chat_auth_is_rejected_before_registry_mutation() {
        let route = convert_chat_route_candidate(&candidate()).unwrap();
        let registry = ConnectionCredentialLeaseRegistry::default();
        let error = register_route_credential(
            &registry,
            &route,
            "header_key:x-api-key".to_owned(),
            "encrypted-material".to_owned(),
        )
        .unwrap_err();

        assert_eq!(error, ProductionRepositoryError::InvalidData);
        assert!(registry.credentials.read().unwrap().is_empty());
    }

    #[tokio::test]
    async fn route_lookup_uses_the_complete_preset_revision_identity() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let pool = database.pool().clone();

        for (preset, revision, provider, model) in [
            ("preset-a", "preset-a@1", "provider-a", "model-a"),
            ("preset-b", "preset-b@1", "provider-b", "model-b"),
        ] {
            sqlx::query(
                "INSERT INTO agent_presets \
                 (preset_id, owner_ref_json, source_json, display_json, \
                  current_stable_revision, created_at) \
                 VALUES (?, '{}', '{}', '{}', 1, 0)",
            )
            .bind(preset)
            .execute(&pool)
            .await
            .unwrap();
            let mut route = candidate();
            route.provider_id = provider.to_owned();
            route.model = model.to_owned();
            let record = ChatRouteRecord {
                schema: ChatRouteRecordSchema::V1,
                task: ChatRouteTask::AgentChat,
                primary: route,
                failovers: Vec::new(),
            };
            sqlx::query(
                "INSERT INTO agent_preset_revisions \
                 (revision_id, preset_id, revision_no, schema_version, \
                  payload_json, revision_digest, created_by, created_at, reason) \
                 VALUES (?, ?, 1, '1.0.0', ?, ?, 'owner', 0, '')",
            )
            .bind(revision)
            .bind(preset)
            .bind(
                serde_json::to_string(&revision_payload(record))
                    .expect("serialize canonical preset revision payload"),
            )
            .bind(if preset == "preset-a" {
                "a".repeat(64)
            } else {
                "b".repeat(64)
            })
            .execute(&pool)
            .await
            .unwrap();
        }

        let repository = ProductionModelRepository::new(pool.clone());
        let resolved = repository
            .resolve_route_record(&ChatRouteIdentity::new(
                "preset-b@1",
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
                "opaque-route".into(),
                7,
            ))
            .await
            .unwrap();
        assert_eq!(resolved.primary.provider_id.as_ref(), "provider-b");
        assert_eq!(resolved.primary.model, "model-b");

        for invalid_identity in [
            ChatRouteIdentity::new(
                "preset-b@1",
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
                "wrong-route".into(),
                7,
            ),
            ChatRouteIdentity::new(
                "preset-b@1",
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
                "opaque-route".into(),
                8,
            ),
            ChatRouteIdentity::new("preset-b@1", "unsupported_task", "opaque-route".into(), 7),
        ] {
            let error = repository
                .resolve_route_record(&invalid_identity)
                .await
                .unwrap_err();
            assert!(matches!(error, ChatBrokerHostError::InvalidRouteRecord(_)));
        }

        let missing = repository
            .resolve_route_record(&ChatRouteIdentity::new(
                "preset-c@1",
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
                "opaque-route".into(),
                7,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            missing,
            ChatBrokerHostError::RouteRecordMissing { .. }
        ));
        pool.close().await;
    }

    #[tokio::test]
    async fn explicit_unsupported_feature_updates_database_and_shared_cache() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let pool = database.pool().clone();
        let provider_id = "0190f5fe-7c00-7a00-8000-000000000099";
        sqlx::query(
            "INSERT INTO providers \
             (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, \
              enabled, config_revision, created_at, updated_at) \
             VALUES (?, 'custom', 'Observed provider', 'https://example.invalid/v1', \
                     'bearer', 'encrypted', 1, 0, 0, 0)",
        )
        .bind(provider_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_models \
             (provider_id, model, enabled, sort_order, created_at, updated_at) \
             VALUES (?, 'observed-model', 1, 0, 0, 0)",
        )
        .bind(provider_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_model_capabilities \
             (provider_id, model, task, traits, protocol, connection_role, provider_params, \
              created_at, updated_at) \
             VALUES (?, 'observed-model', 'chat', '[]', 'openai.chat_text', 'default', '{}', 0, 0)",
        )
        .bind(provider_id)
        .execute(&pool)
        .await
        .unwrap();

        let provider_ref = ProviderIdRef::from(provider_id);
        let digest = provider_config_digest(&pool, &provider_ref).await.unwrap();
        let route = ResolvedChatRoute {
            model_route_id: "observed-route".into(),
            model_route_revision: 1,
            provider_id: provider_ref,
            model: "observed-model".into(),
            protocol: ChatProtocol::OpenaiChat,
            connection_config_ref: "default".into(),
            config_revision_digest: digest,
            credential_ref: "observed-credential".into(),
            features: BTreeSet::from([
                ChatModelFeature::TextInput,
                ChatModelFeature::TextOutput,
                ChatModelFeature::ToolCalls,
                ChatModelFeature::Reasoning,
                ChatModelFeature::Streaming,
            ]),
            activation_features: BTreeSet::new(),
        };
        let cache = ModelTechnicalCapabilityCache::default();
        let observer = ProductionChatCapabilityObserver::new(pool.clone(), cache.clone());
        observer
            .record_unsupported(&route, ChatModelFeature::ToolCalls)
            .await;

        let stored: String = sqlx::query_scalar(
            "SELECT health FROM provider_model_capabilities \
             WHERE provider_id = ? AND model = 'observed-model' AND task = 'chat'",
        )
        .bind(provider_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let health: CapabilityHealth = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            health.unsupported_technical_capabilities,
            vec![ModelTechnicalCapability::FunctionCalling]
        );
        let (_, cached) = cache
            .unsupported_for(&pool, provider_id, "observed-model")
            .await
            .unwrap();
        assert!(cached.contains(&ModelTechnicalCapability::FunctionCalling));

        let repository = ProductionModelRepository::with_technical_capabilities(
            pool.clone(),
            cache.clone(),
        );
        let overlaid = repository
            .apply_technical_capabilities(ResolvedChatRouteSet {
                primary: route.clone(),
                failovers: Vec::new(),
            })
            .await
            .unwrap();
        assert!(!overlaid.primary.features.contains(&ChatModelFeature::ToolCalls));
        assert!(overlaid.primary.features.contains(&ChatModelFeature::Reasoning));
        assert!(overlaid.primary.features.contains(&ChatModelFeature::Streaming));

        sqlx::query(
            "UPDATE providers SET config_revision = 1, \
             base_url = 'https://replacement.example.invalid/v1' WHERE provider_id = ?",
        )
            .bind(provider_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE provider_model_capabilities SET health = NULL, health_checked_at = NULL \
             WHERE provider_id = ? AND model = 'observed-model' AND task = 'chat'",
        )
        .bind(provider_id)
        .execute(&pool)
        .await
        .unwrap();
        observer
            .record_unsupported(&route, ChatModelFeature::Reasoning)
            .await;
        let stale_health: Option<String> = sqlx::query_scalar(
            "SELECT health FROM provider_model_capabilities \
             WHERE provider_id = ? AND model = 'observed-model' AND task = 'chat'",
        )
        .bind(provider_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            stale_health.is_none(),
            "an old route response cannot downgrade a newer provider revision"
        );
        let (_, refreshed) = cache
            .unsupported_for(&pool, provider_id, "observed-model")
            .await
            .unwrap();
        assert!(
            refreshed.is_empty(),
            "a provider revision change invalidates the negative cache"
        );
    }
}
