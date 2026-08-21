use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use nomi_types::message::{Message, TokenUsage};

/// Exact engine transcript boundary for the latest editable root user turn.
///
/// `source_message_id` is the durable database identity of that user message;
/// `start_len` is the engine message count immediately before the turn began.
/// Keeping both values prevents an automatic continuation from moving the
/// rewind boundary into the middle of the same logical user turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableTurnCheckpoint {
    pub source_message_id: String,
    pub start_len: usize,
    /// Host routing state immediately before this root turn. Rewind restores
    /// this exact snapshot instead of guessing from free-form transcript text.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prior_host_context: BTreeMap<String, String>,
}

/// Durable transaction root for one accepted Agent turn.
///
/// The engine persists this snapshot before the first provider await and does
/// not clear it until the host has committed every post-model effect (artifact
/// delivery, memory projection, and the terminal receipt).  A process restart
/// that observes the marker restores this exact root before the session can be
/// resumed, so an interrupted or rejected assistant claim is never replayed as
/// committed history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedTurnRoot {
    pub source_message_id: String,
    pub messages: Vec<Message>,
    pub editable_turn: Option<EditableTurnCheckpoint>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub host_context: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activated_deferred_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provider: String,
    pub model: String,
    pub cwd: String,
    pub total_usage: TokenUsage,
    pub messages: Vec<Message>,
    /// Stable identity of the conversation INSTANCE that owns this session
    /// (e.g. the conversation row's `created_at`). The session directory is
    /// keyed by the reusable integer `conversation_id`, so after a delete +
    /// id reuse (or a DB rebaseline) a new conversation can land on an old
    /// session file. Resume paths compare this token and start fresh on a
    /// mismatch instead of inheriting a stranger's history. `None` = legacy
    /// session written before this field existed (accepted, then migrated).
    #[serde(default)]
    pub owner_token: Option<String>,
    /// Deferred tools activated by ToolSearch for this session. Stored as
    /// canonical registry names so a resumed engine keeps sending their full
    /// schemas. Values are stable activation identities rather than mutable
    /// provider display aliases. A restored identity may remain pending until
    /// its dynamic MCP tool is registered before the first message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activated_deferred_tools: Vec<String>,
    /// Persisted authority for "edit latest message and resubmit".
    ///
    /// Legacy sessions do not contain this field and deserialize as `None`.
    /// They must not guess a rewind boundary from message roles because tool
    /// results, steering, and continuations are also represented as user
    /// messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editable_turn: Option<EditableTurnCheckpoint>,
    /// Small host-owned state that must survive runtime refresh/resume. Keys
    /// and values are opaque to the engine; the host owns their validation and
    /// lifecycle. This avoids reconstructing security-sensitive routing state
    /// from free-form transcript text after a restart.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub host_context: BTreeMap<String, String>,
    /// Exact pre-turn state while an accepted turn is not yet durably
    /// committed by its owner. Legacy sessions deserialize as committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_turn_root: Option<AcceptedTurnRoot>,
    /// Recovery root retained after the engine transcript is checked but
    /// before/after the host publishes its durable terminal. It is not an
    /// interrupted marker for ordinary resume; Conversation boot uses it only
    /// when its authoritative receipt still says the exact turn was Running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_host_terminal_root: Option<AcceptedTurnRoot>,
    /// Idempotence proof for crash recovery. Conversation boot may retry its
    /// durable receipt transition after the transcript root was already
    /// repaired; this exact source id proves the retry is not accepting an
    /// unrelated marker-free session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_interrupted_turn_source: Option<String>,
}

