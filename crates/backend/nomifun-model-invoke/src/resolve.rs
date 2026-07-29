//! The catalog resolution pipeline: `(ModelRef, ModelTask, TaskRequest)` →
//! [`ResolvedCall`] + protocol adapter. This is THE single resolution
//! algorithm — invoke, poll and probe all ride it; no other code path
//! may combine providers/provider_models/provider_connections rows with the
//! platform route table.

use std::sync::Arc;

use nomifun_api_types::{ModelTask, derive_tasks_and_traits};
use nomifun_common::ProviderId;
use nomifun_db::models::Provider;

use crate::adapter::ProtocolAdapter;
use crate::auth::{AuthMaterial, AuthScheme};
use crate::call::{ResolvedCall, ResolvedConnection};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::routes_table::platform_route;
use crate::service::ModelInvokeService;
use crate::types::{ModelRef, TaskRequest};

/// Auth scheme applied when a call rides the DEFAULT connection (the
/// `providers` row itself, which only stores a bare key): most platforms take
/// `Authorization: Bearer`, but gemini wants its key in `x-goog-api-key` and
/// deepgram wants `Authorization: Token`. Named connection profiles declare
/// their own scheme and never consult this table.
fn default_connection_scheme(platform: &str) -> AuthScheme {
    match platform {
        "gemini" => AuthScheme::HeaderKey("x-goog-api-key".into()),
        "deepgram" => AuthScheme::TokenHeader,
        _ => AuthScheme::Bearer,
    }
}

/// Row `tasks` JSON → declared tasks; bad JSON is treated as "unseeded"
/// (empty), which step 3 then resolves via the name/platform heuristic.
fn parse_tasks(raw: &str) -> Vec<ModelTask> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Tolerant JSON-object parse for `provider_models.params` and
/// `provider_connections.extra`: anything that is not a JSON object → `{}`.
fn parse_json_object(raw: &str) -> serde_json::Value {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) if v.is_object() => v,
        _ => serde_json::json!({}),
    }
}

/// A catalog (local DB) read failure. Classified as [`InvokeErrorKind::Config`]:
/// it is a local problem, must not be mistaken for an upstream provider
/// failure, and must stay inert for error-driven self-healing loops
/// (key rotation / failover branch on Auth/RateLimited/Network/...).
fn catalog_err(what: &str, e: nomifun_db::DbError) -> InvokeError {
    InvokeError::config(format!("{what}: {e}"))
}

