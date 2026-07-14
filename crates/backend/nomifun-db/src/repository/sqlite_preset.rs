//! SQLite implementation of the relational preset catalog.

use nomifun_common::now_ms;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::*;
use crate::repository::preset::{IPresetRepository, IPresetStateRepository, IPresetTagRepository};

#[derive(Clone, Debug)]
pub struct SqlitePresetRepository { pool: SqlitePool }
impl SqlitePresetRepository { pub fn new(pool: SqlitePool) -> Self { Self { pool } } }

#[derive(Clone, Debug)]
pub struct SqlitePresetStateRepository { pool: SqlitePool }
impl SqlitePresetStateRepository { pub fn new(pool: SqlitePool) -> Self { Self { pool } } }

#[derive(Clone, Debug)]
pub struct SqlitePresetTagRepository { pool: SqlitePool }
impl SqlitePresetTagRepository { pub fn new(pool: SqlitePool) -> Self { Self { pool } } }

fn unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(e) if e.code().is_some_and(|c| c == "2067" || c == "1555"))
}

async fn load_record(pool: &SqlitePool, id: &str) -> Result<Option<PresetRecord>, DbError> {
    let Some(preset) = sqlx::query_as::<_, PresetRow>("SELECT * FROM presets WHERE id = ?")
        .bind(id).fetch_optional(pool).await? else { return Ok(None); };
    let localizations = sqlx::query_as::<_, PresetLocalizationRow>(
        "SELECT * FROM preset_localizations WHERE preset_id = ? ORDER BY locale")
        .bind(id).fetch_all(pool).await?;
    let targets = sqlx::query_scalar::<_, String>(
        "SELECT target_kind FROM preset_targets WHERE preset_id = ? ORDER BY target_kind")
        .bind(id).fetch_all(pool).await?;
    let agent_preferences = sqlx::query_as::<_, PresetAgentPreferenceRow>(
        "SELECT * FROM preset_agent_preferences WHERE preset_id = ? ORDER BY rank")
        .bind(id).fetch_all(pool).await?;
    let model_preferences = sqlx::query_as::<_, PresetModelPreferenceRow>(
        "SELECT * FROM preset_model_preferences WHERE preset_id = ? ORDER BY rank")
        .bind(id).fetch_all(pool).await?;
    let skill_bindings = sqlx::query_as::<_, PresetSkillBindingRow>(
        "SELECT * FROM preset_skill_bindings WHERE preset_id = ? ORDER BY binding, sort_order")
        .bind(id).fetch_all(pool).await?;
    let knowledge_policy = sqlx::query_as::<_, PresetKnowledgePolicyRow>(
        "SELECT * FROM preset_knowledge_policy WHERE preset_id = ?")
        .bind(id).fetch_optional(pool).await?;
    let knowledge_bases = sqlx::query_as::<_, PresetKnowledgeBaseRow>(
        "SELECT * FROM preset_knowledge_bases WHERE preset_id = ? ORDER BY sort_order")
        .bind(id).fetch_all(pool).await?;
    let examples = sqlx::query_as::<_, PresetExampleRow>(
        "SELECT * FROM preset_examples WHERE preset_id = ? ORDER BY locale, sort_order")
        .bind(id).fetch_all(pool).await?;
    let tag_bindings = sqlx::query_as::<_, PresetTagBindingRow>(
        "SELECT * FROM preset_tag_bindings WHERE preset_id = ? ORDER BY dimension, tag_key")
        .bind(id).fetch_all(pool).await?;
    let user_state = sqlx::query_as::<_, PresetUserStateRow>(
        "SELECT * FROM preset_user_state WHERE preset_id = ?")
        .bind(id).fetch_optional(pool).await?;
    Ok(Some(PresetRecord {
        preset: Some(preset), localizations, targets, agent_preferences, model_preferences,
        skill_bindings, knowledge_policy, knowledge_bases, examples, tag_bindings, user_state,
    }))
}

