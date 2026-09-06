use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    CanonicalErrorCode, NodeRuntimeFingerprint, RuntimeInstallationId,
    RuntimeSelectionRecord, RuntimeSwitchDecision, RuntimeSwitchValidationResult,
};
use tokio::sync::{Mutex, RwLock};

use crate::JavaScriptRuntimeError;

#[derive(Debug)]
pub struct NodeRuntimeManager {
    selection: RwLock<RuntimeSelectionRecord>,
    switch_gate: Mutex<()>,
}

impl NodeRuntimeManager {
    pub fn new(
        selection: RuntimeSelectionRecord,
    ) -> Result<Self, JavaScriptRuntimeError> {
        selection
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        Ok(Self {
            selection: RwLock::new(selection),
            switch_gate: Mutex::new(()),
        })
    }

    pub fn empty() -> Self {
        Self {
            selection: RwLock::new(RuntimeSelectionRecord {
                selected_runtime: None,
                pending_candidate: None,
                validation_result: None,
                last_error: None,
                non_recommended_warning_acknowledged: BTreeSet::new(),
            }),
            switch_gate: Mutex::new(()),
        }
    }

    pub async fn snapshot(&self) -> RuntimeSelectionRecord {
        self.selection.read().await.clone()
    }

    pub async fn begin_switch(
        &self,
        candidate: NodeRuntimeFingerprint,
    ) -> Result<RuntimeSelectionRecord, JavaScriptRuntimeError> {
        candidate
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        let _gate = self.switch_gate.lock().await;
        let mut selection = self.selection.write().await;
        selection.pending_candidate = Some(candidate);
        selection.validation_result = None;
        selection.last_error = None;
        selection
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        Ok(selection.clone())
    }

    pub async fn record_validation(
        &self,
        result: RuntimeSwitchValidationResult,
    ) -> Result<RuntimeSelectionRecord, JavaScriptRuntimeError> {
        result
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        let _gate = self.switch_gate.lock().await;
        let mut selection = self.selection.write().await;
        if selection.pending_candidate.as_ref() != Some(&result.candidate) {
            return Err(JavaScriptRuntimeError::CandidateMismatch);
        }
        selection.validation_result = Some(result);
        selection
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        Ok(selection.clone())
    }

    pub async fn decide(
        &self,
        decision: RuntimeSwitchDecision,
    ) -> Result<RuntimeSelectionRecord, JavaScriptRuntimeError> {
        let _gate = self.switch_gate.lock().await;
        let mut selection = self.selection.write().await;
        let candidate = selection
            .pending_candidate
            .clone()
            .ok_or(JavaScriptRuntimeError::NoPendingCandidate)?;
        match decision {
            RuntimeSwitchDecision::CommitCandidate => {
                let validation = selection
                    .validation_result
                    .as_ref()
                    .ok_or(JavaScriptRuntimeError::ValidationRequired)?;
                if validation.candidate != candidate {
                    return Err(JavaScriptRuntimeError::CandidateMismatch);
                }
                selection.selected_runtime = Some(candidate);
            }
            RuntimeSwitchDecision::AbortAndRestoreSelected => {}
        }
        selection.pending_candidate = None;
        selection.validation_result = None;
        selection.last_error = None;
        selection
            .validate()
            .map_err(|error| JavaScriptRuntimeError::Contract(error.to_string()))?;
        Ok(selection.clone())
    }

    pub async fn record_error(
        &self,
        code: CanonicalErrorCode,
    ) -> RuntimeSelectionRecord {
        let _gate = self.switch_gate.lock().await;
        let mut selection = self.selection.write().await;
        selection.last_error = Some(code);
        selection.clone()
    }

    pub async fn acknowledge_non_recommended(
        &self,
        runtime_installation_id: RuntimeInstallationId,
    ) -> RuntimeSelectionRecord {
        let mut selection = self.selection.write().await;
        selection
            .non_recommended_warning_acknowledged
            .insert(runtime_installation_id);
        selection.clone()
    }
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::{
        DigestHex, JAVASCRIPT_HOST_PROTOCOL_VERSION, JAVASCRIPT_SDK_CONTRACT_VERSION,
        NodeRuntimeSourceKind, RuntimeSwitchParticipantKind,
        RuntimeSwitchParticipantOutcome, RuntimeSwitchParticipantResult,
        RuntimeTarget, VersionString,
    };

    use super::*;

    fn runtime(digest: char) -> NodeRuntimeFingerprint {
        NodeRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from(format!(
                "node-{digest}"
            )),
            source_kind: NodeRuntimeSourceKind::Managed,
            node_version: VersionString::from("24.8.0"),
            node_major: 24,
            runtime_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            executable_digest: DigestHex::from(digest.to_string().repeat(64)),
            javascript_host_protocol_version:
                JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version:
                JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
        }
    }

    #[tokio::test]
    async fn candidate_is_not_committed_before_exact_validation() {
        let manager = NodeRuntimeManager::empty();
        manager.begin_switch(runtime('a')).await.unwrap();
        assert!(matches!(
            manager
                .decide(RuntimeSwitchDecision::CommitCandidate)
                .await,
            Err(JavaScriptRuntimeError::ValidationRequired)
        ));
        assert!(manager.snapshot().await.selected_runtime.is_none());
    }

    #[tokio::test]
    async fn commit_and_abort_keep_one_global_runtime() {
        let manager = NodeRuntimeManager::empty();
        let first = runtime('a');
        manager.begin_switch(first.clone()).await.unwrap();
        manager
            .record_validation(RuntimeSwitchValidationResult {
                candidate: first.clone(),
                foundation_hello_passed: true,
                old_runtime_process_tree_zero: true,
                participants: vec![RuntimeSwitchParticipantResult {
                    kind: RuntimeSwitchParticipantKind::BuildFoundation,
                    owner_id: "javascript-build-foundation".into(),
                    outcome: RuntimeSwitchParticipantOutcome::Passed,
                    error_code: None,
                }],
                completed_at_ms: 1,
            })
            .await
            .unwrap();
        manager
            .decide(RuntimeSwitchDecision::CommitCandidate)
            .await
            .unwrap();

        let second = runtime('b');
        manager.begin_switch(second).await.unwrap();
        let after_abort = manager
            .decide(RuntimeSwitchDecision::AbortAndRestoreSelected)
            .await
            .unwrap();
        assert_eq!(after_abort.selected_runtime, Some(first));
        assert!(after_abort.pending_candidate.is_none());
    }
}
