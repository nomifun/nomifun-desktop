//! Assistant source classification + rule/skill dispatch traits used by
//! `skill_routes` to route rule-md / skill-md reads/writes to the correct
//! source (built-in file, extension resolution, or user-writable directory).
//!
//! These traits live in `nomifun-extension` (not `nomifun-assistant`) so
//! `skill_routes` can depend on them without pulling `nomifun-assistant` into
//! the dependency graph; the concrete implementation ships from
//! `nomifun-assistant::AssistantService`.

use nomifun_api_types::AssistantSource;
use nomifun_common::AppError;

/// Classify an assistant id into its source (builtin / extension / user).
#[async_trait::async_trait]
pub trait AssistantClassifier: Send + Sync {
    /// Return the source of the assistant. Callers treat `User` as "not
    /// known to builtins or extensions"; confirming existence in the user
    /// table is the repository's job.
    async fn classify(&self, id: &str) -> AssistantSource;
}

/// Source-dispatched read/write access for assistant rule/skill md files.
///
/// Implemented by `nomifun_assistant::AssistantService`; depended on by
/// `skill_routes` so the existing `/api/skills/assistant-rule/*` and
/// `/api/skills/assistant-skill/*` endpoints dispatch per source.
#[async_trait::async_trait]
pub trait AssistantRuleDispatcher: Send + Sync {
    async fn read_rule(&self, id: &str, locale: Option<&str>) -> Result<String, AppError>;
    async fn write_rule(&self, id: &str, locale: Option<&str>, content: &str) -> Result<(), AppError>;
    async fn delete_rule(&self, id: &str) -> Result<bool, AppError>;

    async fn read_skill(&self, id: &str, locale: Option<&str>) -> Result<String, AppError>;
    async fn write_skill(&self, id: &str, locale: Option<&str>, content: &str) -> Result<(), AppError>;
    async fn delete_skill(&self, id: &str) -> Result<bool, AppError>;
}
