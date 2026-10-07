use super::*;
use nomifun_voice_contracts::{VoiceLocalFactRef,VoiceLocalReplay,VoiceLocalReplayEntry,VoiceReplayScope};
use std::collections::BTreeMap;

pub(super) fn ensure_schema(conn:&Connection)->Result<(),VoiceError>{
    let mut statement=conn.prepare("PRAGMA table_info(voice_activations)").map_err(failure)?;
    let columns=statement.query_map([],|row|row.get::<_,String>(1)).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure)?;
    if !columns.iter().any(|column|column=="context_floor"){conn.execute_batch("ALTER TABLE voice_activations ADD COLUMN context_floor INTEGER;").map_err(failure)?;}
    if !columns.iter().any(|column|column=="lease_revision"){conn.execute_batch("ALTER TABLE voice_activations ADD COLUMN lease_revision TEXT;").map_err(failure)?;}
    Ok(())
}
#[cfg(test)]mod tests{
    use super::*;
    #[tokio::test]async fn recovery_requires_exact_scope_latest_committed_revision_and_real_consumption(){
        let dir=tempfile::tempdir().unwrap();let journal=VoiceJournal::open(dir.path()).unwrap();
        let scope=VoiceReplayScope{agent_session_id:"session".into(),binding_version:2,context_floor:9};
        for(id,owner,binding,floor)in [("current","owner",2,Some(9)),("old-binding","owner",1,Some(9)),("old-floor","owner",2,Some(0)),("unknown","owner",2,None),("foreign","other",2,Some(9))]{
            journal.activate(VoiceActivationFact{voice_session_id:id.into(),epoch:1,owner_id:owner.into(),agent_session_id:"session".into(),binding_version:binding,route_digest:"a".repeat(64),started_ms:1,context_floor:floor,lease_revision:Some("lease".into())}).await.unwrap();
            let fragment=TranscriptFragment{speaker:VoiceSpeaker::User,fragment_id:"user-1".into(),revision:1,commit:TranscriptCommit::Committed,text:id.into(),media_range:None};
            journal.append(id.into(),1,"user:1".into(),"voice_event".into(),None,serde_json::to_value(VoiceModelEvent::Transcript{fragment}).unwrap()).await.unwrap();
        }
        let update=TranscriptFragment{speaker:VoiceSpeaker::User,fragment_id:"user-1".into(),revision:2,commit:TranscriptCommit::Committed,text:"revised exact user wording".into(),media_range:None};
        journal.append("current".into(),1,"user:2".into(),"voice_event".into(),None,serde_json::to_value(VoiceModelEvent::Transcript{fragment:update.clone()}).unwrap()).await.unwrap();
        let generated=TranscriptFragment{speaker:VoiceSpeaker::Assistant,fragment_id:"assistant".into(),revision:1,commit:TranscriptCommit::Committed,text:"generated words must never count as heard".into(),media_range:None};
        journal.append("current".into(),1,"assistant".into(),"voice_event".into(),None,serde_json::to_value(VoiceModelEvent::Transcript{fragment:generated}).unwrap()).await.unwrap();
        for(state,consumed_us)in [(DeliveryState::Queued,0),(DeliveryState::Played,5000),(DeliveryState::Interrupted,9000)]{
            let receipt=PlaybackReceipt{activation_epoch:1,segment_id:"segment".into(),revision:1,output_generation:1,state,consumed_us,uncertain_tail_us:1000,precision:PlaybackPrecision::Estimated};
            journal.append_playback("current".into(),1,format!("play:{consumed_us}"),receipt).await.unwrap();
        }
        let replay=journal.local_replay("owner".into(),scope.clone(),"lease".into()).await.unwrap();assert_eq!(replay.scope,scope);assert_eq!(replay.entries.len(),2);
        assert!(replay.entries.iter().any(|entry|matches!(entry,VoiceLocalReplayEntry::UserCommitted{fragment,..} if fragment==&update)));
        assert!(replay.entries.iter().any(|entry|matches!(entry,VoiceLocalReplayEntry::Playback{receipt,..} if receipt.consumed_us==9000)));
        let json=serde_json::to_string(&replay).unwrap();assert!(!json.contains("generated words"));assert!(!json.contains("canonical_receipt_id"));assert!(!json.contains("old-floor"));
        assert!(journal.local_replay("owner".into(),scope.clone(),"restored-namespace-lease".into()).await.unwrap().entries.is_empty());
        assert!(journal.local_replay("owner".into(),VoiceReplayScope{context_floor:10,..scope},"lease".into()).await.unwrap().entries.is_empty());
    }
}
impl VoiceJournal {
    /// A bounded voice-local window, never canonical task context or authority.
    /// Unknown historical scope is excluded without converting Agent lineage.
    pub async fn local_replay(&self,owner:String,scope:VoiceReplayScope,lease_revision:String)->Result<VoiceLocalReplay,VoiceError>{
        self.run(move|conn|{
            let mut statement=conn.prepare("SELECT f.sequence,f.voice_session_id,f.epoch,f.kind,f.payload FROM voice_facts f JOIN voice_activations a ON a.voice_session_id=f.voice_session_id AND a.epoch=f.epoch WHERE a.owner_id=?1 AND a.agent_session_id=?2 AND a.binding_version=?3 AND a.context_floor=?4 AND a.lease_revision=?5 AND f.kind IN ('voice_event','playback') ORDER BY f.sequence DESC LIMIT 256").map_err(failure)?;
            let rows=statement.query_map(params![owner,scope.agent_session_id.as_ref(),scope.binding_version,scope.context_floor,lease_revision],|row|Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?,row.get::<_,u64>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?))).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure)?;
            let mut users=BTreeMap::<(String,u64,String),(VoiceLocalFactRef,TranscriptFragment)>::new();
            let mut played=BTreeMap::<(String,u64,String),(VoiceLocalFactRef,PlaybackReceipt)>::new();
            for(sequence,voice,epoch,kind,payload)in rows{
                let origin=VoiceLocalFactRef{voice_session_id:voice.clone(),activation_epoch:epoch,journal_sequence:sequence};
                if kind=="voice_event"{
                    if let VoiceModelEvent::Transcript{fragment}=serde_json::from_str(&payload).map_err(failure)?{
                        if fragment.speaker!=VoiceSpeaker::User{continue;}
                        let key=(voice,epoch,fragment.fragment_id.clone());
                        if users.get(&key).is_none_or(|(_,previous)|fragment.revision>previous.revision){users.insert(key,(origin,fragment));}
                    }
                }else{
                    let receipt:PlaybackReceipt=serde_json::from_str(&payload).map_err(failure)?;
                    if receipt.consumed_us==0||!matches!(receipt.state,DeliveryState::Played|DeliveryState::Interrupted){continue;}
                    let key=(voice,epoch,receipt.segment_id.clone());
                    if played.get(&key).is_none_or(|(_,previous)|receipt.consumed_us>previous.consumed_us){played.insert(key,(origin,receipt));}
                }
            }
            let mut entries=users.into_values().filter(|(_,fragment)|fragment.commit==TranscriptCommit::Committed).map(|(origin,fragment)|VoiceLocalReplayEntry::UserCommitted{origin,fragment})
                .chain(played.into_values().map(|(origin,receipt)|VoiceLocalReplayEntry::Playback{origin,receipt})).collect::<Vec<_>>();
            let sequence=|entry:&VoiceLocalReplayEntry|match entry{VoiceLocalReplayEntry::UserCommitted{origin,..}|VoiceLocalReplayEntry::Playback{origin,..}=>origin.journal_sequence};
            entries.sort_by_key(|entry|std::cmp::Reverse(sequence(entry)));
            let mut selected=Vec::new();let mut bytes=0;
            for entry in entries.into_iter().take(64){let length=serde_json::to_vec(&entry).map_err(failure)?.len();if bytes+length>60*1024{break;}bytes+=length;selected.push(entry);}
            selected.reverse();Ok(VoiceLocalReplay{scope,entries:selected})
        }).await
    }
}