impl ModelInvokeService {
    /// Resolve one task invocation against the catalog. The single algorithm
    /// (documented steps 1-6 below, in order); `enforce_task_membership =
    /// false` is the probe path — probing an untagged model with an explicit
    /// task must not be blocked by catalog membership.
    ///
    /// 1. Provider: `ProviderId::parse` → InvalidParams; row absent →
    ///    Config("provider not found"); `!enabled` → Config("provider
    ///    disabled").
    /// 2. Model row `provider_models(provider_id, model)`: absent → enforce ?
    ///    UnsupportedTask("model not in catalog") : tolerated (tasks treated
    ///    as empty); present but disabled → UnsupportedTask("model disabled").
    /// 3. Task gate (enforce only): declared tasks JSON (bad → empty); task ∉
    ///    non-empty declared → UnsupportedTask; declared empty (unseeded) →
    ///    fall back to `derive_tasks_and_traits(platform, model)`, still
    ///    absent → UnsupportedTask.
    /// 4. protocol = row.protocol (non-empty) ?? `platform_route(platform,
    ///    task).protocol`; role = row.connection_role ?? route role.
    /// 5. Connection: no role → the provider row IS the "default" connection
    ///    (bedrock guarded off; first comma/newline-separated key; scheme per
    ///    [`default_connection_scheme`]); role → `provider_connections` row
    ///    (absent → MissingConnection) with decrypted credentials and its
    ///    declared auth scheme.
    /// 6. model_params = row.params (bad → {}); adapter = registry[(protocol,
    ///    task)] — an *unregistered* protocol coming from the model-row
    ///    override is user-fixable model config → Config, not NoAdapter.
    pub(crate) async fn resolve(
        &self,
        m: &ModelRef,
        task: ModelTask,
        request: TaskRequest,
        enforce_task_membership: bool,
    ) -> Result<(ResolvedCall, Arc<dyn ProtocolAdapter>), InvokeError> {
        // -- 1. Provider row + enabled gate --------------------------------
        let provider_id = ProviderId::parse(m.provider_id.as_str()).map_err(|e| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!("provider_id {:?} is not a canonical ProviderId: {e}", m.provider_id),
            )
        })?;
        let provider = self
            .provider_repo
            .find_by_id(provider_id.as_str())
            .await
            .map_err(|e| catalog_err("failed to read provider", e))?
            .ok_or_else(|| InvokeError::config(format!("provider not found: {provider_id}")))?;
        if !provider.enabled {
            return Err(InvokeError::config(format!("provider disabled: {}", provider.name)));
        }

        // -- 2. Model row (probe tolerates absence: tasks treated as empty) --
        let row = self
            .provider_model_repo
            .get(provider_id.as_str(), &m.model)
            .await
            .map_err(|e| catalog_err("failed to read model row", e))?;
        let row = match row {
            Some(row) if !row.enabled => {
                return Err(InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("model disabled: {}", m.model),
                ));
            }
            Some(row) => Some(row),
            None if enforce_task_membership => {
                return Err(InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("model not in catalog: {}", m.model),
                ));
            }
            None => None,
        };

        // -- 3. Task membership gate (enforced path only) -------------------
        if enforce_task_membership {
            let declared = row.as_ref().map(|r| parse_tasks(&r.tasks)).unwrap_or_default();
            let allowed = if declared.is_empty() {
                // Unseeded row: fall back to the name/platform heuristic.
                derive_tasks_and_traits(&provider.platform, &m.model).0.contains(&task)
            } else {
                declared.contains(&task)
            };
            if !allowed {
                return Err(InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("model {:?} does not declare task {task:?}", m.model),
                ));
            }
        }

        // -- 4. Protocol + connection role: row override ?? route table -----
        let route = platform_route(&provider.platform, task);
        let row_protocol = row
            .as_ref()
            .and_then(|r| r.protocol.as_deref())
            .map(str::trim)
            .filter(|p| !p.is_empty());
        let protocol = row_protocol.unwrap_or(route.protocol);
        let role = row
            .as_ref()
            .and_then(|r| r.connection_role.as_deref())
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .or(route.connection_role);

        // -- 5. Connection material -----------------------------------------
        let connection = match role {
            None => self.default_connection(&provider)?,
            Some(role) => self.role_connection(provider_id.as_str(), role).await?,
        };

        // -- 6. Model params + adapter lookup --------------------------------
        let model_params = row.as_ref().map(|r| parse_json_object(&r.params)).unwrap_or_else(|| serde_json::json!({}));
        let adapter = self.registry.get(protocol, task).map_err(|e| {
            // Reviewer ruling (T1): an UNREGISTERED protocol coming from the
            // model-row override is user-fixable model configuration → Config
            // (400), not a NoAdapter 500. Route-table protocols that nobody
            // registered — and registered adapters lacking the task — stay
            // NoAdapter (genuine server wiring gap).
            if row_protocol == Some(protocol) && !self.registry.contains(protocol) {
                InvokeError::config(format!(
                    "model {:?} declares unknown protocol {protocol:?}; fix the model's protocol override",
                    m.model
                ))
            } else {
                e
            }
        })?;

        let call = ResolvedCall {
            provider_id: provider.provider_id,
            platform: provider.platform,
            model: m.model.clone(),
            task,
            connection,
            model_params,
            request,
        };
        Ok((call, adapter))
    }

    /// Step 5, no-role branch: the `providers` row itself is the "default"
    /// connection. Credentials = the first non-empty entry of the stored
    /// comma/newline-separated key list; scheme per
    /// [`default_connection_scheme`]. bedrock rides its own SigV4 config and
    /// is explicitly not served by the invoke layer in P1.
    fn default_connection(&self, provider: &Provider) -> Result<ResolvedConnection, InvokeError> {
        if provider.platform == "bedrock" {
            return Err(InvokeError::config("bedrock is not supported by the invoke layer yet"));
        }
        let decrypted = nomifun_common::decrypt_string(&provider.api_key_encrypted, &self.encryption_key)
            .map_err(|e| InvokeError::config(format!("failed to decrypt provider api key: {e}")))?;
        let first_key = decrypted
            .split([',', '\n'])
            .map(str::trim)
            .find(|k| !k.is_empty())
            .unwrap_or_default()
            .to_string();
        Ok(ResolvedConnection {
            role: "default".into(),
            base_url: provider.base_url.clone(),
            is_full_url: provider.is_full_url,
            auth: AuthMaterial {
                scheme: default_connection_scheme(&provider.platform),
                credentials: serde_json::json!({ "api_keys": [first_key] }),
            },
            extra: serde_json::json!({}),
        })
    }

    /// Step 5, role branch: the route (or model row) demands a named
    /// `provider_connections` profile; absence is the actionable
    /// MissingConnection error, presence yields decrypted credentials and the
    /// profile's declared auth scheme.
    async fn role_connection(&self, provider_id: &str, role: &str) -> Result<ResolvedConnection, InvokeError> {
        let row = self
            .provider_connection_repo
            .get(provider_id, role)
            .await
            .map_err(|e| catalog_err("failed to read connection profile", e))?
            .ok_or_else(|| {
                InvokeError::new(
                    InvokeErrorKind::MissingConnection,
                    format!("connection profile {role:?} is not configured for this provider，请在供应商连接档案中配置"),
                )
            })?;
        let decrypted = nomifun_common::decrypt_string(&row.credentials_encrypted, &self.encryption_key)
            .map_err(|e| InvokeError::config(format!("failed to decrypt connection {role:?} credentials: {e}")))?;
        let credentials: serde_json::Value = serde_json::from_str(&decrypted).map_err(|e| {
            InvokeError::config(format!("connection {role:?} credentials are not valid JSON: {e}"))
        })?;
        Ok(ResolvedConnection {
            role: row.role,
            base_url: row.base_url,
            is_full_url: row.is_full_url,
            auth: AuthMaterial { scheme: AuthScheme::parse(&row.auth_scheme)?, credentials },
            extra: parse_json_object(&row.extra),
        })
    }
}

