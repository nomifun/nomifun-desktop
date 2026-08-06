//! E2E regression for the unified write stack (P1). Drives the REAL nomi tool
//! → `LiveKnowledge*Sink` → `KnowledgeService::write_document` chain to prove:
//!   1. the reported bug is dead — a staged write-back where the model passes
//!      the workspace-mount path lands in the review inbox mirroring the
//!      original (NOT a new nested file), with the original untouched;
//!   2. the search → read → write loop updates the original in place by handle,
//!      with zero path arithmetic and no duplicate file.

use std::sync::Arc;

use nomi_agent::knowledge_tools::{
    KnowledgeReadTool, KnowledgeRetrievalSink, KnowledgeSearchTool, KnowledgeWritebackSink, KnowledgeWriteTool,
};
use nomi_tools::Tool;
use serde_json::json;

/// `nomifun_realtime` ships no public no-op broadcaster, so define a local one
/// (same pattern as `knowledge_search_e2e`).
struct NoopBroadcaster;

impl nomifun_realtime::UserEventSink for NoopBroadcaster {
    fn send_to_user(
        &self,
        _user_id: &str,
        _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
    ) {
    }
}

async fn build_service() -> (Arc<nomifun_knowledge::KnowledgeService>, tempfile::TempDir) {
    let db = nomifun_db::init_database_memory().await.expect("in-memory db");
    let installation_owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let repo = Arc::new(nomifun_db::SqliteKnowledgeRepository::new(db.pool().clone()));
    let tmp = tempfile::tempdir().unwrap();
    let emitter = nomifun_knowledge::KnowledgeEventEmitter::new(
        Arc::new(NoopBroadcaster),
        Arc::from(installation_owner),
    );
    let svc = Arc::new(nomifun_knowledge::KnowledgeService::new(repo, tmp.path(), emitter));
    (svc, tmp)
}

#[tokio::test]
async fn write_tool_with_mount_prefixed_path_updates_the_original_not_a_nested_copy() {
    let (svc, _tmp) = build_service().await;
    let info = svc.create_base("领域库", "", None, None).await.unwrap();
    svc.write_file(&info.knowledge_base_id, "terms.md", "ORIGINAL").await.unwrap();

    let sink: Arc<dyn KnowledgeWritebackSink> =
        Arc::new(nomifun_ai_agent::LiveKnowledgeWritebackSink { service: svc.clone() });
    let tool = KnowledgeWriteTool::new(
        sink,
        vec![(info.knowledge_base_id.clone(), info.name.clone())],
        vec![info.knowledge_base_id.clone()],
    );

    // The exact reported mistake: the model passes the workspace-mount path.
    let res = tool
        .execute(json!({
            "base": "领域库",
            "rel_path": ".nomi/knowledge/领域库/terms.md",
            "content": "PROPOSED EDIT"
        }))
        .await;
    assert!(!res.is_error, "tool errored: {}", res.content);

    // The mount prefix resolved to the real document: the curated text is still
    // there and the new material joined it.
    let after = svc.read_file(&info.knowledge_base_id, "terms.md").await.unwrap().content;
    assert!(after.contains("ORIGINAL"), "existing content must survive: {after}");
    assert!(after.contains("PROPOSED EDIT"), "new material must be recorded: {after}");
    // No stray nested file under the mount path.
    let files = svc.list_files(&info.knowledge_base_id).await.unwrap();
    assert!(
        !files.iter().any(|f| f.rel_path.contains(".nomi/knowledge")),
        "must not create a nested mount-path file: {files:?}"
    );
}

#[tokio::test]
async fn search_read_write_handle_loop_appends_to_the_addressed_document() {
    let (svc, _tmp) = build_service().await;
    let info = svc.create_base("金融库", "", None, None).await.unwrap();
    svc.write_file(&info.knowledge_base_id, "terms.md", "# 术语表\n市盈率 = PER\n").await.unwrap();

    let retrieval: Arc<dyn KnowledgeRetrievalSink> =
        Arc::new(nomifun_ai_agent::LiveKnowledgeRetrievalSink { service: svc.clone() });
    let writeback: Arc<dyn KnowledgeWritebackSink> =
        Arc::new(nomifun_ai_agent::LiveKnowledgeWritebackSink { service: svc.clone() });

    let search = KnowledgeSearchTool::new(retrieval.clone(), vec![info.knowledge_base_id.clone()]);
    let read = KnowledgeReadTool::new(retrieval, vec![info.knowledge_base_id.clone()]);
    let write = KnowledgeWriteTool::new(
        writeback,
        vec![(info.knowledge_base_id.clone(), info.name.clone())],
        vec![info.knowledge_base_id.clone()],
    );

    // 1. Search → extract the opaque handle from the rendered result.
    let s = search.execute(json!({"query": "市盈率"})).await;
    assert!(!s.is_error, "{}", s.content);
    let handle = s
        .content
        .lines()
        .find_map(|l| l.trim().strip_prefix("handle: "))
        .expect("search result must carry a handle")
        .to_owned();

    // 2. Read the full document by handle (no path arithmetic).
    let r = read.execute(json!({ "handle": handle })).await;
    assert!(!r.is_error && r.content.contains("市盈率"), "read by handle: {}", r.content);

    // 3. Update by handle: the contract asks for ONLY the new material, and the
    //    service appends it to the document it addressed.
    let w = write
        .execute(json!({ "handle": handle, "content": "ROE = 净资产收益率" }))
        .await;
    assert!(!w.is_error, "write by handle: {}", w.content);

    let updated = svc.read_file(&info.knowledge_base_id, "terms.md").await.unwrap().content;
    assert!(updated.contains("ROE"), "new material must be recorded: {updated}");
    assert!(updated.contains("市盈率 = PER"), "curated content must survive: {updated}");
    assert!(updated.starts_with("# 术语表"), "the heading must stay first: {updated}");
    let files = svc.list_files(&info.knowledge_base_id).await.unwrap();
    assert_eq!(
        files.iter().filter(|f| f.rel_path.ends_with("terms.md")).count(),
        1,
        "must not create a duplicate document: {files:?}"
    );
}
