use async_trait::async_trait;
use nomifun_voice_contracts::voice::*;
use nomifun_voice_contracts::{WorkTarget,WorkSteeringPolicy};

#[derive(Clone, Debug)]
pub struct VoiceWorkContext {
    pub context_floor:u64,
    pub context_reference:String,
    pub instructions: String,
    pub initial_facts:Vec<VerifiedVoiceFact>,
    pub observed_target: Option<WorkTarget>,
    pub facts: Vec<VoiceWorkReceipt>,
    pub approvals: Vec<nomifun_voice_contracts::VoiceApprovalPresentation>,
}
#[async_trait]
pub trait VoiceWorkPort: Send + Sync {
    /// App shutdown closes only unadmitted voice intents. Ending media does
    /// not call this method or cancel canonical Agent work.
    async fn shutdown_pending_inputs(&self)->Result<(),VoiceError>{Ok(())}
    /// Only the authenticated explicit voice activation calls this after its
    /// lazy journal reconciliation. No startup or ordinary text dependency.
    async fn recover_pending_inputs(&self,_owner_id:&str,_agent_session_id:&str)->Result<(),VoiceError>{Ok(())}
    /// Revalidate source identities after opening, before exposing an input lease.
    async fn validate_initial_facts(&self,_owner_id:&str,_session:&str,facts:&[VerifiedVoiceFact])->Result<bool,VoiceError>{Ok(facts.is_empty())}
    async fn validate_speech(&self,_owner_id:&str,_session:&str,_projection:&VoiceSpeechProjection)->Result<bool,VoiceError>{Ok(false)}
    /// Data-only validation of an exact canonical speech source. This does
    /// not grant work authority or infer which generated words were heard.
    async fn validate_source_ref(&self,_owner_id:&str,_session:&str,_source:&VoiceSpeechSourceRef)->Result<bool,VoiceError>{Ok(false)}
    /// Returns only invalid references; the production owner can validate a
    /// whole bounded watch set from one canonical read.
    async fn validate_source_refs(&self,owner_id:&str,session:&str,sources:&[VoiceSpeechSourceRef])->Result<Vec<VoiceSpeechSourceRef>,VoiceError>{
        let mut invalid=Vec::new();for source in sources{if !self.validate_source_ref(owner_id,session,source).await?{invalid.push(source.clone());}}Ok(invalid)
    }
    async fn answer_approval(&self, owner_id:&str, operation_key:&str, answer:nomifun_voice_contracts::VoiceApprovalAnswer)
        ->Result<VoiceWorkReceipt,VoiceError>;
    async fn context(&self, owner_id: &str, agent_session_id: &str, binding_version: u64) -> Result<VoiceWorkContext, VoiceError>;
    async fn interact(&self, owner_id: &str, agent_session_id: &str, binding_version: u64,
        operation_key: &str, request: VoiceWorkRequest) -> Result<VoiceWorkReceipt, VoiceError>;
    async fn interact_with_policy(&self,owner_id:&str,agent_session_id:&str,binding_version:u64,operation_key:&str,request:VoiceWorkRequest,policy:WorkSteeringPolicy)->Result<VoiceWorkReceipt,VoiceError>{
        if policy!=WorkSteeringPolicy::SafeBoundary{return Err(VoiceError::new(VoiceErrorKind::Unsupported,"work owner has no proved model-step supersede policy"));}
        self.interact(owner_id,agent_session_id,binding_version,operation_key,request).await
    }
    async fn lookup_operation(&self, owner_id: &str, agent_session_id: &str, operation_key: &str)
        -> Result<Option<VoiceWorkReceipt>, VoiceError>;
    /// Watching is an observation of canonical facts and never starts a task.
    async fn observe(&self, owner_id: &str, target: &WorkTarget) -> Result<VoiceWorkReceipt, VoiceError>;
}
