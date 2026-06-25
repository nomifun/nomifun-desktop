//! TIER 2 multi-agent coordination: a lightweight in-process shared task board
//! (design §3.4). Several sub-agents spawned together can read/write a common
//! list of tasks — claim work to avoid duplication and report progress — instead
//! of running fully blind to one another. Purely in-memory and additive: a
//! sub-agent only sees it when the parent runs a *coordinated* fan-out.

use std::sync::Mutex;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};

use nomi_tools::Tool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub status: TaskStatus,
    pub owner: Option<String>,
    pub notes: String,
}

struct Inner {
    tasks: Vec<Task>,
    seq: u64,
}

/// A shared, lock-guarded task list. Cheap to share via `Arc`; all operations
/// are brief (lock + Vec scan), so a `std::sync::Mutex` is appropriate.
pub struct TaskBoard {
    inner: Mutex<Inner>,
}

impl Default for TaskBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskBoard {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner { tasks: Vec::new(), seq: 0 }),
        }
    }

    /// Add a task and return its id.
    pub fn add(&self, title: &str) -> u64 {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.seq += 1;
        let id = g.seq;
        g.tasks.push(Task {
            id,
            title: title.to_string(),
            status: TaskStatus::Pending,
            owner: None,
            notes: String::new(),
        });
        id
    }

    /// Snapshot of all tasks (cloned).
    pub fn list(&self) -> Vec<Task> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).tasks.clone()
    }

    /// Claim a pending task for `owner`. Errors if missing or already claimed by
    /// someone else — this is the anti-duplication guarantee for siblings.
    pub fn claim(&self, id: u64, owner: &str) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let task = g.tasks.iter_mut().find(|t| t.id == id).ok_or_else(|| format!("no task #{id}"))?;
        match &task.owner {
            Some(existing) if existing != owner => {
                Err(format!("task #{id} already claimed by {existing}"))
            }
            _ => {
                task.owner = Some(owner.to_string());
                task.status = TaskStatus::InProgress;
                Ok(())
            }
        }
    }

    /// Update a task's status and (optionally) append a note.
    pub fn update(&self, id: u64, status: TaskStatus, note: Option<&str>) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let task = g.tasks.iter_mut().find(|t| t.id == id).ok_or_else(|| format!("no task #{id}"))?;
        task.status = status;
        if let Some(n) = note {
            if !task.notes.is_empty() {
                task.notes.push('\n');
            }
            task.notes.push_str(n);
        }
        Ok(())
    }
}

fn parse_status(s: &str) -> Option<TaskStatus> {
    match s {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" => Some(TaskStatus::InProgress),
        "done" => Some(TaskStatus::Done),
        _ => None,
    }
}

