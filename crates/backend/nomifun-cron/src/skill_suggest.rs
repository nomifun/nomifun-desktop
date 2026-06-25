use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomifun_common::now_ms;
use nomifun_db::IConversationRepository;
use nomifun_realtime::EventBroadcaster;
use tokio::fs;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::artifacts::{broadcast_artifact, build_skill_suggest_artifact};
use crate::error::CronError;
use crate::prompt::SKILL_SUGGEST_FILENAME;
use crate::skill_file::{content_hash, has_skill_file, validate_skill_content};

const RETRY_DELAYS_MS: [u64; 3] = [1000, 2000, 3000];

#[derive(Clone)]
pub struct SkillSuggestDetector {
    broadcaster: Arc<dyn EventBroadcaster>,
    conversation_repo: Arc<dyn IConversationRepository>,
    data_dir: PathBuf,
    last_hash_by_job: Arc<Mutex<HashMap<String, String>>>,
}

impl SkillSuggestDetector {
    pub fn new(
        broadcaster: Arc<dyn EventBroadcaster>,
        conversation_repo: Arc<dyn IConversationRepository>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            broadcaster,
            conversation_repo,
            data_dir,
            last_hash_by_job: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn schedule_check(&self, conversation_id: String, job_id: String, workspace: String) {
        let detector = self.clone();
        tokio::spawn(async move {
            detector.check_with_retry(&conversation_id, &job_id, &workspace).await;
        });
    }

    async fn check_with_retry(&self, conversation_id: &str, job_id: &str, workspace: &str) {
        for delay_ms in RETRY_DELAYS_MS {
            sleep(Duration::from_millis(delay_ms)).await;
            match self.check_and_emit(conversation_id, job_id, workspace).await {
                Ok(true) => return,
                Ok(false) => continue,
                Err(err) => {
                    warn!(
                        conversation_id,
                        job_id,
                        error = %err,
                        "Failed checking SKILL_SUGGEST.md"
                    );
                }
            }
        }
    }

    async fn check_and_emit(&self, conversation_id: &str, job_id: &str, workspace: &str) -> Result<bool, CronError> {
        if workspace.trim().is_empty() {
            return Ok(false);
        }

        if has_skill_file(&self.data_dir, job_id).await? {
            self.clear_last_hash(job_id);
            return Ok(true);
        }

        let file_path = Path::new(workspace).join(SKILL_SUGGEST_FILENAME);
        let content = match fs::read_to_string(&file_path).await {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => {
                return Err(CronError::InvalidSkillContent(err.to_string()));
            }
        };

        if content.trim().is_empty() {
            return Ok(false);
        }

        let validated = match validate_skill_content(&content) {
            Ok(validated) => validated,
            Err(_) => return Ok(false),
        };

        let hash = content_hash(&content);
        if self.last_hash(job_id).as_deref() == Some(hash.as_str()) {
            return Ok(true);
        }

        self.set_last_hash(job_id, hash);
        self.emit(
            conversation_id,
            job_id,
            &validated.name,
            &validated.description,
            &content,
        )
        .await;
        Ok(true)
    }

    async fn emit(&self, conversation_id: &str, job_id: &str, name: &str, description: &str, skill_content: &str) {
        self.persist_and_broadcast(conversation_id, job_id, name, description, skill_content)
            .await;
    }

    async fn persist_and_broadcast(
        &self,
        conversation_id: &str,
        job_id: &str,
        name: &str,
        description: &str,
        skill_content: &str,
    ) {
        let row = build_skill_suggest_artifact(conversation_id, job_id, name, description, skill_content, now_ms());

        let row = match self.conversation_repo.upsert_artifact(&row).await {
            Ok(row) => row,
            Err(err) => {
                warn!(
                    conversation_id,
                    job_id,
                    error = %err,
                    "Failed persisting cron skill suggestion artifact"
                );
                return;
            }
        };

        if let Err(err) = broadcast_artifact(&self.broadcaster, &row) {
            warn!(
                conversation_id,
                job_id,
                error = %err,
                "Failed broadcasting cron skill suggestion artifact"
            );
            return;
        }
        debug!(conversation_id, job_id, "Broadcasted cron skill suggestion artifact");
    }

    fn last_hash(&self, job_id: &str) -> Option<String> {
        self.last_hash_by_job
            .lock()
            .ok()
            .and_then(|hashes| hashes.get(job_id).cloned())
    }

    fn set_last_hash(&self, job_id: &str, hash: String) {
        if let Ok(mut hashes) = self.last_hash_by_job.lock() {
            hashes.insert(job_id.to_owned(), hash);
        }
    }

    fn clear_last_hash(&self, job_id: &str) {
        if let Ok(mut hashes) = self.last_hash_by_job.lock() {
            hashes.remove(job_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::models::{ConversationRow, CronJobRow};
    use nomifun_db::{
        ICronRepository, SqliteConversationRepository, SqliteCronRepository, SqlitePool, init_database_memory,
    };
    use nomifun_realtime::BroadcastEventBus;
    use tempfile::tempdir;

    fn make_conversation(id: &str) -> ConversationRow {
        ConversationRow {
            // `create` allocates the PK (AUTOINCREMENT) and ignores this field,
            // but parse the test's id string so the struct is well-typed.
            id: id.parse::<i64>().unwrap_or_default(),
            user_id: "system_default_user".into(),
            name: "Cron Conversation".into(),
            r#type: "acp".into(),
            extra: "{}".into(),
            model: None,
            status: Some("finished".into()),
            source: Some("nomifun".into()),
            channel_chat_id: None,
            pinned: false,
            pinned_at: None,
            cron_job_id: None,
            created_at: now_ms(),
            updated_at: now_ms(),
        }
    }

    /// Seed a minimal `cron_jobs` row so the
    /// `conversation_artifacts.cron_job_id → cron_jobs(id)` FK is satisfied
    /// when a skill-suggest artifact is persisted (foreign_keys=ON).
    async fn seed_cron_job(pool: &SqlitePool, id: &str) {
        let repo = SqliteCronRepository::new(pool.clone());
        repo.insert(&CronJobRow {
            id: id.into(),
            name: "Test Cron".into(),
            enabled: true,
            schedule_kind: "every".into(),
            schedule_value: "60000".into(),
            schedule_tz: None,
            schedule_description: None,
            payload_message: "ping".into(),
            execution_mode: "new_conversation".into(),
            agent_config: None,
            conversation_id: None,
            conversation_title: None,
            agent_type: "acp".into(),
            created_by: "user".into(),
            skill_content: None,
            description: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            next_run_at: None,
            last_run_at: None,
            last_status: None,
            last_error: None,
            run_count: 0,
            retry_count: 0,
            max_retries: 3,
            target_kind: "agent".into(),
            terminal_mode: None,
            terminal_session_id: None,
            terminal_command: None,
            terminal_args: None,
            terminal_script: None,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn emits_skill_suggest_when_file_is_valid() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::write(
            workspace.join(SKILL_SUGGEST_FILENAME),
            "---\nname: daily-report\ndescription: Daily report\n---\n\nCheck sources.\n",
        )
        .await
        .unwrap();

        let db = init_database_memory().await.unwrap();
        let repo: Arc<dyn IConversationRepository> = Arc::new(SqliteConversationRepository::new(db.pool().clone()));
        seed_cron_job(db.pool(), "cron-1").await;
        repo.create(&make_conversation("1")).await.unwrap();

        let bus = Arc::new(BroadcastEventBus::new(16));
        let detector = SkillSuggestDetector::new(bus.clone(), repo.clone(), temp.path().to_path_buf());
        let mut rx = bus.subscribe();

        let emitted = detector
            .check_and_emit("1", "cron-1", &workspace.to_string_lossy())
            .await
            .unwrap();

        assert!(emitted);
        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.name, "conversation.artifact");
        assert_eq!(msg.data["kind"], "skill_suggest");
        assert_eq!(msg.data["status"], "pending");
        assert_eq!(msg.data["payload"]["cron_job_id"], "cron-1");
        assert_eq!(msg.data["payload"]["name"], "daily-report");

        let rows = repo.list_artifacts(1).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "skill_suggest");
        assert_eq!(rows[0].status, "pending");
        assert!(rows[0].payload.contains("\"skillContent\""));
    }

    #[tokio::test]
    async fn suppresses_duplicate_skill_suggest_content() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::write(
            workspace.join(SKILL_SUGGEST_FILENAME),
            "---\nname: daily-report\ndescription: Daily report\n---\n\nCheck sources.\n",
        )
        .await
        .unwrap();

        let db = init_database_memory().await.unwrap();
        let repo: Arc<dyn IConversationRepository> = Arc::new(SqliteConversationRepository::new(db.pool().clone()));
        seed_cron_job(db.pool(), "cron-1").await;
        repo.create(&make_conversation("1")).await.unwrap();
        repo.create(&make_conversation("conv-2")).await.unwrap();

        let bus = Arc::new(BroadcastEventBus::new(16));
        let detector = SkillSuggestDetector::new(bus.clone(), repo, temp.path().to_path_buf());
        let mut rx = bus.subscribe();

        assert!(
            detector
                .check_and_emit("1", "cron-1", &workspace.to_string_lossy())
                .await
                .unwrap()
        );
        assert!(rx.try_recv().is_ok());

        assert!(
            detector
                .check_and_emit("conv-2", "cron-1", &workspace.to_string_lossy())
                .await
                .unwrap()
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn suppresses_skill_suggest_when_saved_skill_exists() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::write(
            workspace.join(SKILL_SUGGEST_FILENAME),
            "---\nname: daily-report\ndescription: Daily report\n---\n\nCheck sources.\n",
        )
        .await
        .unwrap();
        let skill_dir = temp.path().join("cron/skills/cron-cron-1");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: saved-skill\ndescription: Saved skill\n---\n\nDo the task.\n",
        )
        .await
        .unwrap();

        let db = init_database_memory().await.unwrap();
        let repo: Arc<dyn IConversationRepository> = Arc::new(SqliteConversationRepository::new(db.pool().clone()));
        repo.create(&make_conversation("1")).await.unwrap();

        let bus = Arc::new(BroadcastEventBus::new(16));
        let detector = SkillSuggestDetector::new(bus.clone(), repo, temp.path().to_path_buf());
        let mut rx = bus.subscribe();

        let emitted = detector
            .check_and_emit("1", "cron-1", &workspace.to_string_lossy())
            .await
            .unwrap();

        assert!(emitted);
        assert!(rx.try_recv().is_err());
    }
}
