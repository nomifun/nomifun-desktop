use serde::Serialize;

use super::*;
use super::anchored_fs::AnchoredKnowledgeFs;

const MAX_BOUND_KNOWLEDGE_READ_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BOUND_KNOWLEDGE_SEARCH_ENTRIES: usize = 32_768;
const MAX_BOUND_KNOWLEDGE_SEARCH_DOCUMENTS: usize = 4_096;
pub(super) const MAX_BOUND_KNOWLEDGE_SEARCH_FILE_BYTES: u64 =
    8 * 1024 * 1024;
const MAX_BOUND_KNOWLEDGE_SEARCH_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Synthetic score anchor for remote recall hits: server-ordered Search v2
/// results rank above local keyword scores inside a remote-backed base.
const REMOTE_SEARCH_SCORE_BASE: u32 = 1_000_000;

/// One Fresh-v4 Knowledge resource resolved from an authorized typed binding.
///
/// The root is an application-resolved host path. It is validated again for
/// every operation because a directory can be replaced after preset compile.
#[derive(Debug, Clone)]
pub struct BoundKnowledgeBase {
    knowledge_base_id: KnowledgeBaseId,
    name: String,
    root: PathBuf,
    /// Curated remote corpus frozen at binding time: remote token → the
    /// official link the read path resolves it through (`None` for purely
    /// local bases). Remote search recall is intersected with this set.
    remote_links: Option<BTreeMap<String, String>>,
}

impl BoundKnowledgeBase {
    pub fn new(
        knowledge_base_id: KnowledgeBaseId,
        name: impl Into<String>,
        root: impl Into<PathBuf>,
    ) -> Result<Self, AppError> {
        let name = name.into();
        let name = name.trim().to_owned();
        if name.trim().is_empty() {
            return Err(AppError::BadRequest(
                "bound knowledge base name must not be blank".into(),
            ));
        }
        let root = root.into();
        if !root.is_absolute() {
            return Err(AppError::BadRequest(
                "bound knowledge base root must be absolute".into(),
            ));
        }
        Ok(Self {
            knowledge_base_id,
            name,
            root,
            remote_links: None,
        })
    }

    /// Freeze the curated Feishu corpus into this binding. An empty map still
    /// marks the base as remote-backed — remote failures must surface, not
    /// degrade to an empty local search.
    pub fn with_remote_links(mut self, links: BTreeMap<String, String>) -> Self {
        self.remote_links = Some(links);
        self
    }

    pub fn knowledge_base_id(&self) -> &KnowledgeBaseId {
        &self.knowledge_base_id
    }
}

/// One local keyword hit returned by the binding-backed Knowledge owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundKnowledgeSearchHit {
    pub handle: String,
    pub resource_id: KnowledgeBaseId,
    pub knowledge_base_name: String,
    pub relative_path: String,
    pub heading: String,
    pub snippet: String,
    pub score: u32,
}

/// A bounded full-document read returned by the binding-backed Knowledge owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundKnowledgeDocument {
    pub handle: String,
    pub resource_id: KnowledgeBaseId,
    pub relative_path: String,
    pub content: String,
    pub size: u64,
    pub content_sha256: String,
}

/// Database-independent, read-only access for a Knowledge resource that has
/// already been selected by a Fresh-v4 typed resource binding.
#[derive(Clone)]
pub struct BoundKnowledgeReadService {
    feishu: Arc<dyn FeishuKnowledgePort>,
}

impl Default for BoundKnowledgeReadService {
    fn default() -> Self {
        Self {
            feishu: Arc::new(feishu::UnavailableFeishuKnowledgePort),
        }
    }
}

impl BoundKnowledgeReadService {
    pub fn new(feishu: Arc<dyn FeishuKnowledgePort>) -> Self {
        Self { feishu }
    }

    pub(super) async fn load_documents(
        &self,
        knowledge_base: &BoundKnowledgeBase,
    ) -> Result<Vec<RetrievalDocument>, AppError> {
        let kb_id = knowledge_base.knowledge_base_id.clone();
        let kb_name = knowledge_base.name.clone();
        let root = knowledge_base.root.clone();
        let lock_root = root.clone();
        let timeout = AppError::Timeout(format!(
            "bound knowledge search timed out for {}",
            knowledge_base.knowledge_base_id
        ));
        bounded_root_blocking(&lock_root, SEARCH_WALK_BUDGET, Err(timeout), move || {
            AnchoredKnowledgeFs::open(&root)?.search_documents(
                &kb_id,
                &kb_name,
                RetrievalLoadLimits {
                    max_entries: MAX_BOUND_KNOWLEDGE_SEARCH_ENTRIES,
                    max_documents: MAX_BOUND_KNOWLEDGE_SEARCH_DOCUMENTS,
                    max_file_bytes: MAX_BOUND_KNOWLEDGE_SEARCH_FILE_BYTES,
                    max_total_bytes: MAX_BOUND_KNOWLEDGE_SEARCH_TOTAL_BYTES,
                },
            )
        })
        .await
    }

