//! Durable voice input facts. Canonical work status remains with its own owner.
use super::*;
use nomifun_voice_contracts::WorkSteeringPolicy;

pub(super) const SCHEMA:&str="
CREATE TABLE IF NOT EXISTS voice_pending_intents (
 operation_key TEXT PRIMARY KEY, owner_id TEXT NOT NULL, agent_session_id TEXT NOT NULL,
 binding_version INTEGER NOT NULL, context_floor INTEGER, work_steering_policy TEXT NOT NULL DEFAULT 'safe_boundary', text TEXT NOT NULL, revision INTEGER NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('queued','dispatched','cancelled','deferred')),
 canonical_operation_id TEXT, canonical_receipt_id TEXT, created_ms INTEGER NOT NULL,
 FOREIGN KEY(operation_key) REFERENCES voice_work_links(operation_key) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS voice_pending_controls (
 operation_key TEXT PRIMARY KEY, pending_key TEXT NOT NULL, command_json TEXT NOT NULL,
 FOREIGN KEY(pending_key) REFERENCES voice_pending_intents(operation_key) ON DELETE CASCADE);";

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum VoicePendingPhase {Queued,Dispatched,Cancelled,Deferred}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub struct VoicePendingIntent {
    pub operation_key:String,pub owner_id:String,pub agent_session_id:String,pub binding_version:u64,
    pub text:String,pub revision:u64,pub phase:VoicePendingPhase,
    pub canonical_operation_id:Option<String>,pub canonical_receipt_id:Option<String>,
    #[serde(default)]pub context_floor:Option<u64>,
    #[serde(default)]pub work_steering_policy:WorkSteeringPolicy,
}
pub(super) fn ensure_schema(conn:&Connection)->Result<(),VoiceError> {
    conn.execute_batch(SCHEMA).map_err(failure)?;
    let columns=conn.prepare("PRAGMA table_info(voice_pending_intents)").map_err(failure)?.query_map([],|row|row.get::<_,String>(1)).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure)?;
    if !columns.iter().any(|column|column=="context_floor") {conn.execute_batch("ALTER TABLE voice_pending_intents ADD COLUMN context_floor INTEGER;").map_err(failure)?;}
    if !columns.iter().any(|column|column=="work_steering_policy") {conn.execute_batch("ALTER TABLE voice_pending_intents ADD COLUMN work_steering_policy TEXT NOT NULL DEFAULT 'safe_boundary';").map_err(failure)?;}
    conn.execute("UPDATE voice_pending_intents SET phase='deferred' WHERE phase='queued' AND context_floor IS NULL",[]).map_err(failure)?;Ok(())
}
type Row=(String,String,String,u64,String,u64,String,Option<String>,Option<String>,Option<u64>,String);
fn decode(row:Row)->Result<VoicePendingIntent,VoiceError> {
    Ok(VoicePendingIntent {operation_key:row.0,owner_id:row.1,agent_session_id:row.2,binding_version:row.3,text:row.4,revision:row.5,
        phase:match row.6.as_str(){"queued"=>VoicePendingPhase::Queued,"dispatched"=>VoicePendingPhase::Dispatched,"cancelled"=>VoicePendingPhase::Cancelled,"deferred"=>VoicePendingPhase::Deferred,_=>return Err(failure("invalid pending input phase"))},
        canonical_operation_id:row.7,canonical_receipt_id:row.8,context_floor:row.9,work_steering_policy:serde_json::from_value(serde_json::Value::String(row.10)).map_err(failure)?})
}
fn read(conn:&Connection,key:&str)->Result<Option<VoicePendingIntent>,VoiceError> {
    conn.query_row("SELECT operation_key,owner_id,agent_session_id,binding_version,text,revision,phase,canonical_operation_id,canonical_receipt_id,context_floor,work_steering_policy FROM voice_pending_intents WHERE operation_key=?1",[key],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?)))
        .optional().map_err(failure)?.map(decode).transpose()
}
fn owns(input:&VoicePendingIntent,owner:&str,session:&str)->Result<(),VoiceError> {
    if input.owner_id!=owner||input.agent_session_id!=session{return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice input belongs to another owner or Session"));}Ok(())
}
fn fact(tx:&rusqlite::Transaction<'_>,key:&str,event_key:&str,kind:&str,value:&impl Serialize)->Result<(),VoiceError> {
    let body=serde_json::to_string(value).map_err(failure)?;
    let changed=tx.execute("INSERT OR IGNORE INTO voice_facts(voice_session_id,epoch,event_key,kind,correlation_id,created_ms,payload) SELECT voice_session_id,epoch,?2,?3,?1,?4,?5 FROM voice_work_links WHERE operation_key=?1",
        params![key,event_key,kind,now_ms(),body]).map_err(failure)?;
    if changed==0&&!tx.query_row("SELECT EXISTS(SELECT 1 FROM voice_facts WHERE event_key=?1 AND payload=?2)",params![event_key,body],|row|row.get::<_,bool>(0)).map_err(failure)? {
        return Err(failure("voice input fact has no original activation link"));
    }Ok(())
}

impl VoiceJournal {
    pub async fn record_pending_intent(&self,owner:String,session:String,binding:u64,context_floor:u64,key:String,text:String)->Result<(VoicePendingIntent,bool),VoiceError> {
        self.record_pending_intent_with_policy(owner,session,binding,context_floor,key,text,WorkSteeringPolicy::SafeBoundary).await
    }
    pub async fn record_pending_intent_with_policy(&self,owner:String,session:String,binding:u64,context_floor:u64,key:String,text:String,policy:WorkSteeringPolicy)->Result<(VoicePendingIntent,bool),VoiceError> {
        if key.is_empty()||key.len()>256||binding==0||text.trim().is_empty()||text.len()>16*1024{return Err(VoiceError::new(VoiceErrorKind::Configuration,"voice input must be complete and bounded"));}
        self.run(move|conn|{
            let tx=conn.transaction().map_err(failure)?;
            if let Some(input)=read(&tx,&key)? {owns(&input,&owner,&session)?;
                // Original wording remains in the first immutable fact. A
                // correction is a separate operation, never a changed replay.
                let original:String=tx.query_row("SELECT json_extract(payload,'$.text') FROM voice_facts WHERE event_key=?1",[format!("pending:{key}:1")],|row|row.get(0)).map_err(failure)?;
                if input.binding_version!=binding||input.context_floor!=Some(context_floor)||input.work_steering_policy!=policy||original!=text{return Err(failure("voice input replay differs from the original input or frozen steering policy"));}tx.commit().map_err(failure)?;return Ok((input,true));
            }
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM voice_work_links l JOIN voice_activations a ON a.voice_session_id=l.voice_session_id AND a.epoch=l.epoch WHERE l.operation_key=?1 AND l.quarantined=0 AND a.owner_id=?2 AND a.agent_session_id=?3 AND a.binding_version=?4 AND a.ended_ms IS NULL)",params![key,owner,session,binding],|row|row.get(0)).map_err(failure)?;
            if !valid{return Err(VoiceError::new(VoiceErrorKind::StaleEpoch,"voice input lacks a live original activation and committed trigger"));}
            let count:i64=tx.query_row("SELECT COUNT(*) FROM voice_pending_intents WHERE owner_id=?1 AND agent_session_id=?2 AND phase='queued'",params![owner,session],|row|row.get(0)).map_err(failure)?;
            if count>=32{return Err(VoiceError::new(VoiceErrorKind::Backlog,"voice pending input limit reached"));}
            let policy=serde_json::to_value(policy).map_err(failure)?.as_str().ok_or_else(||failure("invalid steering policy"))?.to_owned();
            tx.execute("INSERT INTO voice_pending_intents(operation_key,owner_id,agent_session_id,binding_version,context_floor,text,revision,phase,created_ms,work_steering_policy) VALUES (?1,?2,?3,?4,?5,?6,1,'queued',?7,?8)",params![key,owner,session,binding,context_floor,text,now_ms(),policy]).map_err(failure)?;
            let input=read(&tx,&key)?.ok_or_else(||failure("pending input insert was not observed"))?;
            fact(&tx,&key,&format!("pending:{key}:1"),"work_input_queued",&input)?;tx.commit().map_err(failure)?;Ok((input,false))
        }).await
    }
    pub async fn pending_intent(&self,owner:String,session:String,key:String)->Result<Option<VoicePendingIntent>,VoiceError> {
        self.run(move|conn|{let input=read(conn,&key)?;if let Some(input)=&input{owns(input,&owner,&session)?;}Ok(input)}).await
    }
    /// Original dataset provenance belongs to the input's activation, not the
    /// current profile/credential lease. Old or incomplete facts remain unknown;
    /// recovery must never backfill them from the current installation.
    pub async fn original_input_namespace(&self,owner:String,session:String,key:String)->Result<Option<String>,VoiceError> {
        self.run(move|conn|{
            let Some(input)=read(conn,&key)? else{return Ok(None);};owns(&input,&owner,&session)?;
            let lease:Option<String>=conn.query_row("SELECT a.lease_revision FROM voice_work_links l JOIN voice_activations a ON a.voice_session_id=l.voice_session_id AND a.epoch=l.epoch WHERE l.operation_key=?1 AND l.quarantined=0 AND a.owner_id=?2 AND a.agent_session_id=?3",
                params![key,owner,session],|row|row.get(0)).optional().map_err(failure)?.flatten();
            let Some(lease)=lease else{return Ok(None);};let mut parts=lease.split(':');
            let (Some("ns"),Some(namespace),Some(full),None)=(parts.next(),parts.next(),parts.next(),parts.next()) else{return Ok(None);};
            let valid=|value:&str|value.len()==64&&value.bytes().all(|byte|byte.is_ascii_digit()||matches!(byte,b'a'..=b'f'));
            Ok((valid(namespace)&&valid(full)).then(||namespace.to_owned()))
        }).await
    }
    pub async fn queued_intents(&self)->Result<Vec<VoicePendingIntent>,VoiceError> {
        self.run(|conn|{let mut statement=conn.prepare("SELECT operation_key FROM voice_pending_intents WHERE phase='queued' ORDER BY created_ms,operation_key LIMIT 1024").map_err(failure)?;
            let keys=statement.query_map([],|row|row.get::<_,String>(0)).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure)?;
            keys.into_iter().map(|key|read(conn,&key)?.ok_or_else(||failure("queued fact disappeared"))).collect()}).await
    }
    /// Freeze wording before crossing into the canonical owner. This fact does
    /// not assert admission, running, completion, or any tool result.
    pub async fn begin_dispatch_intent(&self,owner:String,session:String,key:String,revision:u64)->Result<bool,VoiceError> {
        self.run(move|conn|{let tx=conn.transaction().map_err(failure)?;let input=read(&tx,&key)?.ok_or_else(||failure("unknown pending input"))?;owns(&input,&owner,&session)?;
            if input.phase!=VoicePendingPhase::Queued||input.revision!=revision {tx.commit().map_err(failure)?;return Ok(false);}
            tx.execute("UPDATE voice_pending_intents SET phase='dispatched' WHERE operation_key=?1",[&key]).map_err(failure)?;
            let input=read(&tx,&key)?.ok_or_else(||failure("dispatch fact disappeared"))?;
            fact(&tx,&key,&format!("dispatch-request:{key}:{}",uuid::Uuid::now_v7()),"work_input_dispatch_requested",&input)?;
            tx.commit().map_err(failure)?;Ok(true)}).await
    }
    /// Only the typed None result from the app owner may return an input to
    /// queued: no canonical admission was attempted at that boundary.
    pub async fn requeue_without_admission(&self,owner:String,session:String,key:String,revision:u64)->Result<(),VoiceError> {
        self.run(move|conn|{let tx=conn.transaction().map_err(failure)?;let input=read(&tx,&key)?.ok_or_else(||failure("unknown pending input"))?;owns(&input,&owner,&session)?;
            if input.phase!=VoicePendingPhase::Dispatched||input.revision!=revision||input.canonical_operation_id.is_some(){return Err(failure("dispatch cannot be returned to queue after a canonical reference"));}
            tx.execute("UPDATE voice_pending_intents SET phase='queued' WHERE operation_key=?1",[&key]).map_err(failure)?;
            let input=read(&tx,&key)?.ok_or_else(||failure("queued fact disappeared"))?;
            fact(&tx,&key,&format!("no-admission:{key}:{}",uuid::Uuid::now_v7()),"work_input_boundary_wait",&input)?;tx.commit().map_err(failure)?;Ok(())}).await
    }
    pub async fn mark_dispatched(&self,owner:String,session:String,key:String,revision:u64,operation:String,receipt:String)->Result<(),VoiceError> {
        self.run(move|conn|{let tx=conn.transaction().map_err(failure)?;let input=read(&tx,&key)?.ok_or_else(||failure("unknown pending input"))?;owns(&input,&owner,&session)?;
            if input.phase==VoicePendingPhase::Dispatched&&input.canonical_operation_id.is_some() {
                if input.canonical_operation_id.as_deref()==Some(operation.as_str())&&input.canonical_receipt_id.as_deref()==Some(receipt.as_str()){tx.commit().map_err(failure)?;return Ok(());}
                return Err(failure("voice input canonical reference differs"));
            }
            if input.phase!=VoicePendingPhase::Dispatched||input.revision!=revision||operation.is_empty()||receipt.is_empty(){return Err(failure("voice input was closed or revised before dispatch"));}
            tx.execute("UPDATE voice_pending_intents SET phase='dispatched',canonical_operation_id=?2,canonical_receipt_id=?3 WHERE operation_key=?1",params![key,operation,receipt]).map_err(failure)?;
            let input=read(&tx,&key)?.ok_or_else(||failure("dispatched input disappeared"))?;
            fact(&tx,&key,&format!("pending:{key}:dispatched"),"work_input_dispatched",&input)?;tx.commit().map_err(failure)?;Ok(())}).await
    }
    /// Caller owns the per-input dispatch lock. These are input facts, never
    /// a canonical cancel or a claim that a running task has terminated.
    pub async fn control_pending_intent(&self,owner:String,session:String,key:String,command_key:String,revised:Option<(u64,String)>,deferred:bool)->Result<(VoicePendingIntent,bool),VoiceError> {
        self.run(move|conn|{let tx=conn.transaction().map_err(failure)?;let input=read(&tx,&key)?.ok_or_else(||failure("unknown voice input"))?;owns(&input,&owner,&session)?;
            let command=serde_json::to_string(&(revised.clone(),deferred)).map_err(failure)?;
            if let Some((original_key,original))=tx.query_row("SELECT pending_key,command_json FROM voice_pending_controls WHERE operation_key=?1",[&command_key],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).optional().map_err(failure)? {
                if original_key!=key||original!=command{return Err(failure("voice input control replay differs"));}tx.commit().map_err(failure)?;return Ok((input,true));
            }
            if input.phase!=VoicePendingPhase::Queued{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice input was already dispatched or closed"));}
            if let Some((expected,text))=revised {
                if input.revision!=expected||text.trim().is_empty()||text.len()>16*1024{return Err(failure("voice input correction lacks its exact revision or complete bounded wording"));}
                tx.execute("UPDATE voice_pending_intents SET text=?2,revision=revision+1 WHERE operation_key=?1",params![key,text]).map_err(failure)?;
            }else {tx.execute("UPDATE voice_pending_intents SET phase=?2 WHERE operation_key=?1",params![key,if deferred{"deferred"}else{"cancelled"}]).map_err(failure)?;}
            tx.execute("INSERT INTO voice_pending_controls(operation_key,pending_key,command_json) VALUES (?1,?2,?3)",params![command_key,key,command]).map_err(failure)?;
            let input=read(&tx,&key)?.ok_or_else(||failure("controlled input disappeared"))?;fact(&tx,&key,&format!("pending-control:{command_key}"),"work_input_control",&input)?;
            tx.commit().map_err(failure)?;Ok((input,false))}).await
    }
    pub async fn pending_control(&self,owner:String,session:String,command:String)->Result<Option<VoicePendingIntent>,VoiceError> {
        self.run(move|conn|{let key=conn.query_row("SELECT pending_key FROM voice_pending_controls WHERE operation_key=?1",[command],|row|row.get::<_,String>(0)).optional().map_err(failure)?;
            let input=key.map(|key|read(conn,&key)).transpose()?.flatten();if let Some(input)=&input{owns(input,&owner,&session)?;}Ok(input)}).await
    }
    pub async fn defer_intent(&self,owner:String,session:String,key:String)->Result<(),VoiceError> {
        self.run(move|conn|{let tx=conn.transaction().map_err(failure)?;let input=read(&tx,&key)?.ok_or_else(||failure("unknown input"))?;owns(&input,&owner,&session)?;
            if input.phase==VoicePendingPhase::Cancelled||input.canonical_operation_id.is_some(){return Err(failure("referenced or cancelled input cannot be deferred"));}
            tx.execute("UPDATE voice_pending_intents SET phase='deferred' WHERE operation_key=?1",[&key]).map_err(failure)?;
            let input=read(&tx,&key)?.ok_or_else(||failure("deferred fact disappeared"))?;fact(&tx,&key,&format!("deferred:{key}"),"work_input_deferred",&input)?;tx.commit().map_err(failure)?;Ok(())}).await
    }
}