#[cfg(test)]
mod tests {
    use nomifun_api_types::ModelTask;
    use nomifun_common::{encrypt_string, generate_id};
    use nomifun_db::{
        CreateProviderParams, IProviderConnectionRepository, IProviderModelRepository,
        IProviderRepository, NewProviderModel, ProviderModelUpdate,
        SqliteProviderConnectionRepository, SqliteProviderModelRepository,
        SqliteProviderRepository, SqlitePool, UpsertProviderConnectionParams,
        init_database_memory,
    };
    use serde_json::json;

    use super::*;
    use crate::adapter::AdapterRegistry;
    use crate::types::{AsrRequest, ChatTextRequest, ImageGenRequest, InputAsset};
    use crate::{TaskOutcome, TaskResult};

    const TEST_KEY: [u8; 32] = [0x42; 32];

    /// Minimal adapter with configurable id / supported tasks.
    struct FakeAdapter {
        id: &'static str,
        tasks: &'static [ModelTask],
    }

    #[async_trait::async_trait]
    impl ProtocolAdapter for FakeAdapter {
        fn id(&self) -> &'static str {
            self.id
        }
        fn supports(&self, task: ModelTask) -> bool {
            self.tasks.contains(&task)
        }
        async fn submit(
            &self,
            _http: &reqwest::Client,
            _call: &ResolvedCall,
        ) -> Result<TaskOutcome, InvokeError> {
            Ok(TaskOutcome::Done(TaskResult::Text("fake".into())))
        }
    }

    fn fake(id: &'static str, tasks: &'static [ModelTask]) -> Arc<dyn ProtocolAdapter> {
        Arc::new(FakeAdapter { id, tasks })
    }

    /// Registry covering the protocols exercised by these tests.
    fn full_registry() -> Vec<Arc<dyn ProtocolAdapter>> {
        use ModelTask::*;
        vec![
            fake("openai.images", &[ImageGeneration, ImageEdit]),
            fake("openai.chat_text", &[Chat]),
            fake("custom.images", &[ImageGeneration]),
            fake("volc.asr_file", &[SpeechRecognition]),
            fake("deepgram.listen", &[SpeechRecognition]),
            fake("gemini.generate_content", &[ImageGeneration, ImageEdit]),
        ]
    }

    async fn setup(adapters: Vec<Arc<dyn ProtocolAdapter>>) -> (ModelInvokeService, SqlitePool) {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        std::mem::forget(db);
        let service = ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
            reqwest::Client::new(),
            AdapterRegistry::new(adapters),
        );
        (service, pool)
    }

    /// Seed a provider row; api key ciphertext holds a comma/newline separated
    /// list whose first entry is blank — resolve must pick "sk-first".
    async fn seed_provider(pool: &SqlitePool, platform: &str, enabled: bool) -> String {
        let repo = SqliteProviderRepository::new(pool.clone());
        let encrypted = encrypt_string(" ,sk-first\nsk-second", &TEST_KEY).unwrap();
        repo.create(CreateProviderParams {
            provider_id: None,
            platform,
            name: "Test Provider",
            base_url: "https://api.example.com/v1",
            api_key_encrypted: &encrypted,
            models: "[]",
            enabled,
            capabilities: "[]",
            model_context_limits: None,
            model_protocols: None,
            model_descriptions: None,
            model_enabled: None,
            model_health: None,
            bedrock_config: None,
            is_full_url: false,
            sort_order: None,
        })
        .await
        .unwrap()
        .provider_id
    }

    async fn seed_model(
        pool: &SqlitePool,
        provider_id: &str,
        model: &str,
        tasks: &str,
        protocol: Option<&str>,
        enabled: bool,
    ) {
        let repo = SqliteProviderModelRepository::new(pool.clone());
        repo.create(
            provider_id,
            &NewProviderModel {
                model,
                enabled,
                sort_order: 0,
                tasks,
                traits: "[]",
                protocol,
                params: "{}",
                context_limit: None,
                description: None,
                source: "user",
                health: None,
            },
        )
        .await
        .unwrap();
    }

    async fn set_connection_role(pool: &SqlitePool, provider_id: &str, model: &str, role: &str) {
        let repo = SqliteProviderModelRepository::new(pool.clone());
        repo.update(
            provider_id,
            model,
            &ProviderModelUpdate { connection_role: Some(Some(role)), ..Default::default() },
        )
        .await
        .unwrap();
    }

    fn image_request() -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "p".into(),
            count: 1,
            size: None,
            quality: None,
            extra: json!({}),
        })
    }

    fn asr_request() -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset { id: None, role: "audio".into(), bytes: vec![1], mime: "audio/wav".into() },
            language: None,
            prompt: None,
            extra: json!({}),
        })
    }

    fn chat_request() -> TaskRequest {
        TaskRequest::ChatText(ChatTextRequest { prompt: "hi".into(), system: None, extra: json!({}) })
    }

    fn mref(provider_id: &str, model: &str) -> ModelRef {
        ModelRef { provider_id: provider_id.into(), model: model.into() }
    }

    // -- step 1: provider gating ------------------------------------------

    #[tokio::test]
    async fn non_canonical_provider_id_is_invalid_params() {
        let (svc, _pool) = setup(full_registry()).await;
        let err = svc
            .resolve(&mref("openai", "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::InvalidParams);
    }

    #[tokio::test]
    async fn unknown_provider_is_config_error() {
        let (svc, _pool) = setup(full_registry()).await;
        let err = svc
            .resolve(&mref(&generate_id(), "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
        assert!(err.message.contains("provider not found"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn disabled_provider_is_config_error() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", false).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, None, true).await;
        let err = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
        assert!(err.message.contains("disabled"), "message: {}", err.message);
    }

    // -- step 2: model row gating -----------------------------------------

    #[tokio::test]
    async fn model_without_row_is_unsupported_when_enforcing() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        let err = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
        assert!(err.message.contains("not in catalog"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn disabled_model_is_unsupported_task() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, None, false).await;
        let err = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
        assert!(err.message.contains("disabled"), "message: {}", err.message);
    }

    // -- step 3: task membership ------------------------------------------

    #[tokio::test]
    async fn task_not_declared_is_unsupported_task() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-4o", r#"["chat"]"#, None, true).await;
        let err = svc
            .resolve(&mref(&pid, "gpt-4o"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
    }

    #[tokio::test]
    async fn unseeded_row_falls_back_to_name_heuristic_and_allows() {
        // Row exists but tasks were never seeded — "gpt-image-1" is
        // heuristically an image model, so ImageGeneration passes the gate.
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", "[]", None, true).await;
        let (call, adapter) = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "openai.images");
        assert_eq!(call.task, ModelTask::ImageGeneration);
    }

    #[tokio::test]
    async fn unseeded_row_with_uninferable_name_is_unsupported_task() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "foo-bar", "[]", None, true).await;
        let err = svc
            .resolve(&mref(&pid, "foo-bar"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
    }

    #[tokio::test]
    async fn corrupt_tasks_json_is_treated_as_unseeded() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", "not json at all", None, true).await;
        let (_, adapter) = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "openai.images");
    }

    #[tokio::test]
    async fn probe_mode_skips_task_membership() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        // Tagged chat-only: enforcement would reject ImageGeneration...
        seed_model(&pool, &pid, "mystery-model", r#"["chat"]"#, None, true).await;
        let (call, adapter) = svc
            .resolve(&mref(&pid, "mystery-model"), ModelTask::ImageGeneration, image_request(), false)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "openai.images");
        assert_eq!(call.task, ModelTask::ImageGeneration);
    }

    #[tokio::test]
    async fn probe_mode_tolerates_missing_model_row() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        let (call, adapter) = svc
            .resolve(&mref(&pid, "never-cataloged"), ModelTask::ImageGeneration, image_request(), false)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "openai.images");
        assert_eq!(call.model_params, json!({}));
    }

    // -- steps 4-6: protocol, connection, params, adapter -------------------

    #[tokio::test]
    async fn happy_path_default_connection_resolves_decrypted_key_and_default_protocol() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, None, true).await;
        let (call, adapter) = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "openai.images", "default openai.* protocol from route table");
        assert_eq!(call.provider_id, pid);
        assert_eq!(call.platform, "openai");
        assert_eq!(call.model, "gpt-image-1");
        assert_eq!(call.connection.role, "default");
        assert_eq!(call.connection.base_url, "https://api.example.com/v1");
        assert!(!call.connection.is_full_url);
        assert_eq!(call.connection.auth.scheme, AuthScheme::Bearer);
        // Decrypted, split on [',', '\n'], first NON-EMPTY entry.
        assert_eq!(call.connection.auth.credentials, json!({"api_keys": ["sk-first"]}));
        assert_eq!(call.connection.extra, json!({}));
        assert_eq!(call.model_params, json!({}));
    }

    #[tokio::test]
    async fn model_row_protocol_override_wins_over_route_table() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, Some("custom.images"), true).await;
        let (_, adapter) = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "custom.images");
    }

    #[tokio::test]
    async fn unregistered_row_protocol_override_is_config_not_no_adapter() {
        // Reviewer ruling (T1): a protocol string typed into the model row
        // that no adapter serves is user-fixable model config, not a 500.
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, Some("no.such_protocol"), true).await;
        let err = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
        assert!(err.message.contains("gpt-image-1"), "message: {}", err.message);
        assert!(err.message.contains("no.such_protocol"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn registered_row_protocol_lacking_task_stays_no_adapter() {
        // Only the UNREGISTERED branch remaps to Config; a registered adapter
        // that does not support the task is a genuine NoAdapter.
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, Some("openai.chat_text"), true).await;
        let err = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NoAdapter);
    }

    #[tokio::test]
    async fn route_table_protocol_without_adapter_is_no_adapter() {
        // Registry deliberately lacks openai.images: a route-table protocol
        // nobody registered is a server wiring gap, kept as NoAdapter.
        let (svc, pool) = setup(vec![fake("openai.chat_text", &[ModelTask::Chat])]).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, None, true).await;
        let err = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NoAdapter);
    }

    #[tokio::test]
    async fn missing_connection_profile_is_missing_connection_error() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        seed_model(&pool, &pid, "some-audio-model", r#"["speech_recognition"]"#, Some("volc.asr_file"), true).await;
        set_connection_role(&pool, &pid, "some-audio-model", "voice").await;
        let err = svc
            .resolve(&mref(&pid, "some-audio-model"), ModelTask::SpeechRecognition, asr_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::MissingConnection);
        assert!(err.message.contains("voice"), "message: {}", err.message);
        assert!(err.message.contains("请在供应商连接档案中配置"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn route_table_voice_role_resolves_connection_profile() {
        // The multi-connection path end to end: platform route sends volcano
        // ASR to protocol volc.asr_file on role "voice"; the connection row
        // supplies its own base_url, declared scheme and decrypted creds.
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "ark", true).await;
        seed_model(&pool, &pid, "bigmodel-asr", r#"["speech_recognition"]"#, None, true).await;
        let creds = r#"{"app_key":"app-1","access_key":"ak-1","resource_id":"volc.bigasr.auc"}"#;
        let conn_repo = SqliteProviderConnectionRepository::new(pool.clone());
        conn_repo
            .upsert(
                &pid,
                &UpsertProviderConnectionParams {
                    role: "voice",
                    label: Some("Voice"),
                    base_url: "https://openspeech.example.com",
                    auth_scheme: "volc_voice",
                    credentials_encrypted: &encrypt_string(creds, &TEST_KEY).unwrap(),
                    is_full_url: false,
                    extra: r#"{"region":"cn-north-1"}"#,
                },
            )
            .await
            .unwrap();
        let (call, adapter) = svc
            .resolve(&mref(&pid, "bigmodel-asr"), ModelTask::SpeechRecognition, asr_request(), true)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "volc.asr_file");
        assert_eq!(call.connection.role, "voice");
        assert_eq!(call.connection.base_url, "https://openspeech.example.com");
        assert_eq!(call.connection.auth.scheme, AuthScheme::parse("volc_voice").unwrap());
        assert_eq!(call.connection.auth.credentials, serde_json::from_str::<serde_json::Value>(creds).unwrap());
        assert_eq!(call.connection.extra, json!({"region": "cn-north-1"}));
    }

    #[tokio::test]
    async fn connection_extra_bad_json_is_tolerated_as_empty_object() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "ark", true).await;
        seed_model(&pool, &pid, "bigmodel-asr", r#"["speech_recognition"]"#, None, true).await;
        let conn_repo = SqliteProviderConnectionRepository::new(pool.clone());
        conn_repo
            .upsert(
                &pid,
                &UpsertProviderConnectionParams {
                    role: "voice",
                    label: None,
                    base_url: "https://openspeech.example.com",
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypt_string(r#"{"api_key":"sk-voice"}"#, &TEST_KEY).unwrap(),
                    is_full_url: true,
                    extra: "not-json",
                },
            )
            .await
            .unwrap();
        let (call, _) = svc
            .resolve(&mref(&pid, "bigmodel-asr"), ModelTask::SpeechRecognition, asr_request(), true)
            .await
            .unwrap();
        assert_eq!(call.connection.extra, json!({}));
        assert!(call.connection.is_full_url);
        assert_eq!(call.connection.auth.scheme, AuthScheme::Bearer);
    }

    #[tokio::test]
    async fn model_params_flow_through_and_bad_params_become_empty() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "openai", true).await;
        let repo = SqliteProviderModelRepository::new(pool.clone());
        repo.create(
            &pid,
            &NewProviderModel {
                model: "gpt-image-1",
                enabled: true,
                sort_order: 0,
                tasks: r#"["image_generation"]"#,
                traits: "[]",
                protocol: None,
                params: r#"{"endpoint": "/custom/images", "size": "1024x1024"}"#,
                context_limit: None,
                description: None,
                source: "user",
                health: None,
            },
        )
        .await
        .unwrap();
        seed_model(&pool, &pid, "bad-params-image", "not json", None, true).await;
        repo.update(
            &pid,
            "bad-params-image",
            &ProviderModelUpdate { params: Some("also not json"), ..Default::default() },
        )
        .await
        .unwrap();

        let (call, _) = svc
            .resolve(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .unwrap();
        assert_eq!(call.model_params, json!({"endpoint": "/custom/images", "size": "1024x1024"}));

        let (call, _) = svc
            .resolve(&mref(&pid, "bad-params-image"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .unwrap();
        assert_eq!(call.model_params, json!({}));
    }

    // -- default connection auth overrides + bedrock guard ------------------

    #[tokio::test]
    async fn gemini_default_connection_uses_goog_api_key_header() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "gemini", true).await;
        seed_model(&pool, &pid, "gemini-image-x", r#"["image_generation"]"#, None, true).await;
        let (call, adapter) = svc
            .resolve(&mref(&pid, "gemini-image-x"), ModelTask::ImageGeneration, image_request(), true)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "gemini.generate_content");
        assert_eq!(call.connection.auth.scheme, AuthScheme::HeaderKey("x-goog-api-key".into()));
    }

    #[tokio::test]
    async fn deepgram_default_connection_uses_token_header() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "deepgram", true).await;
        seed_model(&pool, &pid, "nova-3", r#"["speech_recognition"]"#, None, true).await;
        let (call, adapter) = svc
            .resolve(&mref(&pid, "nova-3"), ModelTask::SpeechRecognition, asr_request(), true)
            .await
            .unwrap();
        assert_eq!(adapter.id(), "deepgram.listen");
        assert_eq!(call.connection.auth.scheme, AuthScheme::TokenHeader);
    }

    #[tokio::test]
    async fn bedrock_default_connection_is_rejected_as_config() {
        let (svc, pool) = setup(full_registry()).await;
        let pid = seed_provider(&pool, "bedrock", true).await;
        seed_model(&pool, &pid, "claude-x", r#"["chat"]"#, None, true).await;
        let err = svc
            .resolve(&mref(&pid, "claude-x"), ModelTask::Chat, chat_request(), true)
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
        assert!(
            err.message.contains("bedrock is not supported by the invoke layer yet"),
            "message: {}",
            err.message
        );
    }
}
