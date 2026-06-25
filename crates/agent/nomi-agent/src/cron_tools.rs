//! Native tools that let an in-process agent schedule / list / delete its own
//! recurring (cron) jobs through a `CronSink` trait object. The backend injects
//! a concrete sink bound to the agent's conversation; standalone `nomi-cli`
//! passes `None` and these are not registered. Mirrors `requirement_tools`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};

/// One scheduled job, as surfaced to the agent by `CronSink::list`.
#[derive(Debug, Clone)]
pub struct CronJobSummary {
    pub id: String,
    pub name: String,
    /// Human-readable schedule summary (e.g. the cron expression).
    pub schedule: String,
    pub enabled: bool,
}

/// Backend seam for the agent's own scheduled jobs. Implemented by the backend
/// over its `CronService`, bound to the agent's conversation; `nomi-agent` only
/// depends on this trait.
#[async_trait]
pub trait CronSink: Send + Sync {
    /// Schedule `prompt` to re-run on `cron_expr` (5-field cron) in the agent's
    /// own conversation. Returns the new job id.
    async fn create(&self, name: &str, cron_expr: &str, prompt: &str) -> Result<String, String>;

    /// List the agent's scheduled jobs.
    async fn list(&self) -> Result<Vec<CronJobSummary>, String>;

    /// Delete a scheduled job by id.
    async fn delete(&self, job_id: &str) -> Result<(), String>;
}

fn ok(content: String) -> ToolResult {
    ToolResult { content, is_error: false, images: Vec::new() }
}
fn err(content: String) -> ToolResult {
    ToolResult { content, is_error: true, images: Vec::new() }
}