async fn replace_bindings(tx: &mut Transaction<'_, Sqlite>, p: &PresetWriteParams) -> Result<(), sqlx::Error> {
    for table in [
        "preset_localizations", "preset_targets", "preset_agent_preferences",
        "preset_model_preferences", "preset_skill_bindings", "preset_knowledge_policy",
        "preset_knowledge_bases", "preset_examples", "preset_tag_bindings",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE preset_id = ?"))
            .bind(&p.id).execute(&mut **tx).await?;
    }
    for (locale, name, description, routing, instructions) in &p.localizations {
        sqlx::query("INSERT INTO preset_localizations (preset_id,locale,name,description,routing_description,instructions) VALUES (?,?,?,?,?,?)")
            .bind(&p.id).bind(locale).bind(name).bind(description).bind(routing).bind(instructions)
            .execute(&mut **tx).await?;
    }
    for target in &p.targets {
        sqlx::query("INSERT INTO preset_targets (preset_id,target_kind) VALUES (?,?)")
            .bind(&p.id).bind(target).execute(&mut **tx).await?;
    }
    for (rank, (agent_id, required)) in p.agent_preferences.iter().enumerate() {
        sqlx::query("INSERT INTO preset_agent_preferences (preset_id,agent_id,rank,required) VALUES (?,?,?,?)")
            .bind(&p.id).bind(agent_id).bind(rank as i64).bind(required).execute(&mut **tx).await?;
    }
    for (rank, (provider_id, model, required)) in p.model_preferences.iter().enumerate() {
        sqlx::query("INSERT INTO preset_model_preferences (preset_id,provider_id,model,rank,required) VALUES (?,?,?,?,?)")
            .bind(&p.id).bind(provider_id).bind(model).bind(rank as i64).bind(required)
            .execute(&mut **tx).await?;
    }
    for (sort_order, (skill_name, binding, required)) in p.skill_bindings.iter().enumerate() {
        sqlx::query("INSERT INTO preset_skill_bindings (preset_id,skill_name,binding,required,sort_order) VALUES (?,?,?,?,?)")
            .bind(&p.id).bind(skill_name).bind(binding).bind(required).bind(sort_order as i64)
            .execute(&mut **tx).await?;
    }
    let (enabled, mode, writeback, eagerness, grounded) = &p.knowledge_policy;
    sqlx::query("INSERT INTO preset_knowledge_policy (preset_id,enabled,mode,writeback,eagerness,grounded) VALUES (?,?,?,?,?,?)")
        .bind(&p.id).bind(enabled).bind(mode).bind(writeback).bind(eagerness).bind(grounded)
        .execute(&mut **tx).await?;
    for (sort_order, (kb, required)) in p.knowledge_bases.iter().enumerate() {
        sqlx::query("INSERT INTO preset_knowledge_bases (preset_id,knowledge_base_id,sort_order,required) VALUES (?,?,?,?)")
            .bind(&p.id).bind(kb).bind(sort_order as i64).bind(required).execute(&mut **tx).await?;
    }
    let mut locale_counts = std::collections::HashMap::<&str, i64>::new();
    for (locale, prompt) in &p.examples {
        let rank = locale_counts.entry(locale.as_str()).or_default();
        sqlx::query("INSERT INTO preset_examples (preset_id,locale,sort_order,prompt) VALUES (?,?,?,?)")
            .bind(&p.id).bind(locale).bind(*rank).bind(prompt).execute(&mut **tx).await?;
        *rank += 1;
    }
    for (tag, dimension) in &p.tag_bindings {
        sqlx::query("INSERT INTO preset_tag_bindings (preset_id,tag_key,dimension) VALUES (?,?,?)")
            .bind(&p.id).bind(tag).bind(dimension).execute(&mut **tx).await?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl IPresetRepository for SqlitePresetRepository {
    async fn list(&self) -> Result<Vec<PresetRecord>, DbError> {
        let ids = sqlx::query_scalar::<_, String>("SELECT id FROM presets ORDER BY updated_at DESC")
            .fetch_all(&self.pool).await?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids { if let Some(record) = load_record(&self.pool, &id).await? { records.push(record); } }
        Ok(records)
    }

    async fn get(&self, id: &str) -> Result<Option<PresetRecord>, DbError> { load_record(&self.pool, id).await }

    async fn create(&self, p: &PresetWriteParams) -> Result<PresetRecord, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("INSERT INTO presets (id,source_kind,source_key,revision,name,description,routing_description,instructions,avatar,fallback_allowed,created_at,updated_at) VALUES (?,?,?,1,?,?,?,?,?,?,?,?)")
            .bind(&p.id).bind(&p.source_kind).bind(&p.source_key).bind(&p.name)
            .bind(&p.description).bind(&p.routing_description).bind(&p.instructions)
            .bind(&p.avatar).bind(p.fallback_allowed).bind(now).bind(now)
            .execute(&mut *tx).await;
        if let Err(error) = result {
            return Err(if unique_violation(&error) { DbError::Conflict(format!("Preset '{}' already exists", p.id)) } else { DbError::Query(error) });
        }
        replace_bindings(&mut tx, p).await?;
        tx.commit().await?;
        load_record(&self.pool, &p.id).await?.ok_or_else(|| DbError::Init("preset create lost row".into()))
    }

    async fn update(&self, id: &str, p: &PresetWriteParams) -> Result<Option<PresetRecord>, DbError> {
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("UPDATE presets SET source_kind=?,source_key=?,revision=revision+1,name=?,description=?,routing_description=?,instructions=?,avatar=?,fallback_allowed=?,updated_at=? WHERE id=?")
            .bind(&p.source_kind).bind(&p.source_key).bind(&p.name).bind(&p.description)
            .bind(&p.routing_description).bind(&p.instructions).bind(&p.avatar)
            .bind(p.fallback_allowed).bind(now_ms()).bind(id).execute(&mut *tx).await?;
        if result.rows_affected() == 0 { tx.rollback().await?; return Ok(None); }
        let mut replacement = p.clone(); replacement.id = id.to_string();
        replace_bindings(&mut tx, &replacement).await?;
        tx.commit().await?;
        load_record(&self.pool, id).await
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM presets WHERE id=?").bind(id).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_rows(&self) -> Result<Vec<PresetRow>, DbError> {
        Ok(sqlx::query_as::<_, PresetRow>("SELECT * FROM presets ORDER BY updated_at DESC")
            .fetch_all(&self.pool).await?)
    }
}

#[async_trait::async_trait]
impl IPresetStateRepository for SqlitePresetStateRepository {
    async fn get(&self, id: &str) -> Result<Option<PresetUserStateRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM preset_user_state WHERE preset_id=?").bind(id).fetch_optional(&self.pool).await?)
    }
    async fn get_all(&self) -> Result<Vec<PresetUserStateRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM preset_user_state").fetch_all(&self.pool).await?)
    }
    async fn upsert(&self, p: &UpsertPresetStateParams) -> Result<PresetUserStateRow, DbError> {
        let now = now_ms();
        sqlx::query("INSERT INTO preset_user_state (preset_id,enabled,auto_selectable,preferred_agent_id,sort_order,last_used_at,updated_at) VALUES (?,?,?,?,?,?,?) ON CONFLICT(preset_id) DO UPDATE SET enabled=excluded.enabled,auto_selectable=excluded.auto_selectable,preferred_agent_id=excluded.preferred_agent_id,sort_order=excluded.sort_order,last_used_at=excluded.last_used_at,updated_at=excluded.updated_at")
            .bind(&p.preset_id).bind(p.enabled).bind(p.auto_selectable).bind(&p.preferred_agent_id).bind(p.sort_order)
            .bind(p.last_used_at).bind(now).execute(&self.pool).await?;
        self.get(&p.preset_id).await?.ok_or_else(|| DbError::Init("preset state upsert lost row".into()))
    }
    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        Ok(sqlx::query("DELETE FROM preset_user_state WHERE preset_id=?").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }
    async fn delete_orphans(&self, valid_ids: &[&str]) -> Result<u64, DbError> {
        if valid_ids.is_empty() { return Ok(sqlx::query("DELETE FROM preset_user_state").execute(&self.pool).await?.rows_affected()); }
        let placeholders = std::iter::repeat_n("?", valid_ids.len()).collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM preset_user_state WHERE preset_id NOT IN ({placeholders})");
        let mut q = sqlx::query(&sql);
        for id in valid_ids { q = q.bind(id); }
        Ok(q.execute(&self.pool).await?.rows_affected())
    }
}

