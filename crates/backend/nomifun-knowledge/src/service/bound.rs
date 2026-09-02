use serde::Serialize;
use tokio::io::AsyncReadExt;

use super::*;

const MAX_BOUND_KNOWLEDGE_READ_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BOUND_KNOWLEDGE_SEARCH_DOCUMENTS: usize = 4_096;
pub(super) const MAX_BOUND_KNOWLEDGE_SEARCH_FILE_BYTES: u64 =
    8 * 1024 * 1024;
const MAX_BOUND_KNOWLEDGE_SEARCH_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// One Fresh-v4 Knowledge resource resolved from an authorized typed binding.
///
/// The root is an application-resolved host path. It is validated again for
/// every operation because a directory can be replaced after preset compile.
#[derive(Debug, Clone)]
pub struct BoundKnowledgeBase {
    knowledge_base_id: KnowledgeBaseId,
    name: String,
    root: PathBuf,
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
        })
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
#[derive(Clone, Default)]
pub struct BoundKnowledgeReadService {
    search_cache: Arc<RwLock<SearchCacheInner>>,
}

impl BoundKnowledgeReadService {
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

        let kb_id = knowledge_base.knowledge_base_id.clone();
        let kb_name = knowledge_base.name.clone();
        let root = knowledge_base.root.clone();
        let lock_root = root.clone();
        let cache = Arc::clone(&self.search_cache);
        let timeout = AppError::Timeout(format!(
            "bound knowledge search timed out for {}",
            knowledge_base.knowledge_base_id
        ));
        let documents = bounded_root_blocking(
            &lock_root,
            SEARCH_WALK_BUDGET,
            Err(timeout),
            move || {
                load_one_knowledge_root_checked(
                    kb_id,
                    kb_name,
                    root,
                    cache,
                    Some(RetrievalLoadLimits {
                        max_documents:
                            MAX_BOUND_KNOWLEDGE_SEARCH_DOCUMENTS,
                        max_file_bytes:
                            MAX_BOUND_KNOWLEDGE_SEARCH_FILE_BYTES,
                        max_total_bytes:
                            MAX_BOUND_KNOWLEDGE_SEARCH_TOTAL_BYTES,
                    }),
                )
            },
        )
        .await?;

        Ok(local_keyword_candidates(documents, query, limit)
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
            .collect())
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

        let root = knowledge_base.root.clone();
        let path = safe_md_path_bounded(root.clone(), rel_path).await?;
        let metadata = tokio::time::timeout(
            KNOWLEDGE_PATH_INSPECTION_TIMEOUT,
            tokio::fs::symlink_metadata(&path),
        )
        .await
        .map_err(|_| AppError::Timeout("knowledge file inspection timed out".into()))?
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound("knowledge document was not found".into())
            } else {
                AppError::Internal(format!(
                    "failed to inspect bound knowledge document: {error}"
                ))
            }
        })?;
        if metadata_is_link_or_reparse(&path, &metadata) || !metadata.is_file() {
            return Err(AppError::BadRequest(
                "knowledge document must be a real Markdown file".into(),
            ));
        }
        if metadata.len() > MAX_BOUND_KNOWLEDGE_READ_BYTES {
            return Err(AppError::BadRequest(format!(
                "knowledge document exceeds the {} MiB read limit",
                MAX_BOUND_KNOWLEDGE_READ_BYTES / 1024 / 1024
            )));
        }

        let read = async {
            let file = tokio::fs::File::open(&path).await.map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    AppError::NotFound("knowledge document was not found".into())
                } else {
                    AppError::Internal(format!(
                        "failed to open bound knowledge document: {error}"
                    ))
                }
            })?;
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            file.take(MAX_BOUND_KNOWLEDGE_READ_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|error| {
                    AppError::Internal(format!(
                        "failed to read bound knowledge document: {error}"
                    ))
                })?;
            if bytes.len() as u64 > MAX_BOUND_KNOWLEDGE_READ_BYTES {
                return Err(AppError::BadRequest(format!(
                    "knowledge document exceeds the {} MiB read limit",
                    MAX_BOUND_KNOWLEDGE_READ_BYTES / 1024 / 1024
                )));
            }
            String::from_utf8(bytes).map_err(|_| {
                AppError::BadRequest(
                    "knowledge document must contain valid UTF-8 text".into(),
                )
            })
        };
        let content = tokio::time::timeout(KNOWLEDGE_FILE_IO_TIMEOUT, read)
            .await
            .map_err(|_| AppError::Timeout("knowledge file read timed out".into()))??;
        let relative_path = tree_relative_path(&root, &path)?;
        let size = u64::try_from(content.len()).map_err(|_| {
            AppError::Internal(
                "knowledge document size cannot be represented".into(),
            )
        })?;

        Ok(BoundKnowledgeDocument {
            handle: encode_doc_handle(
                &knowledge_base.knowledge_base_id,
                &relative_path,
            ),
            resource_id: knowledge_base.knowledge_base_id.clone(),
            relative_path,
            content_sha256: sha256_text(&content),
            content,
            size,
        })
    }
}