/// `cron_create` — schedule a recurring prompt.
pub struct CronCreateTool {
    sink: Arc<dyn CronSink>,
}
impl CronCreateTool {
    pub fn new(sink: Arc<dyn CronSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for CronCreateTool {
    fn name(&self) -> &str {
        "cron_create"
    }
    fn description(&self) -> &str {
        "Schedule a prompt to re-run automatically on a recurring schedule, in this \
         conversation. Use for 'every morning…', 'check X every 5 minutes', etc. The \
         schedule is a standard 5-field cron expression in the user's local time \
         (minute hour day-of-month month day-of-week)."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short human-readable name for the job" },
                "cron": { "type": "string", "description": "5-field cron expression, e.g. \"0 9 * * 1-5\" (weekdays 9am)" },
                "prompt": { "type": "string", "description": "The prompt to run each time the schedule fires" }
            },
            "required": ["name", "cron", "prompt"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let (Some(name), Some(cron), Some(prompt)) = (
            input["name"].as_str(),
            input["cron"].as_str(),
            input["prompt"].as_str(),
        ) else {
            return err("cron_create requires: name, cron, prompt".to_string());
        };
        match self.sink.create(name, cron, prompt).await {
            Ok(id) => ok(format!("Scheduled job '{name}' ({cron}) — id {id}")),
            Err(e) => err(format!("Failed to schedule job: {e}")),
        }
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
}

/// `cron_list` — list this conversation's scheduled jobs.
pub struct CronListTool {
    sink: Arc<dyn CronSink>,
}
impl CronListTool {
    pub fn new(sink: Arc<dyn CronSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for CronListTool {
    fn name(&self) -> &str {
        "cron_list"
    }
    fn description(&self) -> &str {
        "List the scheduled (cron) jobs for this conversation."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({ "type": "object", "properties": {} })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    async fn execute(&self, _input: Value) -> ToolResult {
        match self.sink.list().await {
            Ok(jobs) if jobs.is_empty() => ok("No scheduled jobs.".to_string()),
            Ok(jobs) => {
                let lines: Vec<String> = jobs
                    .iter()
                    .map(|j| {
                        format!(
                            "- {} [{}] {} ({})",
                            j.id,
                            if j.enabled { "on" } else { "off" },
                            j.name,
                            j.schedule
                        )
                    })
                    .collect();
                ok(lines.join("\n"))
            }
            Err(e) => err(format!("Failed to list jobs: {e}")),
        }
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// `cron_delete` — delete a scheduled job by id.
pub struct CronDeleteTool {
    sink: Arc<dyn CronSink>,
}
impl CronDeleteTool {
    pub fn new(sink: Arc<dyn CronSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for CronDeleteTool {
    fn name(&self) -> &str {
        "cron_delete"
    }
    fn description(&self) -> &str {
        "Delete a scheduled (cron) job by its id (from cron_list)."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": { "job_id": { "type": "string", "description": "The job id to delete" } },
            "required": ["job_id"]
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let Some(job_id) = input["job_id"].as_str() else {
            return err("cron_delete requires: job_id".to_string());
        };
        match self.sink.delete(job_id).await {
            Ok(()) => ok(format!("Deleted scheduled job {job_id}")),
            Err(e) => err(format!("Failed to delete job: {e}")),
        }
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockCronSink {
        created: Mutex<Vec<(String, String, String)>>,
        jobs: Mutex<Vec<CronJobSummary>>,
        deleted: Mutex<Vec<String>>,
        fail: bool,
    }

    #[async_trait]
    impl CronSink for MockCronSink {
        async fn create(&self, name: &str, cron: &str, prompt: &str) -> Result<String, String> {
            if self.fail {
                return Err("boom".into());
            }
            self.created
                .lock()
                .unwrap()
                .push((name.into(), cron.into(), prompt.into()));
            Ok("job-1".into())
        }
        async fn list(&self) -> Result<Vec<CronJobSummary>, String> {
            Ok(self.jobs.lock().unwrap().clone())
        }
        async fn delete(&self, job_id: &str) -> Result<(), String> {
            self.deleted.lock().unwrap().push(job_id.into());
            Ok(())
        }
    }

    #[tokio::test]
    async fn cron_create_calls_sink_and_reports_id() {
        let sink = Arc::new(MockCronSink::default());
        let tool = CronCreateTool::new(sink.clone());
        let r = tool
            .execute(json!({ "name": "daily", "cron": "0 9 * * *", "prompt": "do it" }))
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("job-1"));
        assert_eq!(sink.created.lock().unwrap().len(), 1);
        assert_eq!(sink.created.lock().unwrap()[0].1, "0 9 * * *");
    }

    #[tokio::test]
    async fn cron_create_missing_params_is_error() {
        let tool = CronCreateTool::new(Arc::new(MockCronSink::default()));
        let r = tool.execute(json!({ "name": "x" })).await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn cron_create_surfaces_sink_error() {
        let sink = Arc::new(MockCronSink { fail: true, ..Default::default() });
        let tool = CronCreateTool::new(sink);
        let r = tool
            .execute(json!({ "name": "x", "cron": "* * * * *", "prompt": "p" }))
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("boom"));
    }

    #[tokio::test]
    async fn cron_list_formats_jobs_and_empty() {
        let sink = Arc::new(MockCronSink::default());
        let empty = CronListTool::new(sink.clone()).execute(json!({})).await;
        assert!(empty.content.contains("No scheduled jobs"));

        sink.jobs.lock().unwrap().push(CronJobSummary {
            id: "j1".into(),
            name: "nightly".into(),
            schedule: "0 0 * * *".into(),
            enabled: true,
        });
        let listed = CronListTool::new(sink).execute(json!({})).await;
        assert!(listed.content.contains("j1"));
        assert!(listed.content.contains("nightly"));
    }

    #[tokio::test]
    async fn cron_delete_calls_sink() {
        let sink = Arc::new(MockCronSink::default());
        let r = CronDeleteTool::new(sink.clone())
            .execute(json!({ "job_id": "j9" }))
            .await;
        assert!(!r.is_error);
        assert_eq!(sink.deleted.lock().unwrap()[0], "j9");
    }
}
