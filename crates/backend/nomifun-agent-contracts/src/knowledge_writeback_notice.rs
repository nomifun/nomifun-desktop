//! Knowledge write-back observations on the canonical fact chain.
//! These records grant no tool authority and never authorize execution replay.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ModelFailureDiagnostic, OperationId, ResolvedSnapshotRef, ResourceId};

pub const TURN_KNOWLEDGE_WRITEBACK_STARTED: &str = "turn/knowledge-writeback-started";
pub const TURN_KNOWLEDGE_WRITEBACK_SETTLED: &str = "turn/knowledge-writeback-settled";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeWritebackNoticeStatus {
    Running, Written, Partial, Failed, NoCandidate, NoCompleter, Cancelled, Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeWritebackStage {
    Resolve, Connect, FirstFrame, Stream, Decode, Validate, Write, Settle, Cancelled, Interrupted,
}

/// Bounded observations, never response text, request bodies or credentials.
/// `code` uses the shared application/Agent error vocabulary; success has no
/// error code. Token estimates are local byte-based pressure estimates, not
/// provider tokenization or billing claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeWritebackDiagnostic {
    pub stage: KnowledgeWritebackStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Locally authored, credential-redacted explanation. Provider response
    /// prose and rejected JSON values must never enter this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_frame_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_frames: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_estimate: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_token_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_diagnostic: Option<ModelFailureDiagnostic>,
}