fn render_tasks(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return "(no tasks)".to_string();
    }
    tasks
        .iter()
        .map(|t| {
            let owner = t.owner.as_deref().unwrap_or("-");
            let status = serde_json::to_value(t.status).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            format!("#{} [{}] ({}) {}", t.id, status, owner, t.title)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Tool exposing the shared [`TaskBoard`] to a coordinated sub-agent.
pub struct TaskBoardTool {
    board: std::sync::Arc<TaskBoard>,
    agent_name: String,
}

impl TaskBoardTool {
    pub fn new(board: std::sync::Arc<TaskBoard>, agent_name: impl Into<String>) -> Self {
        Self { board, agent_name: agent_name.into() }
    }
}

#[async_trait]
impl Tool for TaskBoardTool {
    fn name(&self) -> &str {
        "shared_tasks"
    }

    fn description(&self) -> &str {
        "Coordinate with sibling sub-agents via a shared task board.\n\n\
         actions:\n\
         - list: see all tasks (id, status, owner, title).\n\
         - add: add a task (title).\n\
         - claim: claim a task by id before working it, so siblings don't duplicate it.\n\
         - update: set a task's status (pending/in_progress/done) and add a note."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "add", "claim", "update"] },
                "title": { "type": "string", "description": "Task title (for add)." },
                "id": { "type": "integer", "description": "Task id (for claim/update)." },
                "status": { "type": "string", "enum": ["pending", "in_progress", "done"], "description": "New status (for update)." },
                "notes": { "type": "string", "description": "A progress note (for update)." }
            },
            "required": ["action"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn describe(&self, input: &Value) -> String {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
        format!("shared_tasks: {action}")
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let ok = |content: String| ToolResult { content, is_error: false, images: Vec::new() };
        let err = |content: String| ToolResult { content, is_error: true, images: Vec::new() };
        match action {
            "list" => ok(render_tasks(&self.board.list())),
            "add" => match input.get("title").and_then(|v| v.as_str()) {
                Some(title) if !title.trim().is_empty() => {
                    let id = self.board.add(title);
                    ok(format!("Added task #{id}: {title}"))
                }
                _ => err("add requires a non-empty `title`".to_string()),
            },
            "claim" => match input.get("id").and_then(|v| v.as_u64()) {
                Some(id) => match self.board.claim(id, &self.agent_name) {
                    Ok(()) => ok(format!("Claimed task #{id} for {}", self.agent_name)),
                    Err(e) => err(e),
                },
                None => err("claim requires an `id`".to_string()),
            },
            "update" => {
                let Some(id) = input.get("id").and_then(|v| v.as_u64()) else {
                    return err("update requires an `id`".to_string());
                };
                let Some(status) = input.get("status").and_then(|v| v.as_str()).and_then(parse_status) else {
                    return err("update requires a `status` of pending/in_progress/done".to_string());
                };
                let note = input.get("notes").and_then(|v| v.as_str());
                match self.board.update(id, status, note) {
                    Ok(()) => ok(format!("Updated task #{id}")),
                    Err(e) => err(e),
                }
            }
            other => err(format!("unknown action '{other}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn add_list_assigns_incrementing_ids() {
        let b = TaskBoard::new();
        let a = b.add("first");
        let c = b.add("second");
        assert_eq!((a, c), (1, 2));
        let tasks = b.list();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].status, TaskStatus::Pending);
        assert!(tasks[0].owner.is_none());
    }

    #[test]
    fn claim_is_exclusive_across_siblings() {
        let b = TaskBoard::new();
        let id = b.add("work");
        assert!(b.claim(id, "agent-a").is_ok());
        // Same owner re-claim is fine; a different sibling is rejected.
        assert!(b.claim(id, "agent-a").is_ok());
        assert!(b.claim(id, "agent-b").is_err(), "a claimed task must not be re-claimable by another agent");
        assert!(b.claim(999, "agent-a").is_err(), "claiming a missing task errors");

        let t = &b.list()[0];
        assert_eq!(t.status, TaskStatus::InProgress);
        assert_eq!(t.owner.as_deref(), Some("agent-a"));
    }

    #[test]
    fn update_sets_status_and_appends_notes() {
        let b = TaskBoard::new();
        let id = b.add("work");
        b.update(id, TaskStatus::InProgress, Some("started")).unwrap();
        b.update(id, TaskStatus::Done, Some("finished")).unwrap();
        let t = &b.list()[0];
        assert_eq!(t.status, TaskStatus::Done);
        assert_eq!(t.notes, "started\nfinished");
        assert!(b.update(404, TaskStatus::Done, None).is_err());
    }

    #[tokio::test]
    async fn tool_add_claim_update_flow() {
        let board = Arc::new(TaskBoard::new());
        let a = TaskBoardTool::new(board.clone(), "agent-a");
        let b = TaskBoardTool::new(board.clone(), "agent-b");

        let r = a.execute(json!({ "action": "add", "title": "build X" })).await;
        assert!(!r.is_error && r.content.contains("#1"), "{}", r.content);

        // agent-a claims #1; agent-b is then refused.
        assert!(!a.execute(json!({ "action": "claim", "id": 1 })).await.is_error);
        let denied = b.execute(json!({ "action": "claim", "id": 1 })).await;
        assert!(denied.is_error, "sibling must not steal a claimed task");

        let r = a.execute(json!({ "action": "update", "id": 1, "status": "done", "notes": "ok" })).await;
        assert!(!r.is_error);

        let listed = a.execute(json!({ "action": "list" })).await;
        assert!(listed.content.contains("#1 [done] (agent-a) build X"), "got: {}", listed.content);
    }

    #[tokio::test]
    async fn tool_validates_inputs() {
        let board = Arc::new(TaskBoard::new());
        let t = TaskBoardTool::new(board, "a");
        assert!(t.execute(json!({ "action": "add" })).await.is_error, "add needs title");
        assert!(t.execute(json!({ "action": "claim" })).await.is_error, "claim needs id");
        assert!(t.execute(json!({ "action": "update", "id": 1 })).await.is_error, "update needs status");
        assert!(t.execute(json!({ "action": "bogus" })).await.is_error);
    }
}
