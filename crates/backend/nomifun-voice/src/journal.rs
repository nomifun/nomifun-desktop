//! Voice facts in a separate database. Canonical work receipts are references, never execution authority.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use nomifun_voice_contracts::voice::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[path="journal/pending.rs"]
mod pending;
pub use pending::{VoicePendingIntent,VoicePendingPhase};
#[path="journal/data.rs"]
mod data;
pub use data::VoiceSnapshotExport;
#[cfg(test)]
#[path="journal/data_tests.rs"]
mod data_tests;
#[cfg(test)]
#[path="journal/pending_tests.rs"]
mod pending_tests;
#[path="journal/profiles.rs"]mod profiles;
#[path="journal/commands.rs"]mod commands;
#[path="journal/replay.rs"]mod replay;
#[path="journal/source_intents.rs"]mod source_intents;

const SCHEMA_VERSION: i64 = 6;
const MAX_FACT_BYTES: usize = 256 * 1024;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS voice_activations (
 voice_session_id TEXT NOT NULL, epoch INTEGER NOT NULL, owner_id TEXT NOT NULL,
 agent_session_id TEXT NOT NULL, binding_version INTEGER NOT NULL, route_digest TEXT NOT NULL,
 started_ms INTEGER NOT NULL, ended_ms INTEGER, end_reason TEXT,
 PRIMARY KEY(voice_session_id, epoch));
