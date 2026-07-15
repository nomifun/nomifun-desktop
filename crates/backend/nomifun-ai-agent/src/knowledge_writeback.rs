//! Production `KnowledgeWritebackSink` over `KnowledgeService::write_document`.
//! The trait lives in `nomi-agent`; this backend adapter maps the agent-facing
//! request to the service's canonical write path (which resolves the target,
//! enforces staged/direct placement, and writes). The mirror types are kept in
//! separate crates to preserve layering — this is the single mapping point.

use std::sync::Arc;

use async_trait::async_trait;
use nomi_agent::knowledge_tools::{
    KnowledgeWritebackSink, WriteMode as TMode, WriteReceipt, WriteRequest as TReq, WriteTarget,
};
use nomifun_knowledge::{
    KnowledgeService, WriteMode, WriteOp, WritePolicy, WriteRequest, WriteSurface, WriteTargetSpec,
};

/// Bridges the agent-facing write-back trait to the backend KnowledgeService.
pub struct LiveKnowledgeWritebackSink {
    pub service: Arc<KnowledgeService>,
}

#[async_trait]
impl KnowledgeWritebackSink for LiveKnowledgeWritebackSink {
    async fn write(&self, req: TReq) -> Result<WriteReceipt, String> {
        let spec = match req.target {
            WriteTarget::Handle(h) => WriteTargetSpec::Handle(h),
            WriteTarget::Path { kb_id, rel_path } => WriteTargetSpec::Path {
                kb_id,
                rel_path,
            },
        };
        let mode = match req.mode {
            TMode::Direct => WriteMode::Direct,
            TMode::Staged { scope } => WriteMode::Staged { scope },
        };
        // The nomi tool path always permits creating new docs; staged/direct is
        // already decided by the factory. `surface` is informational at this
        // layer (placement is driven by `mode`), so RegularChat is a safe label.
        let policy = WritePolicy { mode, allow_create: true, surface: WriteSurface::RegularChat };
        let bound_kb_ids = req.bound_kb_ids;
        let svc_req = WriteRequest { spec, content: req.content, policy, bound_kb_ids };
        let out = self.service.write_document(svc_req).await.map_err(|e| e.to_string())?;
        Ok(WriteReceipt {
            final_rel_path: out.final_rel_path,
            staged: out.staged,
            updated: matches!(out.op, WriteOp::Update),
        })
    }
}
