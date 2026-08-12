use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use nomifun_api_types::{
    KnowledgeEmbeddingConfig, KnowledgeRerankConfig, KnowledgeRetrievalConfig, ModelTask,
    WebSocketMessage,
};
use nomifun_common::{encrypt_string, AppError};
use nomifun_db::{
    CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteClientPreferenceRepository, SqliteKnowledgeRepository,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, UpdateProviderParams,
};
use nomifun_knowledge::{KnowledgeEventEmitter, KnowledgeService};
use nomifun_model_invoke::{
    AdapterRegistry, InvokeError, ModelInvokeService, ProtocolAdapter, RerankResult,
    ResolvedCall, TaskOutcome, TaskRequest, TaskResult,
};
use nomifun_realtime::UserEventSink;
use tempfile::TempDir;

const TEST_KEY: [u8; 32] = [0x71; 32];
const TEST_MODEL: &str = "retrieval-test-model";
const SEMANTIC_QUERY: &str = "orbital resonance";

#[derive(Clone, Copy)]
enum EmbeddingBehavior {
    Semantic,
    WrongDocumentDimension,
    BumpRevisionAfterQuery,
}

struct FakeEmbeddingAdapter {
    behavior: EmbeddingBehavior,
    calls: AtomicUsize,
    provider_repo: Arc<SqliteProviderRepository>,
    provider_id: String,
    revision_bumped: AtomicBool,
}

#[async_trait::async_trait]
impl ProtocolAdapter for FakeEmbeddingAdapter {
    fn id(&self) -> &'static str {
        "openai.embeddings"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Embedding
    }

    async fn submit(
        &self,
        _http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let TaskRequest::Embedding(request) = &call.request else {
            return Err(InvokeError::config(
                "fake embedding adapter received a non-embedding request",
            ));
        };
        if matches!(self.behavior, EmbeddingBehavior::BumpRevisionAfterQuery)
            && request.inputs.len() == 1
            && request.inputs[0] == SEMANTIC_QUERY
            && !self.revision_bumped.swap(true, Ordering::SeqCst)
        {
            let current = self
                .provider_repo
                .find_by_id(&self.provider_id)
                .await
                .map_err(|error| InvokeError::config(error.to_string()))?
                .ok_or_else(|| InvokeError::config("test provider disappeared"))?;
            let changed_base_url = format!("{}/revision-bump", current.base_url.trim_end_matches('/'));
            self.provider_repo
                .update(
                    &self.provider_id,
                    current.config_revision,
                    UpdateProviderParams {
                        base_url: Some(&changed_base_url),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|error| InvokeError::config(error.to_string()))?;
        }
        let vectors = request
            .inputs
            .iter()
            .map(|input| match self.behavior {
                EmbeddingBehavior::Semantic | EmbeddingBehavior::BumpRevisionAfterQuery => {
                    if input == SEMANTIC_QUERY || input.contains("semantic-target") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    }
                }
                EmbeddingBehavior::WrongDocumentDimension => {
                    if input == SEMANTIC_QUERY {
                        vec![1.0, 0.0]
                    } else {
                        vec![1.0, 0.0, 0.0]
                    }
                }
            })
            .collect();
        Ok(TaskOutcome::Done(TaskResult::Embeddings(vectors)))
    }
}

#[derive(Clone, Copy)]
enum RerankBehavior {
    Reverse,
    DuplicateIndex,
    OutOfRangeIndex,
}

struct FakeRerankAdapter {
    behavior: RerankBehavior,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl ProtocolAdapter for FakeRerankAdapter {
    fn id(&self) -> &'static str {
        "generic.rerank"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Rerank
    }

    async fn submit(
        &self,
        _http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let TaskRequest::Rerank(request) = &call.request else {
            return Err(InvokeError::config(
                "fake rerank adapter received a non-rerank request",
            ));
        };
        let top_n = request.top_n.unwrap_or(request.documents.len() as u32) as usize;
        let results = match self.behavior {
            RerankBehavior::Reverse => (0..top_n)
                .map(|offset| RerankResult {
                    index: request.documents.len() - offset - 1,
                    relevance_score: 1.0 - offset as f32 * 0.1,
                    document: None,
                })
                .collect(),
            RerankBehavior::DuplicateIndex => (0..top_n)
                .map(|offset| RerankResult {
                    index: 0,
                    relevance_score: 1.0 - offset as f32 * 0.1,
                    document: None,
                })
                .collect(),
            RerankBehavior::OutOfRangeIndex => (0..top_n)
                .map(|offset| RerankResult {
                    index: if offset == 0 {
                        request.documents.len()
                    } else {
                        offset - 1
                    },
                    relevance_score: 1.0 - offset as f32 * 0.1,
                    document: None,
                })
                .collect(),
        };
        Ok(TaskOutcome::Done(TaskResult::Reranked(results)))
    }
}

