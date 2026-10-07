//! Additive voice-only catalog/credential lease resolution. Ordinary task,
//! batch and legacy realtime resolution remain owned by their existing path.
use crate::{
    AuthMaterial, AuthScheme, InvokeError, InvokeErrorKind, ModelInvokeService, ModelRef,
    ResolvedConnection, ResolvedTaskConfig, ResolvedTaskTransport,
};
use nomifun_api_types::{ModelTask, parse_persisted_model_traits};
use nomifun_common::ProviderId;
use serde_json::{Value, json};

/// Opaque lease for a server-owned VoiceProfile. No task protocol, route
/// default or Main capability is created by reading it. Deliberately not
/// Debug because its connection holds decrypted authentication material.
#[derive(Clone)]
pub struct VoiceConnectionLease {
    pub provider_id: String,
    pub model: String,
    pub config_revision: i64,
    pub connection: ResolvedConnection,
    pub bedrock_config: Option<String>,
}

fn catalog(what: &str, error: nomifun_db::DbError) -> InvokeError {
    InvokeError::catalog(format!("{what}: {error}"))
}
fn object(raw: &str, field: &str) -> Result<Value, InvokeError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| InvokeError::config(format!("{field} must be a JSON object")))?;
    if !value.is_object() {
        return Err(InvokeError::config(format!(
            "{field} must be a JSON object"
        )));
    }
    Ok(value)
}
fn credentials(encrypted: &str, key: &[u8; 32]) -> Result<Value, InvokeError> {
    let plain = nomifun_common::decrypt_string(encrypted, key)
        .map_err(|_| InvokeError::config("voice credential lease could not be decrypted"))?;
    object(&plain, "voice credentials")
}
impl ModelInvokeService {
    pub async fn resolve_voice_connection_lease(
        &self,
        model: &ModelRef,
        role: &str,
        expected_config_revision: Option<i64>,
    ) -> Result<VoiceConnectionLease, InvokeError> {
        let id = ProviderId::parse(&model.provider_id).map_err(|_| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "voice provider_id must be canonical",
            )
        })?;
        let role = role.trim();
        if role.is_empty() {
            return Err(InvokeError::config(
                "voice connection role must be explicit",
            ));
        }
        for _ in 0..3 {
            let provider = self
                .provider_repo
                .find_by_id(id.as_str())
                .await
                .map_err(|e| catalog("voice provider read failed", e))?
                .ok_or_else(|| InvokeError::config("voice provider is not configured"))?;
            if !provider.enabled {
                return Err(InvokeError::config("voice provider is disabled"));
            }
            if expected_config_revision.is_some_and(|expected| expected != provider.config_revision)
            {
                return Err(InvokeError::config(
                    "voice provider configuration changed after profile resolution",
                ));
            }
            let result = async {
                let row = self
                    .provider_model_repo
                    .get(id.as_str(), &model.model)
                    .await
                    .map_err(|e| catalog("voice model read failed", e))?
                    .ok_or_else(|| {
                        InvokeError::new(
                            InvokeErrorKind::UnsupportedTask,
                            "voice model is not in the catalog; add it through ModelHub explicitly",
                        )
                    })?;
                if !row.enabled {
                    return Err(InvokeError::new(
                        InvokeErrorKind::UnsupportedTask,
                        "voice model is disabled",
                    ));
                }
                let mut connection = if role == "default" {
                    ResolvedConnection {
                        role: role.into(),
                        base_url: provider.base_url.trim().trim_end_matches('/').into(),
                        auth: AuthMaterial {
                            scheme: AuthScheme::parse(&provider.auth_scheme)?,
                            credentials: credentials(
                                &provider.credentials_encrypted,
                                &self.encryption_key,
                            )?,
                        },
                        extra: json!({}),
                    }
                } else {
                    let connection = self
                        .provider_connection_repo
                        .get(id.as_str(), role)
                        .await
                        .map_err(|e| catalog("voice connection read failed", e))?
                        .ok_or_else(|| {
                            InvokeError::new(
                                InvokeErrorKind::MissingConnection,
                                "voice connection role is not configured",
                            )
                        })?;
                    ResolvedConnection {
                        role: connection.role,
                        base_url: connection.base_url.trim().trim_end_matches('/').into(),
                        auth: AuthMaterial {
                            scheme: AuthScheme::parse(&connection.auth_scheme)?,
                            credentials: credentials(
                                &connection.credentials_encrypted,
                                &self.encryption_key,
                            )?,
                        },
                        extra: object(&connection.extra, "voice connection extra")?,
                    }
                };
                connection
                    .extra
                    .as_object_mut()
                    .expect("strict extra")
                    .insert(
                        "nomifun_voice_credential_origin".into(),
                        json!(connection.base_url),
                    );
                Ok(VoiceConnectionLease {
                    provider_id: provider.provider_id.clone(),
                    model: model.model.clone(),
                    config_revision: provider.config_revision,
                    connection,
                    bedrock_config: provider.bedrock_config.clone(),
                })
            }
            .await;
            let current = self
                .provider_repo
                .find_by_id(id.as_str())
                .await
                .map_err(|e| catalog("voice provider revision verification failed", e))?;
            if current.is_some_and(|current| {
                current.enabled && current.config_revision == provider.config_revision
            }) {
                return result;
            }
        }
        Err(InvokeError::config(
            "voice connection graph changed repeatedly during lease resolution",
        ))
    }
    /// Read one explicit realtime capability without choosing its transport.
    /// A registered factory must validate the opaque lease, URL and required
    /// authentication before use. This does not admit legacy realtime calls.
    pub async fn resolve_voice_task_config(
        &self,
        model: &ModelRef,
        expected_revision: Option<i64>,
    ) -> Result<ResolvedTaskConfig, InvokeError> {
        let id = ProviderId::parse(&model.provider_id).map_err(|_| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "voice provider_id must be canonical",
            )
        })?;
        for _ in 0..3 {
            let provider = self
                .provider_repo
                .find_by_id(id.as_str())
                .await
                .map_err(|e| catalog("voice provider read failed", e))?
                .ok_or_else(|| InvokeError::config("voice provider is not configured"))?;
            if !provider.enabled {
                return Err(InvokeError::config("voice provider is disabled"));
            }
            if expected_revision.is_some_and(|expected| expected != provider.config_revision) {
                return Err(InvokeError::config(
                    "voice provider configuration changed after route resolution",
                ));
            }
            let result = async {
                let row = self
                    .provider_model_repo
                    .get(id.as_str(), &model.model)
                    .await
                    .map_err(|e| catalog("voice model read failed", e))?
                    .ok_or_else(|| {
                        InvokeError::new(
                            InvokeErrorKind::UnsupportedTask,
                            "voice model is not in the catalog",
                        )
                    })?;
                if !row.enabled {
                    return Err(InvokeError::new(
                        InvokeErrorKind::UnsupportedTask,
                        "voice model is disabled",
                    ));
                }
                let capability = self
                    .provider_model_capability_repo
                    .get(id.as_str(), &model.model, "realtime_conversation")
                    .await
                    .map_err(|e| catalog("voice capability read failed", e))?
                    .ok_or_else(|| {
                        InvokeError::new(
                            InvokeErrorKind::UnsupportedTask,
                            "voice model has no explicit realtime capability",
                        )
                    })?;
                let protocol = capability.protocol.trim().to_owned();
                let role = capability.connection_role.trim();
                if protocol.is_empty() || role.is_empty() {
                    return Err(InvokeError::config(
                        "voice capability requires explicit protocol and connection role",
                    ));
                }
                let params = object(&capability.provider_params, "voice provider params")?;
                if params
                    .as_object()
                    .expect("strict object")
                    .keys()
                    .any(|key| crate::adapters::is_reserved_local_transport_param_key(key))
                {
                    return Err(InvokeError::config(
                        "voice provider params contain local transport or credential fields",
                    ));
                }
                let mut connection = if role == "default" {
                    ResolvedConnection {
                        role: "default".into(),
                        base_url: provider.base_url.trim().trim_end_matches('/').into(),
                        auth: AuthMaterial {
                            scheme: AuthScheme::parse(&provider.auth_scheme)?,
                            credentials: credentials(
                                &provider.credentials_encrypted,
                                &self.encryption_key,
                            )?,
                        },
                        extra: json!({}),
                    }
                } else {
                    let connection = self
                        .provider_connection_repo
                        .get(id.as_str(), role)
                        .await
                        .map_err(|e| catalog("voice connection read failed", e))?
                        .ok_or_else(|| {
                            InvokeError::new(
                                InvokeErrorKind::MissingConnection,
                                "voice connection role is not configured",
                            )
                        })?;
                    ResolvedConnection {
                        role: connection.role,
                        base_url: connection.base_url.trim().trim_end_matches('/').into(),
                        auth: AuthMaterial {
                            scheme: AuthScheme::parse(&connection.auth_scheme)?,
                            credentials: credentials(
                                &connection.credentials_encrypted,
                                &self.encryption_key,
                            )?,
                        },
                        extra: object(&connection.extra, "voice connection extra")?,
                    }
                };
                // Preserve the origin from which credentials were leased. A
                // cloud factory validates an override against this origin.
                connection
                    .extra
                    .as_object_mut()
                    .expect("strict extra")
                    .insert(
                        "nomifun_voice_credential_origin".into(),
                        json!(connection.base_url),
                    );
                if let Some(base) = &capability.base_url_override {
                    connection.base_url = base.trim().trim_end_matches('/').into();
                }
                let traits = parse_persisted_model_traits(&capability.traits)
                    .map_err(|_| InvokeError::config("voice capability traits are invalid"))?;
                if capability.context_limit.is_some_and(|v| v <= 0)
                    || capability.output_limit.is_some_and(|v| v <= 0)
                {
                    return Err(InvokeError::config(
                        "voice capability limits must be positive",
                    ));
                }
                Ok(ResolvedTaskConfig {
                    provider_id: provider.provider_id.clone(),
                    config_revision: provider.config_revision,
                    platform: provider.platform.clone(),
                    model: model.model.clone(),
                    task: ModelTask::RealtimeConversation,
                    traits,
                    protocol,
                    connection,
                    provider_params: params,
                    transport: ResolvedTaskTransport {
                        endpoint: capability.endpoint,
                        poll_endpoint: capability.poll_endpoint,
                        content_endpoint: capability.content_endpoint,
                        realtime_endpoint: capability.realtime_endpoint,
                        allow_cross_origin_credentials: capability.allow_cross_origin_credentials,
                    },
                    context_limit: capability.context_limit,
                    output_limit: capability.output_limit,
                    bedrock_config: provider.bedrock_config.clone(),
                })
            }
            .await;
            let current = self
                .provider_repo
                .find_by_id(id.as_str())
                .await
                .map_err(|e| catalog("voice provider revision verification failed", e))?;
            if current.is_some_and(|current| {
                current.enabled && current.config_revision == provider.config_revision
            }) {
                return result;
            }
        }
        Err(InvokeError::config(
            "voice invocation graph changed repeatedly during lease resolution",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{
        CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
        SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
        SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
    };
    use std::sync::Arc;
    #[tokio::test]
    async fn profile_connection_lease_reads_catalog_without_creating_realtime_capability() {
        let key = [0x59; 32];
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let providers = Arc::new(SqliteProviderRepository::new(pool.clone()));
        let id = "0190f5fe-7c00-7a00-8000-000000000282";
        let capabilities = [NewProviderModelCapability {
            task: "chat",
            traits: "[]",
            protocol: "openai.chat_text",
            connection_role: "default",
            provider_params: "{}",
            ..Default::default()
        }];
        let encrypted =
            nomifun_common::encrypt_string(r#"{"api_keys":["test-only-lease-key"]}"#, &key)
                .unwrap();
        providers
            .create(
                CreateProviderParams {
                    provider_id: Some(id),
                    platform: "custom",
                    name: "existing ModelHub model",
                    base_url: "https://voice.invalid/v1",
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypted,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: None,
                },
                &NewProviderModel {
                    model: "gpt-live-1",
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &capabilities,
                },
                &[],
            )
            .await
            .unwrap();
        let revision = providers
            .find_by_id(id)
            .await
            .unwrap()
            .unwrap()
            .config_revision;
        let capabilities = Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
        let service = ModelInvokeService::new(
            providers.clone(),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            capabilities.clone(),
            Arc::new(SqliteProviderConnectionRepository::new(pool)),
            key,
            reqwest::Client::new(),
            crate::AdapterRegistry::new(crate::default_adapters()),
        );
        let model = ModelRef {
            provider_id: id.into(),
            model: "gpt-live-1".into(),
        };
        let lease = service
            .resolve_voice_connection_lease(&model, "default", Some(revision))
            .await
            .unwrap();
        assert_eq!(lease.config_revision, revision);
        assert_eq!(lease.connection.role, "default");
        assert!(
            service
                .resolve_voice_task_config(&model, Some(revision))
                .await
                .is_err(),
            "legacy RT configuration remains absent"
        );
        use nomifun_db::IProviderModelCapabilityRepository;
        assert!(
            capabilities
                .get(id, "gpt-live-1", "realtime_conversation")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            providers
                .find_by_id(id)
                .await
                .unwrap()
                .unwrap()
                .config_revision,
            revision
        );
        let main = service
            .resolve_task_config(&model, ModelTask::Chat)
            .await
            .unwrap();
        assert_eq!(main.protocol, "openai.chat_text");
        assert!(
            service
                .resolve_voice_connection_lease(&model, "missing-role", Some(revision))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn opaque_voice_lease_does_not_change_ordinary_or_legacy_validation() {
        let key = [0x58; 32];
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let providers = Arc::new(SqliteProviderRepository::new(pool.clone()));
        let id = "0190f5fe-7c00-7a00-8000-000000000281";
        let capabilities = [NewProviderModelCapability {
            task: "realtime_conversation",
            traits: "[]",
            protocol: "test.local_voice",
            connection_role: "default",
            provider_params: "{}",
            ..Default::default()
        }];
        let encrypted = nomifun_common::encrypt_string(r#"{"api_keys":[]}"#, &key).unwrap();
        providers
            .create(
                CreateProviderParams {
                    provider_id: Some(id),
                    platform: "custom",
                    name: "opaque local voice",
                    base_url: "",
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypted,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: None,
                },
                &NewProviderModel {
                    model: "explicit-local-model",
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &capabilities,
                },
                &[],
            )
            .await
            .unwrap();
        let revision = providers
            .find_by_id(id)
            .await
            .unwrap()
            .unwrap()
            .config_revision;
        let service = ModelInvokeService::new(
            providers,
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool)),
            key,
            reqwest::Client::new(),
            crate::AdapterRegistry::new(crate::default_adapters()),
        );
        let model = ModelRef {
            provider_id: id.into(),
            model: "explicit-local-model".into(),
        };
        let lease = service
            .resolve_voice_task_config(&model, Some(revision))
            .await
            .unwrap();
        assert_eq!(lease.connection.base_url, "");
        assert_eq!(lease.protocol, "test.local_voice");
        assert_eq!(
            lease.connection.extra["nomifun_voice_credential_origin"],
            ""
        );
        assert!(
            service
                .resolve_task_config(&model, ModelTask::RealtimeConversation)
                .await
                .is_err(),
            "ordinary resolution still enforces its original registered protocol/auth contract"
        );
        assert!(
            service
                .resolve_voice_task_config(&model, Some(revision + 1))
                .await
                .is_err()
        );
        assert!(
            super::super::create_openai_live_model_port(
                lease.connection,
                json!({}),
                "gpt-live-1".into()
            )
            .is_err(),
            "cloud constructor cannot use a credentialless opaque lease"
        );
    }
}
