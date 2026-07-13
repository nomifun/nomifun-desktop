use crate::error::DbError;
use crate::models::{
    CreatePresetTagParams, PresetRecord, PresetRow, PresetTagRow, PresetUserStateRow,
    PresetWriteParams, UpdatePresetTagParams, UpsertPresetStateParams,
};

#[async_trait::async_trait]
pub trait IPresetRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<PresetRecord>, DbError>;
    async fn get(&self, id: &str) -> Result<Option<PresetRecord>, DbError>;
    async fn create(&self, params: &PresetWriteParams) -> Result<PresetRecord, DbError>;
    /// Replaces all authored fields and bindings and increments revision.
    async fn update(&self, id: &str, params: &PresetWriteParams) -> Result<Option<PresetRecord>, DbError>;
    async fn delete(&self, id: &str) -> Result<bool, DbError>;
    async fn list_rows(&self) -> Result<Vec<PresetRow>, DbError>;
}

#[async_trait::async_trait]
pub trait IPresetStateRepository: Send + Sync {
    async fn get(&self, preset_id: &str) -> Result<Option<PresetUserStateRow>, DbError>;
    async fn get_all(&self) -> Result<Vec<PresetUserStateRow>, DbError>;
    async fn upsert(&self, params: &UpsertPresetStateParams) -> Result<PresetUserStateRow, DbError>;
    async fn delete(&self, preset_id: &str) -> Result<bool, DbError>;
    async fn delete_orphans(&self, valid_ids: &[&str]) -> Result<u64, DbError>;
}

#[async_trait::async_trait]
pub trait IPresetTagRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<PresetTagRow>, DbError>;
    async fn get(&self, key: &str) -> Result<Option<PresetTagRow>, DbError>;
    async fn create(&self, params: &CreatePresetTagParams<'_>) -> Result<PresetTagRow, DbError>;
    async fn update(&self, key: &str, params: &UpdatePresetTagParams<'_>) -> Result<Option<PresetTagRow>, DbError>;
    async fn delete(&self, key: &str) -> Result<bool, DbError>;
}
