use std::path::{Path,PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::VoiceJournal;
use nomifun_voice_contracts::voice::{VoiceError,VoiceErrorKind};

/// One lazy voice database owner, shared by the service and its canonical work
/// adapter. Construction never accesses disk or the main Agent database.
pub struct SharedVoiceJournal {
    data_dir:PathBuf,
    journal:Mutex<Option<Arc<VoiceJournal>>>,
}
impl SharedVoiceJournal {
    pub fn new(data_dir:PathBuf)->Self{Self{data_dir,journal:Mutex::new(None)}}
    pub fn data_dir(&self)->&Path{&self.data_dir}
    pub async fn opened(&self)->Option<Arc<VoiceJournal>>{self.journal.lock().await.clone()}
    pub async fn get_or_open(&self)->Result<Arc<VoiceJournal>,VoiceError>{
        let mut current=self.journal.lock().await;
        if let Some(journal)=&*current{return Ok(journal.clone());}
        let data_dir=self.data_dir.clone();
        let journal=tokio::task::spawn_blocking(move||VoiceJournal::open(&data_dir)).await
            .map_err(|_|VoiceError::new(VoiceErrorKind::JournalUnavailable,"voice store initialization failed"))??;
        let journal=Arc::new(journal);*current=Some(journal.clone());Ok(journal)
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[tokio::test]async fn construction_is_lazy_and_concurrent_consumers_share_one_recovery_owner(){
        let dir=tempfile::tempdir().unwrap();let store=SharedVoiceJournal::new(dir.path().into());
        assert!(store.opened().await.is_none());assert!(!dir.path().join("voice").exists());
        let(a,b)=tokio::join!(store.get_or_open(),store.get_or_open());let(a,b)=(a.unwrap(),b.unwrap());
        assert!(Arc::ptr_eq(&a,&b));assert!(!dir.path().join("nomifun-backend.db").exists());
    }
}
