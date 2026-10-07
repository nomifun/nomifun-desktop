//! Explicit voice data extension. No canonical mutation or main backup hook.
use super::*;
use std::io::Read;

const MAX_EXPORT_BYTES:u64=32*1024*1024;
#[derive(Debug)]
pub struct VoiceSnapshotExport {pub bytes:Vec<u8>,pub boundary:VoiceBackupBoundary}
pub(super) fn ensure_schema(conn:&Connection)->Result<(),VoiceError> {
    let columns=conn.prepare("PRAGMA table_info(voice_delete_intents)").map_err(failure)?.query_map([],|row|row.get::<_,String>(1)).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure)?;
    if !columns.iter().any(|column|column=="owner_id"){conn.execute_batch("ALTER TABLE voice_delete_intents ADD COLUMN owner_id TEXT;").map_err(failure)?;}
    if !columns.iter().any(|column|column=="include_profiles"){conn.execute_batch("ALTER TABLE voice_delete_intents ADD COLUMN include_profiles INTEGER;").map_err(failure)?;}Ok(())
}

fn export_owner(conn:&Connection,owner:&str)->Result<(),VoiceError> {
    let foreign:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM voice_activations WHERE owner_id<>?1) OR EXISTS(SELECT 1 FROM voice_profiles WHERE owner_id<>?1) OR EXISTS(SELECT 1 FROM voice_pending_intents WHERE owner_id<>?1)",[owner],|row|row.get(0)).map_err(failure)?;
    if foreign{return Err(VoiceError::new(VoiceErrorKind::Authentication,"A mixed-owner voice database cannot be exported as one owner's snapshot."));}Ok(())
}
pub(super) fn session_owner(conn:&Connection,owner:&str,session:&str)->Result<(),VoiceError> {
    let foreign:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM voice_activations WHERE agent_session_id=?2 AND owner_id<>?1) OR EXISTS(SELECT 1 FROM voice_profiles WHERE agent_session_id=?2 AND owner_id<>?1) OR EXISTS(SELECT 1 FROM voice_pending_intents WHERE agent_session_id=?2 AND owner_id<>?1)",params![owner,session],|row|row.get(0)).map_err(failure)?;
    if foreign{return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice Session data belongs to another owner"));}Ok(())
}
impl VoiceJournal {
    pub async fn referenced_sessions(&self,owner:String)->Result<Vec<String>,VoiceError> {
        self.run(move|conn|{let mut statement=conn.prepare("SELECT agent_session_id FROM voice_activations WHERE owner_id=?1 UNION SELECT agent_session_id FROM voice_profiles WHERE owner_id=?1 UNION SELECT agent_session_id FROM voice_pending_intents WHERE owner_id=?1 ORDER BY agent_session_id").map_err(failure)?;
            let sessions=statement.query_map([owner],|row|row.get::<_,String>(0)).map_err(failure)?.collect::<Result<Vec<_>,_>>().map_err(failure)?;Ok(sessions)}).await
    }
    pub async fn export_snapshot(&self,owner:String)->Result<VoiceSnapshotExport,VoiceError> {
        self.run(move|conn|{
            export_owner(conn,&owner)?;let directory=tempfile::tempdir().map_err(failure)?;let path=directory.path().join("voice.snapshot.sqlite3");
            conn.backup(rusqlite::DatabaseName::Main,&path,None).map_err(failure)?;
            let snapshot=Connection::open_with_flags(&path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(failure)?;
            // Validate the actual SQLite snapshot rather than only an earlier
            // ownership read. Concurrent writers cannot add another owner.
            export_owner(&snapshot,&owner)?;
            let length=std::fs::metadata(&path).map_err(failure)?.len();if length>MAX_EXPORT_BYTES{return Err(VoiceError::new(VoiceErrorKind::Backlog,"voice snapshot exceeds the bounded export size"));}
            let schema_version=snapshot.query_row("PRAGMA user_version",[],|row|row.get(0)).map_err(failure)?;
            let boundary=VoiceBackupBoundary {schema_version,max_sequence:snapshot.query_row("SELECT COALESCE(MAX(sequence),0) FROM voice_facts",[],|row|row.get(0)).map_err(failure)?,
                activation_count:snapshot.query_row("SELECT COUNT(*) FROM voice_activations",[],|row|row.get(0)).map_err(failure)?,work_link_count:snapshot.query_row("SELECT COUNT(*) FROM voice_work_links",[],|row|row.get(0)).map_err(failure)?};
            drop(snapshot);let mut bytes=Vec::with_capacity(length as usize);std::fs::File::open(&path).map_err(failure)?.take(MAX_EXPORT_BYTES+1).read_to_end(&mut bytes).map_err(failure)?;
            if bytes.len() as u64>MAX_EXPORT_BYTES{return Err(VoiceError::new(VoiceErrorKind::Backlog,"voice snapshot grew beyond its bound"));}Ok(VoiceSnapshotExport {bytes,boundary})
        }).await
    }
    pub async fn begin_owned_session_delete(&self,owner:String,session:String,include_profiles:bool)->Result<(),VoiceError> {
        self.run(move|conn|{let tx=conn.transaction().map_err(failure)?;session_owner(&tx,&owner,&session)?;
            let existing=tx.query_row("SELECT owner_id,include_profiles FROM voice_delete_intents WHERE agent_session_id=?1",[&session],|row|Ok((row.get::<_,Option<String>>(0)?,row.get::<_,Option<bool>>(1)?))).optional().map_err(failure)?;
            if let Some((old_owner,old_mode))=existing {if old_owner.as_ref().is_some_and(|old|old!=&owner)||old_mode.unwrap_or(false)!=include_profiles{return Err(failure("voice delete intent mode or owner differs from its immutable request"));}}
            else {tx.execute("INSERT INTO voice_delete_intents(agent_session_id,requested_ms,owner_id,include_profiles) VALUES (?1,?2,?3,?4)",params![session,now_ms(),owner,include_profiles]).map_err(failure)?;}
            tx.commit().map_err(failure)?;Ok(())}).await
    }
    /// Passive tombstone GC passes false and keeps Non-Agent profiles. The
    /// explicit authenticated voice-data route may request their removal.
    pub async fn finish_owned_session_delete(&self,owner:String,session:String,include_profiles:bool)->Result<(),VoiceError> {
        self.run(move|conn|{let tx=conn.transaction().map_err(failure)?;session_owner(&tx,&owner,&session)?;
            let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM voice_activations WHERE owner_id=?1 AND agent_session_id=?2 AND ended_ms IS NULL)",params![owner,session],|row|row.get(0)).map_err(failure)?;
            if active{return Err(VoiceError::new(VoiceErrorKind::Closed,"voice cleanup must revoke and settle its active leases before removing facts"));}
            let intent=tx.query_row("SELECT owner_id,include_profiles FROM voice_delete_intents WHERE agent_session_id=?1",[&session],|row|Ok((row.get::<_,Option<String>>(0)?,row.get::<_,Option<bool>>(1)?))).optional().map_err(failure)?.ok_or_else(||failure("voice deletion needs its durable intent"))?;
            if intent.0.as_ref().is_some_and(|saved|saved!=&owner)||intent.1.unwrap_or(false)!=include_profiles{return Err(failure("voice deletion cannot change its durable owner or profile mode"));}
            tx.execute("DELETE FROM voice_activations WHERE owner_id=?1 AND agent_session_id=?2",params![owner,session]).map_err(failure)?;
            if include_profiles {tx.execute("DELETE FROM voice_profiles WHERE owner_id=?1 AND agent_session_id=?2",params![owner,session]).map_err(failure)?;}
            tx.execute("DELETE FROM voice_delete_intents WHERE agent_session_id=?1",[session]).map_err(failure)?;tx.commit().map_err(failure)?;Ok(())}).await
    }
}