struct NoopUserEvents;

impl UserEventSink for NoopUserEvents {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
}

struct Harness {
    service: Arc<KnowledgeService>,
    provider_id: String,
    embedding_adapter: Arc<FakeEmbeddingAdapter>,
    rerank_adapter: Arc<FakeRerankAdapter>,
    root: TempDir,
    _database: nomifun_db::Database,
}

impl Harness {
    async fn add_base(&self, files: &[(&str, &str)]) -> nomifun_common::KnowledgeBaseId {
        let base_root = self.root.path().join("base");
        std::fs::create_dir_all(&base_root).unwrap();
        for (relative, content) in files {
            let path = base_root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
        self.service
            .create_base("retrieval test", "", base_root.to_str(), None)
            .await
            .unwrap()
            .knowledge_base_id
    }

    async fn configure(&self, embedding: KnowledgeEmbeddingConfig, rerank: KnowledgeRerankConfig) {
        self.service
            .update_retrieval_config(KnowledgeRetrievalConfig { embedding, rerank })
            .await
            .unwrap();
    }

    fn remote_embedding(&self) -> KnowledgeEmbeddingConfig {
        serde_json::from_value(serde_json::json!({
            "mode": "remote",
            "provider_id": self.provider_id,
            "model": TEST_MODEL,
        }))
        .unwrap()
    }

    fn remote_rerank(&self) -> KnowledgeRerankConfig {
        serde_json::from_value(serde_json::json!({
            "mode": "remote",
            "provider_id": self.provider_id,
            "model": TEST_MODEL,
        }))
        .unwrap()
    }
}

async fn harness(
    embedding_behavior: EmbeddingBehavior,
    rerank_behavior: RerankBehavior,
) -> Harness {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let pool = database.pool().clone();
    let provider_id = seed_provider(&pool).await;
    let provider_repo = Arc::new(SqliteProviderRepository::new(pool.clone()));

    let embedding_adapter = Arc::new(FakeEmbeddingAdapter {
        behavior: embedding_behavior,
        calls: AtomicUsize::new(0),
        provider_repo: provider_repo.clone(),
        provider_id: provider_id.clone(),
        revision_bumped: AtomicBool::new(false),
    });
    let rerank_adapter = Arc::new(FakeRerankAdapter {
        behavior: rerank_behavior,
        calls: AtomicUsize::new(0),
    });
    let invoke = Arc::new(ModelInvokeService::new(
        provider_repo,
        Arc::new(SqliteProviderModelRepository::new(pool.clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
        Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
        TEST_KEY,
        reqwest::Client::new(),
        AdapterRegistry::new(vec![
            embedding_adapter.clone() as Arc<dyn ProtocolAdapter>,
            rerank_adapter.clone() as Arc<dyn ProtocolAdapter>,
        ]),
    ));

    let root = tempfile::tempdir().unwrap();
    let owner_id = nomifun_db::installation_owner_id(&pool).await.unwrap();
    let service = Arc::new(KnowledgeService::new(
        Arc::new(SqliteKnowledgeRepository::new(pool.clone())),
        &root.path().join("data"),
        KnowledgeEventEmitter::new(Arc::new(NoopUserEvents), Arc::from(owner_id)),
    ));
    service.set_retrieval_runtime(
        Arc::new(SqliteClientPreferenceRepository::new(pool)),
        invoke,
    );

    Harness {
        service,
        provider_id,
        embedding_adapter,
        rerank_adapter,
        root,
        _database: database,
    }
}

async fn seed_provider(pool: &nomifun_db::SqlitePool) -> String {
    let encrypted = encrypt_string(r#"{"api_keys":["sk-retrieval-test"]}"#, &TEST_KEY).unwrap();
    let capabilities = [
        NewProviderModelCapability {
            task: "embedding",
            traits: "[]",
            protocol: "openai.embeddings",
            connection_role: "default",
            endpoint: Some("/embeddings"),
            provider_params: "{}",
            ..Default::default()
        },
        NewProviderModelCapability {
            task: "rerank",
            traits: "[]",
            protocol: "generic.rerank",
            connection_role: "default",
            endpoint: Some("/rerank"),
            provider_params: "{}",
            ..Default::default()
        },
    ];
    let repository = SqliteProviderRepository::new(pool.clone());
    let (provider, _) = repository
        .create(
            CreateProviderParams {
                provider_id: None,
                platform: "siliconflow",
                name: "Retrieval test provider",
                base_url: "https://api.example.test/v1",
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: None,
            },
            &NewProviderModel {
                model: TEST_MODEL,
                enabled: true,
                sort_order: 0,
                description: None,
                capabilities: &capabilities,
            },
            &[],
        )
        .await
        .unwrap();
    provider.provider_id
}

#[tokio::test]
async fn remote_embedding_finds_a_document_without_keyword_overlap_by_cosine() {
    let harness = harness(EmbeddingBehavior::Semantic, RerankBehavior::Reverse).await;
    let base_id = harness
        .add_base(&[
            ("target.md", "# Target\nsemantic-target: phase-aligned motion"),
            ("decoy.md", "# Decoy\nseasonal bakery inventory"),
        ])
        .await;
    harness
        .configure(harness.remote_embedding(), KnowledgeRerankConfig::Local {})
        .await;

    let hits = harness
        .service
        .search_bases(&[base_id], SEMANTIC_QUERY, 2)
        .await
        .unwrap();

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].rel_path, "target.md");
    assert!(
        harness.embedding_adapter.calls.load(Ordering::SeqCst) >= 2,
        "the real invoke path must embed both the query and documents"
    );
}

#[tokio::test]
async fn local_keyword_candidates_are_reordered_by_remote_rerank_indices() {
    let harness = harness(EmbeddingBehavior::Semantic, RerankBehavior::Reverse).await;
    let base_id = harness
        .add_base(&[
            ("00-first.md", "# First\nneedle alpha"),
            ("01-second.md", "# Second\nneedle beta"),
        ])
        .await;
    harness
        .configure(KnowledgeEmbeddingConfig::Local {}, harness.remote_rerank())
        .await;

    let hits = harness
        .service
        .search_bases(&[base_id], "needle", 2)
        .await
        .unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.rel_path.as_str()).collect::<Vec<_>>(),
        vec!["01-second.md", "00-first.md"]
    );
    assert_eq!(harness.rerank_adapter.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn remote_embedding_dimension_mismatch_fails_closed() {
    let harness = harness(
        EmbeddingBehavior::WrongDocumentDimension,
        RerankBehavior::Reverse,
    )
    .await;
    let base_id = harness
        .add_base(&[("document.md", "# Document\nsemantic-target")])
        .await;
    harness
        .configure(harness.remote_embedding(), KnowledgeRerankConfig::Local {})
        .await;

    let error = harness
        .service
        .search_bases(&[base_id], SEMANTIC_QUERY, 1)
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::BadGateway(_)));
    assert!(error.to_string().contains("dimension 3"), "{error}");
}

