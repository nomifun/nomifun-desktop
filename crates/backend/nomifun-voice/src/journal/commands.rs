use super::*;
impl VoiceJournal {
    /// An explicit UI command has immutable arguments under its retry key. This
    /// is not a fabricated provider tool call or a transcription revision.
    pub async fn reserve_user_command(&self,id:String,epoch:u64,key:String,digest:String)->Result<(VoiceWorkLink,bool),VoiceError>{
        if key.trim().is_empty()||key.len()>256{return Err(VoiceError::new(VoiceErrorKind::Configuration,"invalid voice UI command key"));}
        let trigger=format!("ui:{key}");
        self.run(move|conn|{
            let tx=conn.transaction().map_err(failure)?;
            let active:bool=tx.query_row("SELECT ended_ms IS NULL FROM voice_activations WHERE voice_session_id=?1 AND epoch=?2",params![id,epoch],|row|row.get(0)).optional().map_err(failure)?.unwrap_or(false);
            if !active{return Err(VoiceError::new(VoiceErrorKind::Closed,"voice activation closed"));}
            let prior:Option<String>=tx.query_row("SELECT input_revision FROM voice_work_links WHERE voice_session_id=?1 AND epoch=?2 AND upstream_trigger_id=?3 LIMIT 1",params![id,epoch,trigger],|row|row.get(0)).optional().map_err(failure)?;
            if prior.is_some_and(|prior|prior!=digest){return Err(VoiceError::new(VoiceErrorKind::Configuration,"voice UI retry key reused with different arguments"));}
            let operation_key=format!("voice:{}",nomifun_agent_contracts::digest_payload(&(id.as_str(),epoch,trigger.as_str(),digest.as_str())).map_err(failure)?.0);
            let fresh=tx.execute("INSERT INTO voice_work_links(voice_session_id,epoch,upstream_trigger_id,input_revision,operation_key) VALUES (?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![id,epoch,trigger,digest,operation_key]).map_err(failure)?==1;
            let link=tx.query_row("SELECT l.operation_key,l.canonical_receipt_id,l.quarantined,a.owner_id,a.agent_session_id FROM voice_work_links l JOIN voice_activations a ON a.voice_session_id=l.voice_session_id AND a.epoch=l.epoch WHERE l.operation_key=?1",[operation_key],|row|Ok(VoiceWorkLink{operation_key:row.get(0)?,canonical_receipt_id:row.get(1)?,quarantined:row.get(2)?,owner_id:row.get(3)?,agent_session_id:row.get(4)?})).map_err(failure)?;
            tx.commit().map_err(failure)?;Ok((link,fresh))
        }).await
    }
}
