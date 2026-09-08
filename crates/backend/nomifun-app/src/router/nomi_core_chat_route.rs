//! Host-owned materialization of the initial Nomi-core Chat route.
//!
//! Agent Settings stores a complete opaque route record in the immutable
//! preset revision.  The renderer may choose among the candidates returned by
//! this materializer, but it must not derive route ids, credential references,
//! or provider configuration digests itself.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use axum::http::StatusCode;
use nomifun_agent_contracts::{
    ChatRouteCandidate, ChatRouteFeature, ChatRouteProtocol, ChatRouteRecord,
    ChatRouteRecordSchema, ChatRouteTask, ConnectionConfigRef, DigestHex, ModelRouteId, UserId,
};
use nomifun_agent_control_plane::{ControlPlaneError, DefaultChatRouteResolver};
use nomifun_api_types::ModelTrait;
use nomifun_chat_model_broker::ProviderIdRef;
use nomifun_db::{
    IProviderConnectionRepository, IProviderModelCapabilityRepository, IProviderModelRepository,
    IProviderRepository, SqlitePool, SqliteProviderConnectionRepository,
    SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
    SqliteProviderRepository,
};
use uuid::Uuid;

use super::chat_broker_host::provider_config_digest;

const MAX_DEFAULT_CHAT_CANDIDATES: usize = 32;

/// Materializes enabled provider/model Chat capabilities into one immutable
/// route record.  This object owns no credentials; it only reads the provider
/// graph and emits non-secret route metadata.
#[derive(Clone)]
pub(crate) struct NomiCoreDefaultChatRouteResolver {
    pool: SqlitePool,
}

impl NomiCoreDefaultChatRouteResolver {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DefaultChatRouteResolver for NomiCoreDefaultChatRouteResolver {
    async fn resolve_default_chat_route(
        &self,
        _owner: &UserId,
    ) -> Result<Option<ChatRouteRecord>, ControlPlaneError> {
        self.resolve_route(None).await
    }