#[tokio::test]
async fn remote_embedding_rejects_a_provider_revision_change_between_batches() {
    let harness = harness(
        EmbeddingBehavior::BumpRevisionAfterQuery,
        RerankBehavior::Reverse,
    )
    .await;
    let base_id = harness
        .add_base(&[("document.md", "# Document\nsemantic-target")])
        .await;
    harness
        .configure(harness.remote_embedding(), KnowledgeRerankConfig::Local {})
        .await;

    let error = harness
        .service
        .search_bases(&[base_id], SEMANTIC_QUERY, 1)
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Conflict(_)), "{error}");
    assert!(error.to_string().contains("changed during retrieval"), "{error}");
}

#[tokio::test]
async fn remote_rerank_duplicate_and_out_of_range_indices_fail_closed() {
    for (behavior, expected_message) in [
        (RerankBehavior::DuplicateIndex, "duplicate index 0"),
        (RerankBehavior::OutOfRangeIndex, "out-of-range index 2"),
    ] {
        let harness = harness(EmbeddingBehavior::Semantic, behavior).await;
        let base_id = harness
            .add_base(&[
                ("00-first.md", "# First\nneedle alpha"),
                ("01-second.md", "# Second\nneedle beta"),
            ])
            .await;
        harness
            .configure(KnowledgeEmbeddingConfig::Local {}, harness.remote_rerank())
            .await;

        let error = harness
            .service
            .search_bases(&[base_id], "needle", 2)
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::BadGateway(_)));
        assert!(error.to_string().contains(expected_message), "{error}");
    }
}