    pub async fn search(
        &self,
        knowledge_base: &BoundKnowledgeBase,
        query: &str,
        limit: usize,
    ) -> Result<Vec<BoundKnowledgeSearchHit>, AppError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(AppError::BadRequest(
                "knowledge search query must not be blank".into(),
            ));
        }
        if !(1..=20).contains(&limit) {
            return Err(AppError::BadRequest(
                "knowledge search limit must be between 1 and 20".into(),
            ));
        }

        let documents = self.load_documents(knowledge_base).await?;

        let mut hits: Vec<BoundKnowledgeSearchHit> =
            local_keyword_candidates(documents, query, limit)
                .into_iter()
                .map(|candidate| {
                    let hit = candidate.hit;
                    BoundKnowledgeSearchHit {
                        handle: encode_doc_handle(&hit.kb_id, &hit.rel_path),
                        resource_id: hit.kb_id,
                        knowledge_base_name: hit.kb_name,
                        relative_path: hit.rel_path,
                        heading: hit.heading,
                        snippet: hit.snippet,
                        score: hit.score,
                    }
                })
                .collect();

        // Remote-backed bases union local stub recall with a live Search v2
        // pass intersected against the curated token set. A port failure is
        // propagated — a remote base must never degrade to a silent empty
        // result that hides a broken connection.
        if let Some(remote_links) = &knowledge_base.remote_links {
            let remote_hits = self.feishu.search_documents(query).await?;
            let mut seen_remote = HashSet::new();
            for (rank, hit) in remote_hits
                .into_iter()
                .filter(|hit| remote_links.contains_key(&hit.token))
                .take(limit)
                .enumerate()
            {
                if !seen_remote.insert(hit.token.clone()) {
                    continue;
                }
                let rel_path = feishu::remote_rel_path(&hit.token);
                hits.push(BoundKnowledgeSearchHit {
                    handle: encode_doc_handle(&knowledge_base.knowledge_base_id, &rel_path),
                    resource_id: knowledge_base.knowledge_base_id.clone(),
                    knowledge_base_name: knowledge_base.name.clone(),
                    relative_path: rel_path,
                    heading: hit.title,
                    snippet: hit.summary.unwrap_or_default(),
                    // Remote recall keeps its server ranking above local
                    // keyword scores for this remote-backed base.
                    score: REMOTE_SEARCH_SCORE_BASE - rank as u32,
                });
            }
            hits.sort_by(|left, right| {
                right
                    .score
                    .cmp(&left.score)
                    .then(left.relative_path.cmp(&right.relative_path))
            });
            hits.truncate(limit);
        }

        Ok(hits)
    }

    pub async fn read(
        &self,
        knowledge_base: &BoundKnowledgeBase,
        handle: &str,
    ) -> Result<BoundKnowledgeDocument, AppError> {
        let (handle_kb_id, rel_path) = decode_doc_handle(handle).ok_or_else(|| {
            AppError::BadRequest("invalid knowledge document handle".into())
        })?;
        if handle_kb_id != knowledge_base.knowledge_base_id {
            return Err(AppError::Forbidden(
                "knowledge document handle points to a different bound resource".into(),
            ));
        }

        // `.remote/` pseudo-paths resolve through the curated corpus frozen
        // into the binding — the body is fetched live, never persisted.
        if let Some(token) = feishu::remote_token(&rel_path) {
            let link = knowledge_base
                .remote_links
                .as_ref()
                .and_then(|links| links.get(token))
                .ok_or_else(|| {
                    AppError::Forbidden(
                        "knowledge document handle is not part of this bound base's curated remote set"
                            .into(),
                    )
                })?;
            let document = self.feishu.fetch_document(link).await?;
            let size = document.markdown.len() as u64;
            return Ok(BoundKnowledgeDocument {
                handle: encode_doc_handle(&knowledge_base.knowledge_base_id, &rel_path),
                resource_id: knowledge_base.knowledge_base_id.clone(),
                relative_path: rel_path,
                content_sha256: sha256_text(&document.markdown),
                content: document.markdown,
                size,
            });
        }

        let root = knowledge_base.root.clone();
        let lock_root = root.clone();
        let file = bounded_root_blocking(
            &lock_root,
            KNOWLEDGE_FILE_IO_TIMEOUT,
            Err(AppError::Timeout(
                "knowledge file read timed out".into(),
            )),
            move || {
                AnchoredKnowledgeFs::open(&root)?.read_markdown(
                    &rel_path,
                    MAX_BOUND_KNOWLEDGE_READ_BYTES,
                )
            },
        )
        .await?;

        Ok(BoundKnowledgeDocument {
            handle: encode_doc_handle(
                &knowledge_base.knowledge_base_id,
                &file.rel_path,
            ),
            resource_id: knowledge_base.knowledge_base_id.clone(),
            relative_path: file.rel_path,
            content_sha256: sha256_text(&file.content),
            content: file.content,
            size: file.size,
        })
    }
}