#[async_trait::async_trait]
impl IPresetTagRepository for SqlitePresetTagRepository {
    async fn list(&self) -> Result<Vec<PresetTagRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM preset_tags ORDER BY dimension,sort_order,created_at").fetch_all(&self.pool).await?)
    }
    async fn get(&self, key: &str) -> Result<Option<PresetTagRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM preset_tags WHERE key=?").bind(key).fetch_optional(&self.pool).await?)
    }
    async fn create(&self, p: &CreatePresetTagParams<'_>) -> Result<PresetTagRow, DbError> {
        let now = now_ms();
        sqlx::query("INSERT INTO preset_tags (key,dimension,label,sort_order,created_at) VALUES (?,?,?,?,?)")
            .bind(p.key).bind(p.dimension).bind(p.label).bind(p.sort_order).bind(now)
            .execute(&self.pool).await.map_err(|e| if unique_violation(&e) { DbError::Conflict(format!("Preset tag '{}' already exists", p.key)) } else { DbError::Query(e) })?;
        self.get(p.key).await?.ok_or_else(|| DbError::Init("preset tag create lost row".into()))
    }
    async fn update(&self, key: &str, p: &UpdatePresetTagParams<'_>) -> Result<Option<PresetTagRow>, DbError> {
        let Some(mut row) = self.get(key).await? else { return Ok(None); };
        if let Some(label) = p.label { row.label = label.to_string(); }
        if let Some(sort) = p.sort_order { row.sort_order = sort; }
        sqlx::query("UPDATE preset_tags SET label=?,sort_order=? WHERE key=?")
            .bind(&row.label).bind(row.sort_order).bind(key).execute(&self.pool).await?;
        Ok(Some(row))
    }
    async fn delete(&self, key: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM preset_tag_bindings WHERE tag_key=?").bind(key).execute(&mut *tx).await?;
        let changed = sqlx::query("DELETE FROM preset_tags WHERE key=?").bind(key).execute(&mut *tx).await?.rows_affected() > 0;
        tx.commit().await?; Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::init_database_memory;

    fn sample(id: &str) -> PresetWriteParams {
        PresetWriteParams {
            id: id.into(), source_kind: "user".into(), source_key: Some(id.into()),
            name: "Research preset".into(), description: Some("Reusable research role".into()),
            routing_description: Some("Use for evidence gathering".into()),
            instructions: "Cite primary sources.".into(), avatar: None, fallback_allowed: true,
            localizations: vec![("zh-CN".into(), Some("研究设定".into()), None, None, Some("引用一手来源。".into()))],
            targets: vec!["conversation".into(), "execution_step".into()],
            agent_preferences: vec![("nomi".into(), false)],
            model_preferences: vec![(Some("prov_x".into()), "model_x".into(), true)],
            skill_bindings: vec![("web-search".into(), "include".into(), true), ("unsafe-auto".into(), "exclude_auto".into(), false)],
            knowledge_policy: (true, "staged".into(), false, Some("conservative".into()), true),
            knowledge_bases: vec![("kb_docs".into(), true)],
            examples: vec![(String::new(), "Research this topic".into())],
            tag_bindings: vec![("audience-engineer".into(), "audience".into())],
        }
    }

    #[tokio::test]
    async fn preset_aggregate_round_trip_and_revision() {
        let db = init_database_memory().await.unwrap();
        let repo = SqlitePresetRepository::new(db.pool().clone());
        let created = repo.create(&sample("preset_test")).await.unwrap();
        assert_eq!(created.preset.as_ref().unwrap().revision, 1);
        assert_eq!(created.model_preferences[0].provider_id.as_deref(), Some("prov_x"));
        assert_eq!(created.skill_bindings.len(), 2);
        assert_eq!(created.knowledge_bases[0].knowledge_base_id, "kb_docs");
        let updated = repo.update("preset_test", &sample("preset_test")).await.unwrap().unwrap();
        assert_eq!(updated.preset.unwrap().revision, 2);
    }

    #[tokio::test]
    async fn migration_removes_legacy_template_tables() {
        let db = init_database_memory().await.unwrap();
        for table in ["assistants", "assistant_overrides", "assistant_tags"] {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table).fetch_one(db.pool()).await.unwrap();
            assert_eq!(count, 0, "legacy table {table} must not survive migration");
        }
        for table in ["presets", "preset_agent_preferences", "preset_model_preferences", "preset_skill_bindings", "preset_knowledge_bases", "preset_user_state"] {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table).fetch_one(db.pool()).await.unwrap();
            assert_eq!(count, 1, "preset table {table} must exist");
        }
    }
}