CREATE TABLE IF NOT EXISTS voice_facts (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, voice_session_id TEXT NOT NULL, epoch INTEGER NOT NULL,
 event_key TEXT NOT NULL, kind TEXT NOT NULL, correlation_id TEXT, created_ms INTEGER NOT NULL,
 payload TEXT NOT NULL, UNIQUE(voice_session_id, epoch, event_key),
 FOREIGN KEY(voice_session_id,epoch) REFERENCES voice_activations(voice_session_id,epoch) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS voice_work_links (
 voice_session_id TEXT NOT NULL, epoch INTEGER NOT NULL, upstream_trigger_id TEXT NOT NULL,
 input_revision TEXT NOT NULL, operation_key TEXT NOT NULL UNIQUE, canonical_receipt_id TEXT,
 quarantined INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(voice_session_id,epoch,upstream_trigger_id,input_revision),
 FOREIGN KEY(voice_session_id,epoch) REFERENCES voice_activations(voice_session_id,epoch) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS voice_delete_intents (agent_session_id TEXT PRIMARY KEY, requested_ms INTEGER NOT NULL);
PRAGMA user_version=1;";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceActivationFact {
    pub voice_session_id: String, pub epoch: u64, pub owner_id: String,
    pub agent_session_id: String, pub binding_version: u64, pub route_digest: String, pub started_ms: i64,
    #[serde(default)]pub context_floor:Option<u64>,
    #[serde(default)]pub lease_revision:Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceWorkLink {
    pub operation_key: String, pub canonical_receipt_id: Option<String>, pub quarantined: bool,
    pub owner_id:String,pub agent_session_id:String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceBackupBoundary {
    pub schema_version: i64, pub max_sequence: i64, pub activation_count: i64, pub work_link_count: i64,
}
#[derive(Clone)]
pub struct VoiceJournal { inner: Arc<Mutex<Connection>>, path: PathBuf }
fn failure(error: impl std::fmt::Display) -> VoiceError {
    VoiceError::new(VoiceErrorKind::JournalUnavailable, format!("voice journal unavailable: {error}"))
}
fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64 }
impl VoiceJournal {
    /// Lazy voice-only initialization; failure does not mutate the canonical database.
    pub fn open(data_dir: &Path) -> Result<Self, VoiceError> {
        let dir = data_dir.join("voice"); std::fs::create_dir_all(&dir).map_err(failure)?;
        let path = dir.join("voice.sqlite3");
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) { return Err(failure("journal must not be a symbolic link")); }
        let connection = Connection::open(&path).map_err(failure)?;
        connection.busy_timeout(std::time::Duration::from_secs(2)).map_err(failure)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;").map_err(failure)?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(failure)?;
        if version > SCHEMA_VERSION { return Err(failure("unsupported future schema; data preserved")); }
        if version == 0 { connection.execute_batch(SCHEMA).map_err(failure)?; }
        if version<=SCHEMA_VERSION {pending::ensure_schema(&connection)?;replay::ensure_schema(&connection)?;data::ensure_schema(&connection)?;connection.execute_batch(profiles::SCHEMA).map_err(failure)?;connection.execute_batch(source_intents::SCHEMA).map_err(failure)?;connection.pragma_update(None,"user_version",SCHEMA_VERSION).map_err(failure)?;}
        // Process recovery never revives an input lease, socket or cached audio.
        connection.execute("UPDATE voice_activations SET ended_ms=?1,end_reason='process_recovered' WHERE ended_ms IS NULL", [now_ms()]).map_err(failure)?;
        Ok(Self { inner: Arc::new(Mutex::new(connection)), path })
    }
    pub fn path(&self) -> &Path { &self.path }
    async fn run<T: Send + 'static>(&self, operation: impl FnOnce(&mut Connection) -> Result<T, VoiceError> + Send + 'static) -> Result<T, VoiceError> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || { let mut connection = inner.lock().map_err(|_| failure("journal lock poisoned"))?; operation(&mut connection) })
            .await.map_err(failure)?
    }
    pub async fn activate(&self, fact: VoiceActivationFact) -> Result<(), VoiceError> {
        self.run(move |conn| {
            if fact.epoch == 0 || fact.binding_version == 0 || fact.route_digest.len() != 64 { return Err(failure("invalid activation identity")); }
            conn.execute("INSERT INTO voice_activations(voice_session_id,epoch,owner_id,agent_session_id,binding_version,route_digest,started_ms,context_floor,lease_revision) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![fact.voice_session_id, fact.epoch, fact.owner_id, fact.agent_session_id, fact.binding_version, fact.route_digest, fact.started_ms,fact.context_floor,fact.lease_revision]).map_err(failure)?;
            Ok(())
        }).await
    }
    pub async fn append(&self, voice_session_id: String, epoch: u64, event_key: String, kind: String,
        correlation_id: Option<String>, payload: serde_json::Value) -> Result<i64, VoiceError> {
        self.append_owned(voice_session_id,epoch,event_key,kind,correlation_id,payload,false).await
    }
    /// Measured endpoint receipts may arrive after a closed lease. They preserve
    /// consumed media facts but cannot reopen capture or admit canonical work.
    pub async fn append_playback(&self,voice_session_id:String,epoch:u64,event_key:String,receipt:PlaybackReceipt)->Result<i64,VoiceError>{
        let terminal=matches!(receipt.state,DeliveryState::Played|DeliveryState::Interrupted|DeliveryState::Revoked);
        self.append_owned(voice_session_id,epoch,event_key,"playback".into(),Some(receipt.segment_id.clone()),serde_json::to_value(receipt).map_err(failure)?,terminal).await
    }
    async fn append_owned(&self,voice_session_id:String,epoch:u64,event_key:String,kind:String,correlation_id:Option<String>,payload:serde_json::Value,allow_closed:bool)->Result<i64,VoiceError>{
        let payload = serde_json::to_string(&payload).map_err(failure)?;
        if payload.len() > MAX_FACT_BYTES || event_key.len() > 2048 { return Err(failure("voice fact exceeds bounded journal payload")); }
        self.run(move |conn| {
            let tx = conn.transaction().map_err(failure)?;
            let active: bool = tx.query_row("SELECT ended_ms IS NULL OR ?3 FROM voice_activations WHERE voice_session_id=?1 AND epoch=?2", params![voice_session_id,epoch,allow_closed], |r| r.get(0)).optional().map_err(failure)?.unwrap_or(false);
            if !active { return Err(failure("activation closed; new facts rejected")); }
            tx.execute("INSERT INTO voice_facts(voice_session_id,epoch,event_key,kind,correlation_id,created_ms,payload) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(voice_session_id,epoch,event_key) DO NOTHING",
                params![voice_session_id,epoch,event_key,kind,correlation_id,now_ms(),payload]).map_err(failure)?;
            let (sequence, existing): (i64,String) = tx.query_row("SELECT sequence,payload FROM voice_facts WHERE voice_session_id=?1 AND epoch=?2 AND event_key=?3", params![voice_session_id,epoch,event_key], |r| Ok((r.get(0)?,r.get(1)?))).map_err(failure)?;
            if existing != payload { return Err(failure("voice fact key reused with different payload")); }
            tx.commit().map_err(failure)?; Ok(sequence)
        }).await
    }
    /// Reserve before canonical admission. Missing link after a crash is resolved by querying operation_key, never blindly resubmitting.
    pub async fn reserve_trigger(&self, voice_session_id: String, epoch: u64, trigger_id: String, input_revision: String) -> Result<(VoiceWorkLink, bool), VoiceError> {
        self.run(move |conn| {
            let tx = conn.transaction().map_err(failure)?;
            let active: bool = tx.query_row("SELECT ended_ms IS NULL FROM voice_activations WHERE voice_session_id=?1 AND epoch=?2", params![voice_session_id,epoch], |r| r.get(0)).optional().map_err(failure)?.unwrap_or(false);
            if !active { return Err(failure("voice activation closed")); }
            let key = nomifun_agent_contracts::digest_payload(&(voice_session_id.as_str(),epoch,trigger_id.as_str(),input_revision.as_str())).map_err(failure)?;
            let operation_key = format!("voice:{}", key.as_ref());
            let inserted = tx.execute("INSERT INTO voice_work_links(voice_session_id,epoch,upstream_trigger_id,input_revision,operation_key) VALUES (?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
                params![voice_session_id,epoch,trigger_id,input_revision,operation_key]).map_err(failure)? == 1;
            let link = tx.query_row("SELECT l.operation_key,l.canonical_receipt_id,l.quarantined,a.owner_id,a.agent_session_id FROM voice_work_links l JOIN voice_activations a ON a.voice_session_id=l.voice_session_id AND a.epoch=l.epoch WHERE l.operation_key=?1", [operation_key], |r| Ok(VoiceWorkLink { operation_key:r.get(0)?,canonical_receipt_id:r.get(1)?,quarantined:r.get(2)?,owner_id:r.get(3)?,agent_session_id:r.get(4)? })).map_err(failure)?;
            tx.commit().map_err(failure)?; Ok((link, inserted))
        }).await
    }
    pub async fn associate_receipt(&self, operation_key: String, receipt_id: String) -> Result<(), VoiceError> {
        self.run(move |conn| {
            let changed = conn.execute("UPDATE voice_work_links SET canonical_receipt_id=?1 WHERE operation_key=?2 AND quarantined=0 AND (canonical_receipt_id IS NULL OR canonical_receipt_id=?1)", params![receipt_id,operation_key]).map_err(failure)?;
            if changed != 1 { return Err(failure("canonical receipt association conflict")); } Ok(())
        }).await
    }
    pub async fn unresolved_links(&self) -> Result<Vec<VoiceWorkLink>, VoiceError> {
        self.run(|conn| {
            let mut statement = conn.prepare("SELECT l.operation_key,l.canonical_receipt_id,l.quarantined,a.owner_id,a.agent_session_id FROM voice_work_links l JOIN voice_activations a ON a.voice_session_id=l.voice_session_id AND a.epoch=l.epoch WHERE l.quarantined=0").map_err(failure)?;
            statement.query_map([], |r| Ok(VoiceWorkLink {operation_key:r.get(0)?,canonical_receipt_id:r.get(1)?,quarantined:r.get(2)?,owner_id:r.get(3)?,agent_session_id:r.get(4)?})).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure)
        }).await
    }
    pub async fn quarantine(&self, operation_key: String) -> Result<(), VoiceError> {
        self.run(move |conn| { conn.execute("UPDATE voice_work_links SET quarantined=1 WHERE operation_key=?1", [operation_key]).map_err(failure)?; Ok(()) }).await
    }
    pub async fn close(&self, voice_session_id: String, epoch: u64, termination: VoiceTermination) -> Result<(), VoiceError> {
        self.run(move |conn| {
            conn.execute("UPDATE voice_activations SET ended_ms=COALESCE(ended_ms,?1),end_reason=COALESCE(end_reason,?2) WHERE voice_session_id=?3 AND epoch=?4",
                params![now_ms(),serde_json::to_string(&termination).map_err(failure)?,voice_session_id,epoch]).map_err(failure)?; Ok(())
        }).await
    }
    pub async fn request_delete(&self, agent_session_id: String) -> Result<(), VoiceError> {
        self.run(move |conn| {
            let existing=conn.query_row("SELECT include_profiles FROM voice_delete_intents WHERE agent_session_id=?1",[&agent_session_id],|row|row.get::<_,Option<bool>>(0)).optional().map_err(failure)?;
            if existing.is_some_and(|mode|mode!=Some(true)){return Err(failure("explicit deletion cannot replace an existing passive delete intent"));}
            conn.execute("INSERT OR IGNORE INTO voice_delete_intents(agent_session_id,requested_ms,include_profiles) VALUES (?1,?2,1)", params![agent_session_id,now_ms()]).map_err(failure)?; Ok(()) }).await
    }
    pub async fn finish_delete(&self, agent_session_id: String) -> Result<(), VoiceError> {
        self.run(move |conn| {
            let tx = conn.transaction().map_err(failure)?;
            let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM voice_activations WHERE agent_session_id=?1 AND ended_ms IS NULL)",[&agent_session_id],|row|row.get(0)).map_err(failure)?;
            if active{return Err(failure("active voice leases must be closed before recovery deletes their facts"));}
            let intent=tx.query_row("SELECT owner_id,include_profiles FROM voice_delete_intents WHERE agent_session_id=?1",[&agent_session_id],|row|Ok((row.get::<_,Option<String>>(0)?,row.get::<_,Option<bool>>(1)?))).optional().map_err(failure)?.ok_or_else(||failure("voice deletion requires its durable intent"))?;
            if let Some(owner)=&intent.0 {data::session_owner(&tx,owner,&agent_session_id)?;}
            tx.execute("DELETE FROM voice_activations WHERE agent_session_id=?1", [&agent_session_id]).map_err(failure)?;
            if intent.1.unwrap_or(false){tx.execute("DELETE FROM voice_profiles WHERE agent_session_id=?1", [&agent_session_id]).map_err(failure)?;}
            tx.execute("DELETE FROM voice_delete_intents WHERE agent_session_id=?1", [agent_session_id]).map_err(failure)?;
            tx.commit().map_err(failure)?; Ok(())
        }).await
    }
    pub async fn pending_deletes(&self) -> Result<Vec<String>, VoiceError> {
        self.run(|conn| { let mut s = conn.prepare("SELECT agent_session_id FROM voice_delete_intents").map_err(failure)?;
            s.query_map([], |r| r.get(0)).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure) }).await
    }
    /// Retention removes high-frequency presentation facts; stable admission links remain until Session deletion.
    pub async fn retain_since(&self, cutoff_ms: i64) -> Result<usize, VoiceError> {
        self.run(move |conn| conn.execute("DELETE FROM voice_facts WHERE created_ms<?1 AND kind NOT LIKE 'work_%' AND EXISTS(SELECT 1 FROM voice_activations a WHERE a.voice_session_id=voice_facts.voice_session_id AND a.epoch=voice_facts.epoch AND a.ended_ms IS NOT NULL)", [cutoff_ms]).map_err(failure)).await
    }
    pub async fn backup(&self, destination: PathBuf) -> Result<VoiceBackupBoundary, VoiceError> {
        self.run(move |conn| {
            if destination.exists() { return Err(failure("voice backup destination already exists")); }
            conn.backup(rusqlite::DatabaseName::Main, &destination, None).map_err(failure)?;
            let snapshot = Connection::open_with_flags(&destination, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(failure)?;
            Ok(VoiceBackupBoundary { schema_version: SCHEMA_VERSION,
                max_sequence:snapshot.query_row("SELECT COALESCE(MAX(sequence),0) FROM voice_facts",[],|r|r.get(0)).map_err(failure)?,
                activation_count:snapshot.query_row("SELECT COUNT(*) FROM voice_activations",[],|r|r.get(0)).map_err(failure)?,
                work_link_count:snapshot.query_row("SELECT COUNT(*) FROM voice_work_links",[],|r|r.get(0)).map_err(failure)? })
        }).await
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn fact() -> VoiceActivationFact { VoiceActivationFact {voice_session_id:"v".into(),epoch:1,owner_id:"u".into(),agent_session_id:"a".into(),binding_version:1,route_digest:"a".repeat(64),started_ms:now_ms(),context_floor:Some(0),lease_revision:Some("lease".into())} }
    #[tokio::test] async fn durable_trigger_replay_backup_recovery_and_delete_keep_work_separate() {
        let dir=tempfile::tempdir().unwrap(); let journal=VoiceJournal::open(dir.path()).unwrap();
        journal.activate(fact()).await.unwrap();
        let (first, fresh)=journal.reserve_trigger("v".into(),1,"t".into(),"r1".into()).await.unwrap(); assert!(fresh);
        journal.associate_receipt(first.operation_key.clone(),"canonical1".into()).await.unwrap();
        let (replay,fresh)=journal.reserve_trigger("v".into(),1,"t".into(),"r1".into()).await.unwrap(); assert!(!fresh); assert_eq!(replay.canonical_receipt_id.as_deref(),Some("canonical1"));
        journal.append("v".into(),1,"played1".into(),"playback".into(),None,serde_json::json!({"consumed_us":12000})).await.unwrap();
        let boundary=journal.backup(dir.path().join("snapshot.sqlite3")).await.unwrap(); assert_eq!(boundary.max_sequence,1); assert_eq!(boundary.work_link_count,1);
        drop(journal); let journal=VoiceJournal::open(dir.path()).unwrap();
        assert!(journal.reserve_trigger("v".into(),1,"new".into(),"r1".into()).await.is_err());
        assert_eq!(journal.unresolved_links().await.unwrap().len(),1);
        journal.request_delete("a".into()).await.unwrap(); assert_eq!(journal.pending_deletes().await.unwrap(),vec!["a"]);
        journal.finish_delete("a".into()).await.unwrap(); assert!(journal.unresolved_links().await.unwrap().is_empty());
        assert!(!dir.path().join("nomifun-backend.db").exists());
    }
}