impl Session {
    /// Recover a crash-interrupted accepted turn in memory.
    ///
    /// Usage is intentionally retained: provider cost remains true even when
    /// the provisional transcript is rejected. All resumable conversational
    /// state is restored from the marker and the marker is consumed; callers
    /// must atomically persist the returned state before allowing resume.
    pub fn recover_interrupted_accepted_turn(&mut self) -> bool {
        let Some(root) = self.accepted_turn_root.take() else {
            return false;
        };
        self.last_interrupted_turn_source = Some(root.source_message_id);
        self.pending_host_terminal_root = None;
        self.messages = root.messages;
        self.editable_turn = root.editable_turn;
        self.host_context = root.host_context;
        self.activated_deferred_tools = root.activated_deferred_tools;
        self.updated_at = Utc::now();
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIndex {
    pub sessions: Vec<SessionMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub model: String,
    /// First user message, truncated to 80 chars
    pub summary: String,
    pub message_count: usize,
}

pub struct SessionManager {
    directory: PathBuf,
    max_sessions: usize,
    #[cfg(test)]
    fail_next_save: AtomicUsize,
}

impl SessionManager {
    pub fn new(directory: PathBuf, max_sessions: usize) -> Self {
        Self {
            directory,
            max_sessions,
            #[cfg(test)]
            fail_next_save: AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_next_save_for_test(&self) {
        self.fail_next_save.fetch_add(1, Ordering::SeqCst);
    }

    /// Create a new session, return it
    pub fn create(
        &self,
        provider: &str,
        model: &str,
        cwd: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<Session> {
        std::fs::create_dir_all(&self.directory)?;

        let id = if let Some(custom_id) = session_id {
            // Validate that the ID doesn't already exist
            let index = self.load_index()?;
            if index.sessions.iter().any(|s| s.id == custom_id) {
                anyhow::bail!("Session ID '{}' already exists", custom_id);
            }
            custom_id.to_string()
        } else {
            generate_short_id()
        };
        let session = Session {
            id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            provider: provider.to_string(),
            model: model.to_string(),
            cwd: cwd.to_string(),
            total_usage: TokenUsage::default(),
            messages: Vec::new(),
            owner_token: None,
            activated_deferred_tools: Vec::new(),
            editable_turn: None,
            host_context: BTreeMap::new(),
            accepted_turn_root: None,
            pending_host_terminal_root: None,
            last_interrupted_turn_source: None,
        };
        self.save(&session)?;
        self.update_index(&session)?;
        self.cleanup_old()?;
        Ok(session)
    }

    /// Save current session state (called after each turn)
    pub fn save(&self, session: &Session) -> anyhow::Result<()> {
        #[cfg(test)]
        if self
            .fail_next_save
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            anyhow::bail!("injected session save failure");
        }
        std::fs::create_dir_all(&self.directory)?;
        let filename = format!(
            "{}_{}.json",
            session.created_at.format("%Y-%m-%d"),
            session.id
        );
        let path = self.directory.join(&filename);
        write_json_atomic(&path, session)
    }

    /// Load a session by ID (or "latest")
    pub fn load(&self, id_or_latest: &str) -> anyhow::Result<Session> {
        let index = self.load_index()?;

        let meta = if id_or_latest == "latest" {
            index
                .sessions
                .last()
                .ok_or_else(|| anyhow::anyhow!("No sessions found"))?
        } else {
            index
                .sessions
                .iter()
                .find(|s| s.id == id_or_latest)
                .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", id_or_latest))?
        };

        let pattern = format!("*_{}.json", meta.id);
        let session_files: Vec<_> =
            glob::glob(self.directory.join(&pattern).to_string_lossy().as_ref())?
                .filter_map(|r| r.ok())
                .collect();

        let path = session_files
            .first()
            .ok_or_else(|| anyhow::anyhow!("Session file not found for '{}'", meta.id))?;

        let content = std::fs::read_to_string(path)?;
        let mut session: Session = serde_json::from_str(&content)?;
        if session.recover_interrupted_accepted_turn() {
            // A resumable session is a committed-history boundary. Refuse to
            // return the recovered root until both transcript and index have
            // been durably repaired; a retry will see either the still-marked
            // old document or the fully recovered root, never a provisional
            // suffix accepted as success.
            self.save(&session)?;
            self.update_index(&session)?;
        }
        Ok(session)
    }

    /// List all sessions
    pub fn list(&self) -> anyhow::Result<Vec<SessionMeta>> {
        let index = self.load_index()?;
        Ok(index.sessions)
    }

    fn load_index(&self) -> anyhow::Result<SessionIndex> {
        let index_path = self.directory.join("index.json");
        match std::fs::read_to_string(&index_path) {
            Ok(content) => Ok(serde_json::from_str(&content)?),
            Err(_) => Ok(SessionIndex {
                sessions: Vec::new(),
            }),
        }
    }

    /// Update the session index (public, called from engine after save)
    pub fn update_index_for(&self, session: &Session) -> anyhow::Result<()> {
        self.update_index(session)
    }

    fn update_index(&self, session: &Session) -> anyhow::Result<()> {
        let mut index = self.load_index()?;

        // Extract summary from first user message
        let summary = session
            .messages
            .iter()
            .find(|m| m.role == nomi_types::message::Role::User)
            .and_then(|m| {
                m.content.iter().find_map(|c| {
                    if let nomi_types::message::ContentBlock::Text { text } = c {
                        Some(truncate_str(text, 80))
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        let meta = SessionMeta {
            id: session.id.clone(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            model: session.model.clone(),
            summary,
            message_count: session.messages.len(),
        };

        // Update existing or add new
        if let Some(existing) = index.sessions.iter_mut().find(|s| s.id == session.id) {
            *existing = meta;
        } else {
            index.sessions.push(meta);
        }

        let index_path = self.directory.join("index.json");
        write_json_atomic(&index_path, &index)
    }

    /// Remove oldest sessions beyond max_sessions
    fn cleanup_old(&self) -> anyhow::Result<()> {
        let mut index = self.load_index()?;
        if index.sessions.len() <= self.max_sessions {
            return Ok(());
        }

        // Sort by created_at, remove oldest
        index.sessions.sort_by_key(|s| s.created_at);
        let to_remove = index.sessions.len() - self.max_sessions;
        let removed: Vec<_> = index.sessions.drain(..to_remove).collect();

        // Delete session files
        for meta in &removed {
            let pattern = format!("*_{}.json", meta.id);
            if let Ok(paths) = glob::glob(self.directory.join(&pattern).to_string_lossy().as_ref())
            {
                for path in paths.flatten() {
                    let _ = std::fs::remove_file(path);
                }
            }
        }

        // Save updated index
        let index_path = self.directory.join("index.json");
        write_json_atomic(&index_path, &index)?;
        Ok(())
    }

    /// Remove a session by id: delete its `*_{id}.json` file(s) and drop its
    /// index entry. Best-effort and idempotent. Called when a conversation is
    /// deleted so a future conversation that reuses this integer id cannot
    /// resume the stale session (defense-in-depth alongside `owner_token`).
    pub fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        let pattern = format!("*_{}.json", id);
        if let Ok(paths) = glob::glob(self.directory.join(&pattern).to_string_lossy().as_ref()) {
            for path in paths.flatten() {
                let _ = std::fs::remove_file(path);
            }
        }
        let index_path = self.directory.join("index.json");
        if index_path.exists() {
            let mut index = self.load_index()?;
            let before = index.sessions.len();
            index.sessions.retain(|s| s.id != id);
            if index.sessions.len() != before {
                write_json_atomic(&index_path, &index)?;
            }
        }
        Ok(())
    }
}

/// Commit one JSON document without exposing a partially-written transcript
/// or index to a concurrent/crash-time resume. The temporary file is flushed
/// before the platform's replace operation and the containing directory is
/// synced where the platform exposes that durability primitive.
fn write_json_atomic(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("session path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(directory)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session.json");
    let mut temporary = tempfile::Builder::new()
        .prefix(&format!(".{file_name}.write."))
        .tempfile_in(directory)?;
    temporary.write_all(&bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| anyhow::Error::new(error.error))?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

/// Decide whether a loaded session may be resumed for the conversation instance
/// identified by `expected_owner` / `conv_created_ms` (see [`Session::owner_token`]).
/// Missing owner tokens are invalid. The v3 hard cut never accepts or upgrades
/// ownerless historical session files.
pub fn session_belongs_to(
    session_owner: Option<&str>,
    session_created_ms: i64,
    expected_owner: &str,
    conv_created_ms: i64,
) -> bool {
    session_owner == Some(expected_owner) && session_created_ms >= conv_created_ms
}

fn generate_short_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{:06x}", nanos & 0xFFFFFF)
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max - 3).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_types::message::{ContentBlock, Message, Role};
    use tempfile::tempdir;

    #[test]
    fn test_create_session() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);

        let result = manager.create("openai", "gpt-4", "/tmp", None);
        assert!(result.is_ok());

        let session = result.unwrap();
        assert_eq!(session.provider, "openai");
        assert_eq!(session.model, "gpt-4");
        assert_eq!(session.cwd, "/tmp");
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_save_and_load_session() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);

        let mut session = manager
            .create("anthropic", "claude-3", "/home", None)
            .unwrap();
        session.activated_deferred_tools = vec![
            "nomi_knowledge_create_base".into(),
            "nomi_knowledge_update_base".into(),
        ];
        session.editable_turn = Some(EditableTurnCheckpoint {
            source_message_id: "message-root".into(),
            start_len: 0,
            prior_host_context: Default::default(),
        });
        manager.save(&session).unwrap();
        let loaded = manager.load(&session.id).unwrap();

        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.provider, "anthropic");
        assert_eq!(loaded.model, "claude-3");
        assert_eq!(loaded.cwd, "/home");
        assert_eq!(loaded.activated_deferred_tools, session.activated_deferred_tools);
        assert_eq!(loaded.editable_turn, session.editable_turn);
    }

    #[test]
    fn load_durably_recovers_an_interrupted_accepted_turn_root() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);
        let mut session = manager
            .create("anthropic", "claude-3", "/workspace", Some("recover-root"))
            .unwrap();
        let prior = Message::new(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "trusted prior history".to_owned(),
            }],
        );
        let prior_checkpoint = EditableTurnCheckpoint {
            source_message_id: "prior-source".to_owned(),
            start_len: 0,
            prior_host_context: Default::default(),
        };
        let mut prior_host_context = BTreeMap::new();
        prior_host_context.insert("route".to_owned(), "prior".to_owned());
        session.messages = vec![prior.clone()];
        session.editable_turn = Some(prior_checkpoint.clone());
        session.host_context = prior_host_context.clone();
        session.activated_deferred_tools = vec!["prior_tool".to_owned()];
        session.accepted_turn_root = Some(AcceptedTurnRoot {
            source_message_id: "active-source".to_owned(),
            messages: vec![prior.clone()],
            editable_turn: Some(prior_checkpoint.clone()),
            host_context: prior_host_context.clone(),
            activated_deferred_tools: vec!["prior_tool".to_owned()],
        });
        session.messages.push(Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "provisional request".to_owned(),
            }],
        ));
        session.messages.push(Message::new(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "provisional answer".to_owned(),
            }],
        ));
        session.host_context.insert("route".to_owned(), "provisional".to_owned());
        session.activated_deferred_tools.push("provisional_tool".to_owned());
        session.total_usage.input_tokens = 17;
        manager.save(&session).unwrap();
        manager.update_index_for(&session).unwrap();

        let loaded = manager.load("recover-root").unwrap();
        assert_eq!(
            serde_json::to_value(&loaded.messages).unwrap(),
            serde_json::to_value(vec![prior]).unwrap()
        );
        assert_eq!(loaded.editable_turn, Some(prior_checkpoint));
        assert_eq!(loaded.host_context, prior_host_context);
        assert_eq!(loaded.activated_deferred_tools, vec!["prior_tool"]);
        assert_eq!(loaded.total_usage.input_tokens, 17);
        assert!(loaded.accepted_turn_root.is_none());

        let fresh = SessionManager::new(dir.path().to_path_buf(), 10)
            .load("recover-root")
            .unwrap();
        assert_eq!(
            serde_json::to_value(&fresh.messages).unwrap(),
            serde_json::to_value(&loaded.messages).unwrap()
        );
        assert!(fresh.accepted_turn_root.is_none());
        let meta = manager
            .list()
            .unwrap()
            .into_iter()
            .find(|meta| meta.id == "recover-root")
            .unwrap();
        assert_eq!(meta.message_count, 1);
    }

    #[test]
    fn ordinary_load_preserves_a_pending_host_terminal_as_committed_history() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);
        let mut session = manager
            .create("anthropic", "claude-3", "/workspace", Some("pending-host"))
            .unwrap();
        let prior = Message::new(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "trusted prior history".to_owned(),
            }],
        );
        let committed = Message::new(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "host-sealed answer".to_owned(),
            }],
        );
        session.messages = vec![prior.clone(), committed.clone()];
        session.pending_host_terminal_root = Some(AcceptedTurnRoot {
            source_message_id: "host-source".to_owned(),
            messages: vec![prior],
            editable_turn: None,
            host_context: BTreeMap::new(),
            activated_deferred_tools: Vec::new(),
        });
        manager.save(&session).unwrap();
        manager.update_index_for(&session).unwrap();

        let loaded = manager.load("pending-host").unwrap();
        assert_eq!(
            serde_json::to_value(&loaded.messages).unwrap(),
            serde_json::to_value(vec![
                Message::new(
                    Role::Assistant,
                    vec![ContentBlock::Text {
                        text: "trusted prior history".to_owned(),
                    }],
                ),
                committed,
            ])
            .unwrap()
        );
        assert!(loaded.accepted_turn_root.is_none());
        assert_eq!(
            loaded
                .pending_host_terminal_root
                .as_ref()
                .map(|root| root.source_message_id.as_str()),
            Some("host-source")
        );

        let fresh = SessionManager::new(dir.path().to_path_buf(), 10)
            .load("pending-host")
            .unwrap();
        assert_eq!(fresh.messages.len(), 2);
        assert!(fresh.pending_host_terminal_root.is_some());
    }

    #[test]
    fn legacy_session_without_deferred_activations_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);
        let session = manager.create("openai", "gpt-4", "/tmp", None).unwrap();
        let mut value = serde_json::to_value(&session).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("activated_deferred_tools");

        let loaded: Session = serde_json::from_value(value).unwrap();

        assert!(loaded.activated_deferred_tools.is_empty());
    }

    #[test]
    fn legacy_session_without_editable_checkpoint_fails_closed() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);
        let session = manager.create("openai", "gpt-4", "/tmp", None).unwrap();
        let mut value = serde_json::to_value(&session).unwrap();
        value.as_object_mut().unwrap().remove("editable_turn");

        let loaded: Session = serde_json::from_value(value).unwrap();

        assert!(loaded.editable_turn.is_none());
    }

    #[test]
    fn legacy_session_without_accepted_turn_root_is_committed() {
        let manager = SessionManager::new(tempdir().unwrap().path().to_path_buf(), 10);
        let session = manager.create("openai", "gpt-4", "/tmp", None).unwrap();
        let mut value = serde_json::to_value(&session).unwrap();
        value.as_object_mut().unwrap().remove("accepted_turn_root");

        let loaded: Session = serde_json::from_value(value).unwrap();

        assert!(loaded.accepted_turn_root.is_none());
    }

    #[test]
    fn legacy_session_without_pending_host_terminal_root_is_committed() {
        let manager = SessionManager::new(tempdir().unwrap().path().to_path_buf(), 10);
        let session = manager.create("openai", "gpt-4", "/tmp", None).unwrap();
        let mut value = serde_json::to_value(&session).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("pending_host_terminal_root");

        let loaded: Session = serde_json::from_value(value).unwrap();

        assert!(loaded.pending_host_terminal_root.is_none());
    }

    #[test]
    fn test_load_nonexistent_returns_error() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);

        let result = manager.load("nonexistent-id");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_sessions_empty() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);

        let sessions = manager.list().unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_list_sessions_sorted_by_time() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);

        let s1 = manager.create("openai", "gpt-4", "/tmp", None).unwrap();
        let s2 = manager
            .create("anthropic", "claude-3", "/home", None)
            .unwrap();

        let list = manager.list().unwrap();
        assert_eq!(list.len(), 2);

        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&s1.id.as_str()));
        assert!(ids.contains(&s2.id.as_str()));
    }

    #[test]
    fn test_update_index() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);

        let mut session = manager.create("openai", "gpt-4", "/tmp", None).unwrap();

        let msg = Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "hello".to_string(),
            }],
        );
        session.messages.push(msg);

        manager.update_index_for(&session).unwrap();

        let list = manager.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].summary, "hello");
        assert_eq!(list[0].message_count, 1);
    }

    #[test]
    fn test_cleanup_old_sessions() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 2);

        let _s1 = manager.create("openai", "gpt-4", "/tmp", None).unwrap();
        let _s2 = manager.create("openai", "gpt-4", "/tmp", None).unwrap();
        let _s3 = manager.create("openai", "gpt-4", "/tmp", None).unwrap();

        let list = manager.list().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_session_id_uniqueness() {
        let id1 = generate_short_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = generate_short_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn host_context_survives_session_refresh() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);
        let mut session = manager
            .create("openai", "gpt-4", "/tmp", Some("host-context"))
            .unwrap();
        session.host_context.insert(
            "nomifun.image_generation.route".to_owned(),
            "explicit_external".to_owned(),
        );
        manager.save(&session).unwrap();

        let restored = manager.load("host-context").unwrap();
        assert_eq!(
            restored
                .host_context
                .get("nomifun.image_generation.route")
                .map(String::as_str),
            Some("explicit_external")
        );
    }

    #[test]
    fn session_belongs_to_matrix() {
        let conv = 1_782_638_611_752i64; // conversation instance created_at (ms)
        let after = conv + 5_000; // a legitimate session is created at/after that
        // Same stamped instance, session postdates conv → accept.
        assert!(session_belongs_to(
            Some("1782638611752"),
            after,
            "1782638611752",
            conv
        ));
        // Stamped for a different instance → reject.
        assert!(!session_belongs_to(
            Some("1700000000000"),
            after,
            "1782638611752",
            conv
        ));
        // Ownerless files are never accepted, regardless of timestamp.
        assert!(!session_belongs_to(None, conv - 1, "1782638611752", conv));
        assert!(!session_belongs_to(None, after, "1782638611752", conv));
        // Even a correctly stamped file must not predate the conversation row.
        assert!(!session_belongs_to(
            Some("1782638611752"),
            conv - 1,
            "1782638611752",
            conv
        ));
    }

    #[test]
    fn delete_session_removes_only_target_file_and_index_entry() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);

        // Two sessions with explicit ids (mirrors conversation_id-keyed sessions).
        manager.create("openai", "gpt-4", "/tmp", Some("3")).unwrap();
        manager.create("openai", "gpt-4", "/tmp", Some("7")).unwrap();

        manager.delete_session("3").unwrap();

        // The deleted session is gone; the sibling survives.
        assert!(manager.load("3").is_err(), "deleted session must not load");
        assert!(manager.load("7").is_ok(), "sibling session must survive");
        let ids: Vec<String> = manager.list().unwrap().into_iter().map(|m| m.id).collect();
        assert!(!ids.contains(&"3".to_string()), "index must drop the deleted id");
        assert!(ids.contains(&"7".to_string()), "index must keep the sibling id");
    }

    #[test]
    fn delete_session_is_idempotent_when_absent() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf(), 10);
        // No sessions / no index yet — must not error.
        assert!(manager.delete_session("3").is_ok());
    }
}