    async fn resolve_selected_chat_route(
        &self,
        _owner: &UserId,
        model: &nomifun_api_types::AgentChatModelSelectionDto,
    ) -> Result<Option<ChatRouteRecord>, ControlPlaneError> {
        self.resolve_route(Some(model)).await
    }
}

impl NomiCoreDefaultChatRouteResolver {
    async fn resolve_route(
        &self,
        selected: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> Result<Option<ChatRouteRecord>, ControlPlaneError> {
        let providers = SqliteProviderRepository::new(self.pool.clone())
            .list()
            .await
            .map_err(|_| unavailable())?;
        let models = SqliteProviderModelRepository::new(self.pool.clone())
            .list()
            .await
            .map_err(|_| unavailable())?;
        let capabilities = SqliteProviderModelCapabilityRepository::new(self.pool.clone())
            .list()
            .await
            .map_err(|_| unavailable())?;
        let provider_order = providers
            .iter()
            .filter(|provider| provider.enabled)
            .map(|provider| {
                (
                    provider.provider_id.clone(),
                    (provider.sort_order, provider.provider_id.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if provider_order.is_empty() {
            return Ok(None);
        }

        let connection_repo = SqliteProviderConnectionRepository::new(self.pool.clone());
        let mut connection_ids = BTreeMap::new();
        for provider_id in provider_order.keys() {
            for connection in connection_repo
                .list_for_provider(provider_id)
                .await
                .map_err(|_| unavailable())?
            {
                connection_ids.insert(
                    (connection.provider_id, connection.role),
                    connection.connection_id,
                );
            }
        }

        let model_order = models
            .into_iter()
            .filter(|model| model.enabled)
            .map(|model| {
                (
                    (model.provider_id.clone(), model.model.clone()),
                    (model.sort_order, model.model),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut digest_by_provider = BTreeMap::<String, DigestHex>::new();
        let mut rows = capabilities
            .into_iter()
            .filter(|capability| capability.task == "chat")
            .filter(|capability| selected.is_none_or(|model|
                capability.provider_id == model.provider_id && capability.model == model.model))
            .filter_map(|capability| {
                let provider_key = provider_order.get(&capability.provider_id)?;
                let model_key = (
                    capability.provider_id.clone(),
                    capability.model.clone(),
                );
                let model_key_order = model_order.get(&model_key)?;
                let protocol = protocol_for(&capability.protocol)?;
                let connection_config_ref = if capability.connection_role == "default" {
                    ConnectionConfigRef::from("default")
                } else {
                    ConnectionConfigRef::from(
                        connection_ids
                            .get(&(capability.provider_id.clone(), capability.connection_role.clone()))?
                            .clone(),
                    )
                };
                Some((
                    (
                        provider_key.0,
                        provider_key.1.clone(),
                        model_key_order.0,
                        model_key_order.1.clone(),
                        capability.provider_id.clone(),
                        capability.model.clone(),
                    ),
                    capability,
                    protocol,
                    connection_config_ref,
                ))
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.0.cmp(&right.0));

        let mut candidates = Vec::with_capacity(rows.len().min(MAX_DEFAULT_CHAT_CANDIDATES));
        for (_, capability, protocol, connection_config_ref) in
            rows.into_iter().take(MAX_DEFAULT_CHAT_CANDIDATES)
        {
            let config_revision_digest = match digest_by_provider
                .entry(capability.provider_id.clone())
            {
                std::collections::btree_map::Entry::Occupied(entry) => entry.get().clone(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let digest = provider_config_digest(
                        &self.pool,
                        &ProviderIdRef::from(capability.provider_id.clone()),
                    )
                    .await
                    .map_err(|_| unavailable())?;
                    entry.insert(digest.clone());
                    digest
                }
            };
            let features = features_for(&capability.traits)?;
            candidates.push(ChatRouteCandidate {
                model_route_id: ModelRouteId::from(Uuid::now_v7().to_string()),
                model_route_revision: 1,
                provider_id: capability.provider_id,
                model: capability.model,
                protocol,
                connection_config_ref,
                config_revision_digest,
                credential_ref: format!("nomi-core-chat-credential-{}", Uuid::now_v7()),
                features,
            });
        }

        let mut candidate_iter = candidates.into_iter();
        let Some(primary) = candidate_iter.next() else {
            return Ok(None);
        };
        let failovers = candidate_iter
            .filter(|candidate| {
                candidate.model_route_id != primary.model_route_id
                    || candidate.model_route_revision != primary.model_route_revision
            })
            .collect::<Vec<_>>();
        let record = ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary,
            failovers,
        };
        record.validate().map_err(|error| {
            ControlPlaneError::canonical(
                "MODEL_ROUTE_RECORD_INVALID",
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("host Chat route materialization failed: {error}"),
            )
        })?;
        Ok(Some(record))
    }
}

fn protocol_for(value: &str) -> Option<ChatRouteProtocol> {
    match value {
        "anthropic.messages" => Some(ChatRouteProtocol::Anthropic),
        "openai.chat_text" => Some(ChatRouteProtocol::OpenaiChat),
        "openai.responses" => Some(ChatRouteProtocol::OpenaiResponses),
        "gemini.generate_text" => Some(ChatRouteProtocol::Gemini),
        "bedrock.anthropic_messages" => Some(ChatRouteProtocol::Bedrock),
        "vertex.anthropic_messages" => Some(ChatRouteProtocol::Vertex),
        _ => None,
    }
}

fn features_for(raw: &str) -> Result<BTreeSet<ChatRouteFeature>, ControlPlaneError> {
    let traits: Vec<ModelTrait> = serde_json::from_str(raw).map_err(|_| {
        ControlPlaneError::canonical(
            "MODEL_ROUTE_RECORD_INVALID",
            StatusCode::INTERNAL_SERVER_ERROR,
            "a configured Chat capability has invalid trait metadata",
        )
    })?;
    let mut features = BTreeSet::from([
        ChatRouteFeature::TextInput,
        ChatRouteFeature::TextOutput,
    ]);
    for model_trait in traits {
        let feature = match model_trait {
            ModelTrait::VisionInput => ChatRouteFeature::ImageInput,
            ModelTrait::FunctionCalling => ChatRouteFeature::ToolCalls,
            ModelTrait::Reasoning => ChatRouteFeature::Reasoning,
            ModelTrait::WebSearch => continue,
            ModelTrait::AudioInput => ChatRouteFeature::AudioInput,
            ModelTrait::AudioOutput => ChatRouteFeature::AudioOutput,
            ModelTrait::VideoInput => continue,
            ModelTrait::Realtime => ChatRouteFeature::ProviderRoundState,
            ModelTrait::Streaming => continue,
        };
        features.insert(feature);
    }
    Ok(features)
}

fn unavailable() -> ControlPlaneError {
    ControlPlaneError::canonical(
        "MODEL_ROUTE_NOT_FOUND",
        StatusCode::SERVICE_UNAVAILABLE,
        "the host Chat model catalog is temporarily unavailable",
    )
}
