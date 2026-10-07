use std::collections::BTreeMap;
use nomifun_voice_contracts::voice::*;

/// Only delivery provenance is kept here. Canonical authorization remains in
/// VoiceWorkPort, and consumed media stays immutable in VoiceJournal.
#[derive(Default)]
pub(super) struct SpeechSourceLedger {
    sources:BTreeMap<String,(VoiceSpeechSourceRef,u64)>,
    revoked:BTreeMap<String,u64>,
}
const MAX_SOURCES:usize=256;
const MAX_REVOCATIONS:usize=512;
impl SpeechSourceLedger {
    pub fn register(&mut self,source:VoiceSpeechSourceRef,generation:u64)->Result<bool,VoiceError>{
        source.validate().map_err(|message|VoiceError::new(VoiceErrorKind::Configuration,message))?;
        if self.revoked.get(&source.message_id).is_some_and(|revision|*revision>=source.revision){return Ok(false);}
        if self.sources.get(&source.message_id).is_some_and(|(prior,_)|prior.revision>source.revision){return Ok(false);}
        if !self.sources.contains_key(&source.message_id)&&self.sources.len()>=MAX_SOURCES{
            return Err(VoiceError::new(VoiceErrorKind::Backlog,"canonical speech source retention budget reached"));
        }
        self.sources.insert(source.message_id.clone(),(source,generation));Ok(true)
    }
    pub fn allows(&self,source:&VoiceSpeechSourceRef,generation:u64)->bool{
        self.revoked.get(&source.message_id).is_none_or(|revision|*revision<source.revision)
            &&self.sources.get(&source.message_id).is_some_and(|(current,current_generation)|current==source&&*current_generation==generation)
    }
    pub fn references(&self)->Vec<VoiceSpeechSourceRef>{self.sources.values().map(|(source,_)|source.clone()).collect()}
    pub fn revoke(&mut self,message:&str,revision:u64,_generation:u64)->Result<bool,VoiceError>{
        if !self.revoked.contains_key(message)&&self.revoked.len()>=MAX_REVOCATIONS{
            return Err(VoiceError::new(VoiceErrorKind::Backlog,"canonical speech revocation retention budget reached"));
        }
        self.revoked.entry(message.into()).and_modify(|prior|*prior=(*prior).max(revision)).or_insert(revision);
        let invalid=self.sources.get(message).is_some_and(|(source,_)|source.revision<=revision);
        if invalid{self.sources.remove(message);}
        Ok(invalid)
    }
}

#[cfg(test)]mod tests{
    use super::*;
    fn source(id:&str,revision:u64)->VoiceSpeechSourceRef{VoiceSpeechSourceRef{message_id:id.into(),revision,through_seq:revision}}
    #[test]fn earlier_source_is_revoked_after_later_source_and_inflight_validation_cannot_restore_it(){
        let mut ledger=SpeechSourceLedger::default();let a=source("canonical-a",10);let b=source("canonical-b",20);
        assert!(ledger.register(a.clone(),1).unwrap());assert!(ledger.register(b.clone(),1).unwrap());
        assert!(ledger.revoke("canonical-a",11,1).unwrap());
        assert!(!ledger.allows(&a,1));assert!(ledger.allows(&b,1));assert!(!ledger.register(a,1).unwrap());
        assert!(ledger.register(source("canonical-a",12),2).unwrap());assert!(!ledger.allows(&b,2));
    }
    #[test]fn revocation_before_observation_and_out_of_order_revisions_never_publish_stale_speech(){
        let mut ledger=SpeechSourceLedger::default();assert!(!ledger.revoke("a",3,1).unwrap());
        assert!(!ledger.register(source("a",2),1).unwrap());assert!(ledger.register(source("a",4),1).unwrap());
        assert!(!ledger.register(source("a",3),1).unwrap());assert!(ledger.allows(&source("a",4),1));
        assert!(ledger.revoke("a",4,2).unwrap(),"cached source remains revocable across output generations");
        assert!(!ledger.revoke("a",4,2).unwrap(),"the same invalidated source cannot repeatedly rebuild output");
    }
}
