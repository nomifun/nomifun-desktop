use crate::error::DbError;
use crate::models::{SkillTagRow, UpsertSkillTagParams};

/// CRUD for per-skill tag assignments (keyed by skill name).
#[async_trait::async_trait]
pub trait ISkillTagRepository: Send + Sync {
    async fn get_all(&self) -> Result<Vec<SkillTagRow>, DbError>;
    async fn upsert(&self, params: &UpsertSkillTagParams<'_>) -> Result<SkillTagRow, DbError>;
    async fn delete(&self, skill_name: &str) -> Result<bool, DbError>;
}
