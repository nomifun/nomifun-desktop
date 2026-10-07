use super::*;
pub(super) const SCHEMA:&str="CREATE TABLE IF NOT EXISTS voice_source_intents (
 voice_session_id TEXT NOT NULL,epoch INTEGER NOT NULL,source_digest TEXT NOT NULL,
 original_operation_key TEXT NOT NULL,
 PRIMARY KEY(voice_session_id,epoch,source_digest),
 FOREIGN KEY(original_operation_key) REFERENCES voice_work_links(operation_key) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS voice_source_trigger_aliases (
 voice_session_id TEXT NOT NULL,epoch INTEGER NOT NULL,upstream_trigger_id TEXT NOT NULL,input_revision TEXT NOT NULL,
 original_operation_key TEXT NOT NULL,
 PRIMARY KEY(voice_session_id,epoch,upstream_trigger_id,input_revision),
 FOREIGN KEY(original_operation_key) REFERENCES voice_work_links(operation_key) ON DELETE CASCADE);";
impl VoiceJournal{
    /// A retrying model may produce another call ID. The same committed source
    /// and same request still refer to one original canonical admission key.
    pub async fn reserve_source_trigger(&self,voice:String,epoch:u64,trigger:String,revision:String,source_digest:String)->Result<(VoiceWorkLink,bool),VoiceError>{
        if source_digest!=revision||source_digest.len()!=64||!source_digest.bytes().all(|b|b.is_ascii_hexdigit()){
            return Err(failure("source intent requires its exact normalized input digest"));
        }
        self.run(move|conn|{
            let tx=conn.transaction().map_err(failure)?;
            let active:bool=tx.query_row("SELECT ended_ms IS NULL FROM voice_activations WHERE voice_session_id=?1 AND epoch=?2",params![voice,epoch],|row|row.get(0)).optional().map_err(failure)?.unwrap_or(false);
            if !active{return Err(failure("voice activation closed"));}
            let original:Option<String>=tx.query_row("SELECT original_operation_key FROM voice_source_intents WHERE voice_session_id=?1 AND epoch=?2 AND source_digest=?3",params![voice,epoch,source_digest],|row|row.get(0)).optional().map_err(failure)?;
            let mut fresh=false;
            let key=match original{Some(key)=>key,None=>{
                let key=format!("voice:{}",nomifun_agent_contracts::digest_payload(&(voice.as_str(),epoch,trigger.as_str(),revision.as_str())).map_err(failure)?.as_ref());
                fresh=tx.execute("INSERT INTO voice_work_links(voice_session_id,epoch,upstream_trigger_id,input_revision,operation_key) VALUES (?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![voice,epoch,trigger,revision,key]).map_err(failure)?==1;
                tx.execute("INSERT INTO voice_source_intents(voice_session_id,epoch,source_digest,original_operation_key) VALUES (?1,?2,?3,?4)",params![voice,epoch,source_digest,key]).map_err(failure)?;key
            }};
            tx.execute("INSERT INTO voice_source_trigger_aliases(voice_session_id,epoch,upstream_trigger_id,input_revision,original_operation_key) VALUES (?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![voice,epoch,trigger,revision,key]).map_err(failure)?;
            let alias:String=tx.query_row("SELECT original_operation_key FROM voice_source_trigger_aliases WHERE voice_session_id=?1 AND epoch=?2 AND upstream_trigger_id=?3 AND input_revision=?4",params![voice,epoch,trigger,revision],|row|row.get(0)).map_err(failure)?;
            if alias!=key{return Err(failure("source trigger alias conflicts with its immutable original operation"));}
            let link=tx.query_row("SELECT l.operation_key,l.canonical_receipt_id,l.quarantined,a.owner_id,a.agent_session_id FROM voice_work_links l JOIN voice_activations a ON a.voice_session_id=l.voice_session_id AND a.epoch=l.epoch WHERE l.operation_key=?1",[key],|row|Ok(VoiceWorkLink{operation_key:row.get(0)?,canonical_receipt_id:row.get(1)?,quarantined:row.get(2)?,owner_id:row.get(3)?,agent_session_id:row.get(4)?})).map_err(failure)?;
            tx.commit().map_err(failure)?;Ok((link,fresh))
        }).await
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    async fn fixture()->(tempfile::TempDir,VoiceJournal){
        let dir=tempfile::tempdir().unwrap();let journal=VoiceJournal::open(dir.path()).unwrap();
        journal.activate(VoiceActivationFact{voice_session_id:"voice".into(),epoch:1,owner_id:"owner".into(),agent_session_id:"session".into(),binding_version:1,context_floor:Some(0),lease_revision:Some("lease".into()),route_digest:"b".repeat(64),started_ms:now_ms()}).await.unwrap();(dir,journal)
    }
    #[tokio::test]
    async fn changed_model_call_identity_preserves_one_original_receipt_and_durable_aliases(){
        let(dir,journal)=fixture().await;let revision="a".repeat(64);
        let(first,fresh)=journal.reserve_source_trigger("voice".into(),1,"call-A".into(),revision.clone(),revision.clone()).await.unwrap();assert!(fresh);
        journal.associate_receipt(first.operation_key.clone(),"canonical-original".into()).await.unwrap();
        let(alias,fresh)=journal.reserve_source_trigger("voice".into(),1,"call-B".into(),revision.clone(),revision.clone()).await.unwrap();assert!(!fresh);assert_eq!(alias.operation_key,first.operation_key);assert_eq!(alias.canonical_receipt_id.as_deref(),Some("canonical-original"));
        let count:i64=journal.run(|conn|conn.query_row("SELECT COUNT(*) FROM voice_work_links",[],|r|r.get(0)).map_err(failure)).await.unwrap();assert_eq!(count,1);
        drop(journal);let journal=VoiceJournal::open(dir.path()).unwrap();
        let key=first.operation_key;let count:i64=journal.run(move|conn|conn.query_row("SELECT COUNT(*) FROM voice_source_trigger_aliases WHERE original_operation_key=?1",[key],|r|r.get(0)).map_err(failure)).await.unwrap();assert_eq!(count,2);
        assert!(journal.reserve_source_trigger("voice".into(),1,"after-restart".into(),revision.clone(),revision).await.is_err(),"recovery never revives the old media/input lease");
        assert!(!dir.path().join("nomifun-backend.db").exists());
    }
    #[tokio::test]
    async fn unconfirmed_original_reservation_never_becomes_a_fresh_admission_by_new_call_id(){
        let(_dir,journal)=fixture().await;let revision="a".repeat(64);
        let(original,_)=journal.reserve_trigger("voice".into(),1,"original".into(),revision.clone()).await.unwrap();
        let(adopted,fresh)=journal.reserve_source_trigger("voice".into(),1,"original".into(),revision.clone(),revision.clone()).await.unwrap();assert!(!fresh);assert_eq!(original.operation_key,adopted.operation_key);
        let(alias,fresh)=journal.reserve_source_trigger("voice".into(),1,"new-call".into(),revision.clone(),revision).await.unwrap();assert!(!fresh);assert_eq!(alias.operation_key,original.operation_key);assert!(alias.canonical_receipt_id.is_none());
        journal.quarantine(original.operation_key).await.unwrap();
        let(alias,fresh)=journal.reserve_source_trigger("voice".into(),1,"third-call".into(),"a".repeat(64),"a".repeat(64)).await.unwrap();assert!(!fresh);assert!(alias.quarantined);
        assert!(journal.reserve_source_trigger("voice".into(),1,"bad-proof".into(),"a".repeat(64),"b".repeat(64)).await.is_err());
    }
}