impl KnowledgeWritebackDiagnostic {
    pub fn new(stage: KnowledgeWritebackStage) -> Self {
        Self { stage, code: None, detail: None, elapsed_ms: None, first_frame_ms: None,
            received_frames: None, received_bytes: None, output_bytes: None,
            input_tokens_estimate: None, output_token_limit: None, output_tokens: None,
            provider_diagnostic: None }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.code.as_ref().is_some_and(|code| code.is_empty() || code.len() > 96
            || !code.bytes().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'))
            || self.first_frame_ms.zip(self.elapsed_ms).is_some_and(|(first, elapsed)| first > elapsed)
            || self.detail.as_ref().is_some_and(|value| value.trim().is_empty() || value.len() > 2048 || value.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t')))
            || self.provider_diagnostic.as_ref().is_some_and(|value| serde_json::to_vec(value).map_or(true, |bytes| bytes.len() > 8192)) {
            return Err("Knowledge write-back diagnostic is inconsistent or unbounded".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeWritebackNoticeFile {
    pub kb_id: ResourceId,
    pub rel_path: String,
}

/// Renderer-only view of exact canonical observations. Old completed-Turn
/// receipts may lack an event sequence; new observations always supply it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeWritebackNotice {
    pub status: KnowledgeWritebackNoticeStatus,
    pub written: Vec<KnowledgeWritebackNoticeFile>,
    /// Read-only compatibility with already committed current-generation
    /// observations. New writers use `diagnostic` and never fill both fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<KnowledgeWritebackDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_seq: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeWritebackStartedV1 {
    pub operation_id: OperationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub binding_version: u64,
    pub execution_instance_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeWritebackSettledV1 {
    pub operation_id: OperationId,
    pub status: KnowledgeWritebackNoticeStatus,
    pub written: Vec<KnowledgeWritebackNoticeFile>,
    /// Already committed observations remain immutable; new writers only
    /// emit the structured diagnostic below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<KnowledgeWritebackDiagnostic>,
}

fn validate_operation(operation: &OperationId) -> Result<(), String> {
    let value = operation.as_ref();
    if value.trim() != value || value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err("Knowledge write-back requires a bounded exact Turn operation".into());
    }
    Ok(())
}

pub fn validate_knowledge_execution_instance(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.trim() != value || value.chars().any(char::is_control) {
        return Err("invalid Knowledge write-back execution instance".into());
    }
    Ok(())
}

impl KnowledgeWritebackStartedV1 {
    pub fn validate(&self) -> Result<(), String> {
        validate_operation(&self.operation_id)?;
        validate_knowledge_execution_instance(&self.execution_instance_id)?;
        if self.binding_version == 0 { return Err("Knowledge write-back binding version is missing".into()); }
        Ok(())
    }
}

impl KnowledgeWritebackSettledV1 {
    pub fn validate(&self) -> Result<(), String> {
        validate_operation(&self.operation_id)?;
        if self.status == KnowledgeWritebackNoticeStatus::Running {
            return Err("Knowledge write-back settlement must be terminal".into());
        }
        KnowledgeWritebackNotice {
            status: self.status.clone(), written: self.written.clone(), detail: self.detail.clone(), diagnostic: self.diagnostic.clone(), event_seq: None,
        }.validate()
    }

    pub fn notice(&self, event_seq: u64) -> KnowledgeWritebackNotice {
        KnowledgeWritebackNotice { status: self.status.clone(), written: self.written.clone(), detail: self.detail.clone(), diagnostic: self.diagnostic.clone(), event_seq: Some(event_seq) }
    }
}

impl KnowledgeWritebackNotice {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(diagnostic) = &self.diagnostic {
            diagnostic.validate()?;
            if self.detail.is_some() {
                return Err("Knowledge write-back must not duplicate structured and legacy diagnostics".into());
            }
        }
        let invalid_targets = match self.status {
            KnowledgeWritebackNoticeStatus::Written | KnowledgeWritebackNoticeStatus::Partial => self.written.is_empty(),
            // Cancellation is a stop request, not rollback. Indivisible owner
            // publication may already have returned successful file receipts.
            KnowledgeWritebackNoticeStatus::Cancelled | KnowledgeWritebackNoticeStatus::Interrupted => false,
            KnowledgeWritebackNoticeStatus::Running | KnowledgeWritebackNoticeStatus::Failed
                | KnowledgeWritebackNoticeStatus::NoCandidate | KnowledgeWritebackNoticeStatus::NoCompleter => !self.written.is_empty(),
        };
        if self.written.len() > 8 || invalid_targets
            || self.event_seq == Some(0)
            || self.detail.as_ref().is_some_and(|detail| detail.trim().is_empty() || detail.len() > 2048 || detail.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))) {
            return Err("Knowledge write-back observation has inconsistent or unbounded outcome".into());
        }
        let mut targets = std::collections::BTreeSet::new();
        for file in &self.written {
            if uuid::Uuid::parse_str(file.kb_id.as_ref()).ok()
                .is_none_or(|id| id.get_version_num() != 7 || id.to_string() != file.kb_id.as_ref())
                || file.rel_path.is_empty() || file.rel_path.len() > 4096
                || file.rel_path.starts_with('/') || file.rel_path.contains('\\')
                || file.rel_path.chars().any(char::is_control)
                || file.rel_path.as_bytes().get(1) == Some(&b':')
                || file.rel_path.split('/').any(|part| part.is_empty() || matches!(part, "." | ".."))
                || !targets.insert((file.kb_id.as_ref(), file.rel_path.as_str())) {
                return Err("invalid Knowledge write-back notice target".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn notice() -> KnowledgeWritebackNotice {
        KnowledgeWritebackNotice {
            status: KnowledgeWritebackNoticeStatus::Written,
            written: vec![KnowledgeWritebackNoticeFile { kb_id: "0190f5fe-7c00-7a00-8000-000000000001".into(), rel_path: "notes/loops.md".into() }],
            detail: None, diagnostic: None, event_seq: None,
        }
    }
    #[test]
    fn knowledge_notice_requires_consistent_bounded_outcomes() {
        let mut value = notice(); value.validate().unwrap();
        value.status = KnowledgeWritebackNoticeStatus::Partial; value.validate().unwrap();
        value.written.push(value.written[0].clone()); assert!(value.validate().is_err());
        value.written.clear(); assert!(value.validate().is_err());
        for status in [KnowledgeWritebackNoticeStatus::Running, KnowledgeWritebackNoticeStatus::Failed, KnowledgeWritebackNoticeStatus::NoCandidate,
            KnowledgeWritebackNoticeStatus::NoCompleter, KnowledgeWritebackNoticeStatus::Cancelled, KnowledgeWritebackNoticeStatus::Interrupted] {
            value.status = status; value.validate().unwrap();
        }
        value.detail = Some("x".repeat(2049)); assert!(value.validate().is_err());
        value.detail = None; value.event_seq = Some(0); assert!(value.validate().is_err());
    }
    #[test]
    fn knowledge_notice_preserves_actual_publication_receipts_after_cancel_or_interruption() {
        for status in [KnowledgeWritebackNoticeStatus::Cancelled, KnowledgeWritebackNoticeStatus::Interrupted] {
            let mut value = notice(); value.status = status.clone(); value.validate().unwrap();
            KnowledgeWritebackSettledV1 { operation_id: "original-turn".into(), status: status.clone(), written: value.written.clone(), detail: None, diagnostic: None }.validate().unwrap();
            value.written.clear(); value.validate().unwrap();
        }
        for status in [KnowledgeWritebackNoticeStatus::Running, KnowledgeWritebackNoticeStatus::Failed,
            KnowledgeWritebackNoticeStatus::NoCandidate, KnowledgeWritebackNoticeStatus::NoCompleter] {
            let mut value = notice(); value.status = status; assert!(value.validate().is_err(), "{:?}", value.status);
        }
    }
    #[test]
    fn knowledge_notice_rejects_unsafe_navigation_and_unrecognized_status() {
        for path in ["../other.md", "/tmp/other.md", "C:/other.md", "notes\\other.md", "notes//other.md"] {
            let mut value = notice(); value.written[0].rel_path = path.into(); assert!(value.validate().is_err(), "{path}");
        }
        assert!(serde_json::from_value::<KnowledgeWritebackNotice>(serde_json::json!({ "status":"extracting", "written":notice().written })).is_err());
    }

    #[test]
    fn knowledge_diagnostic_is_bounded_and_survives_canonical_roundtrip() {
        let unknown = KnowledgeWritebackDiagnostic::new(KnowledgeWritebackStage::Interrupted);
        let wire = serde_json::to_value(&unknown).unwrap();
        for field in ["elapsed_ms","first_frame_ms","received_frames","received_bytes","output_bytes","input_tokens_estimate","output_token_limit","output_tokens"] {
            assert!(wire.get(field).is_none(), "unknown metric {field} must not become zero");
        }
        let mut diagnostic = KnowledgeWritebackDiagnostic::new(KnowledgeWritebackStage::Stream);
        diagnostic.code = Some("USER_LLM_PROVIDER_TIMEOUT".into());
        diagnostic.elapsed_ms = Some(90_000);
        diagnostic.first_frame_ms = Some(1200);
        diagnostic.received_frames = Some(123);
        diagnostic.received_bytes = Some(640_000);
        diagnostic.output_bytes = Some(9000);
        diagnostic.input_tokens_estimate = Some(5000);
        diagnostic.output_token_limit = Some(16_384);
        diagnostic.provider_diagnostic = Some(ModelFailureDiagnostic::new(crate::ModelFailureReason::RequestTimeout));
        let outcome = KnowledgeWritebackSettledV1 { operation_id: "completed-turn".into(), status: KnowledgeWritebackNoticeStatus::Failed,
            written: vec![], detail: None, diagnostic: Some(diagnostic.clone()) };
        outcome.validate().unwrap();
        let decoded: KnowledgeWritebackSettledV1 = serde_json::from_value(serde_json::to_value(&outcome).unwrap()).unwrap();
        assert_eq!(decoded.notice(123).diagnostic, Some(diagnostic.clone()));
        diagnostic.detail = Some("知".repeat(700)); assert!(diagnostic.validate().is_err());
        diagnostic.detail = Some("safe local cause".into()); diagnostic.validate().unwrap();
        diagnostic.code = Some("untrusted body with spaces".into()); assert!(diagnostic.validate().is_err());
        diagnostic.code = Some("TIMEOUT".into()); diagnostic.first_frame_ms = Some(90_001); assert!(diagnostic.validate().is_err());
        let mut old = notice(); old.detail = Some("existing immutable detail".into()); old.validate().unwrap();
        old.diagnostic = Some(KnowledgeWritebackDiagnostic::new(KnowledgeWritebackStage::Write)); assert!(old.validate().is_err());
    }
}
