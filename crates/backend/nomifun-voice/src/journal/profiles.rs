use super::*;
use nomifun_voice_contracts::VoiceProfile;
pub(super) const SCHEMA:&str="CREATE TABLE IF NOT EXISTS voice_profiles (
 profile_id TEXT PRIMARY KEY,owner_id TEXT NOT NULL,agent_session_id TEXT NOT NULL,
 binding_version INTEGER NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,updated_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS voice_profiles_session ON voice_profiles(owner_id,agent_session_id,updated_ms);";
impl VoiceJournal {
    pub async fn profile(&self,owner:String,id:String)->Result<Option<VoiceProfile>,VoiceError>{
        self.run(move|conn|{
            let payload:Option<String>=conn.query_row("SELECT payload FROM voice_profiles WHERE profile_id=?1 AND owner_id=?2",params![id,owner],|row|row.get(0)).optional().map_err(failure)?;
            payload.map(|json|serde_json::from_str(&json).map_err(failure)).transpose()
        }).await
    }
    pub async fn session_profile(&self,owner:String,session:String)->Result<Option<VoiceProfile>,VoiceError>{
        self.run(move|conn|{
            let payload:Option<String>=conn.query_row("SELECT payload FROM voice_profiles WHERE owner_id=?1 AND agent_session_id=?2 ORDER BY updated_ms DESC,profile_id DESC LIMIT 1",params![owner,session],|row|row.get(0)).optional().map_err(failure)?;
            payload.map(|json|serde_json::from_str(&json).map_err(failure)).transpose()
        }).await
    }
    pub async fn save_profile(&self,owner:String,profile:VoiceProfile,expected:u64)->Result<VoiceProfile,VoiceError>{
        if profile.profile_id.trim().is_empty()||profile.profile_id.len()>256||profile.binding_version==0||expected.checked_add(1)!=Some(profile.revision)||profile.revision>i64::MAX as u64{return Err(VoiceError::new(VoiceErrorKind::Configuration,"invalid voice profile revision"));}
        profile.route.validate().map_err(|message|VoiceError::new(VoiceErrorKind::Configuration,message))?;
        let payload=serde_json::to_string(&profile).map_err(failure)?;
        if payload.len()>64*1024{return Err(VoiceError::new(VoiceErrorKind::Configuration,"voice profile exceeds configuration budget"));}
        self.run(move|conn|{
            let tx=conn.transaction().map_err(failure)?;
            let deleting:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM voice_delete_intents WHERE agent_session_id=?1)",[profile.agent_session_id.as_ref()],|row|row.get(0)).map_err(failure)?;
            if deleting{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice data deletion is pending; settle its original request before saving a new profile"));}
            let existing:Option<(String,u64)>=tx.query_row("SELECT owner_id,revision FROM voice_profiles WHERE profile_id=?1",[&profile.profile_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(failure)?;
            if existing.as_ref().is_some_and(|(saved_owner,revision)|saved_owner!=&owner||*revision!=expected)||existing.is_none()&&expected!=0{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice profile changed or is not owned by this caller"));}
            tx.execute("INSERT INTO voice_profiles(profile_id,owner_id,agent_session_id,binding_version,revision,payload,updated_ms) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(profile_id) DO UPDATE SET agent_session_id=excluded.agent_session_id,binding_version=excluded.binding_version,revision=excluded.revision,payload=excluded.payload,updated_ms=excluded.updated_ms",params![profile.profile_id,owner,profile.agent_session_id.as_ref(),profile.binding_version,profile.revision,payload,now_ms()]).map_err(failure)?;
            tx.commit().map_err(failure)?;Ok(profile)
        }).await
    }
}
