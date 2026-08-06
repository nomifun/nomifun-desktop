//! Production `KnowledgeWritebackSink` over `KnowledgeService::write_document`.
//! The trait lives in `nomi-agent`; this backend adapter maps the agent-facing
//! request to the service's canonical write path, which resolves the target and
//! then either creates the document or appends the new material under
//! compare-and-swap. The mirror types are kept in separate crates to preserve
//! layering — this is the single mapping point.

use std::sync::Arc;

use async_trait::async_trait;
use nomi_agent::knowledge_tools::{
    KnowledgeWritebackSink, WriteReceipt, WriteRequest as TReq, WriteTarget,
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
        // The nomi tool path always permits creating new docs, and there is one
        // landing spot left. Whether this session may write at all was already
        // decided by the factory when it chose to install the sink; `surface` is
        // informational at this layer, so RegularChat is a safe label.
        let policy = WritePolicy {
            mode: WriteMode::Direct,
            allow_create: true,
            surface: WriteSurface::RegularChat,
        };
        let bound_kb_ids = req.bound_kb_ids;
        let svc_req = WriteRequest { spec, content: req.content, policy, bound_kb_ids };
        let out = self.service.write_document(svc_req).await.map_err(|e| e.to_string())?;
        Ok(WriteReceipt {
            final_rel_path: out.final_rel_path,
            updated: matches!(out.op, WriteOp::Update),
        })
    }
}
