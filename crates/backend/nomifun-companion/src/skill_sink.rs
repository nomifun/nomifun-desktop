//! `CompanionSkillStoreSink` — bridges the companion's skill registry + on-disk
//! SKILL.md bodies to the `nomifun_ai_agent::CompanionSkillSink` trait the agent
//! engine consumes for skill auto-use (design §7).
//!
//! `active_skills` feeds the per-turn `when_to_use` index (the `CompanionSkillContributor`);
//! `load_skill_body` resolves a named skill's SKILL.md on demand (the `companion_skill` tool).
//! Both are scoped to ONE companion — the owner resolved from the roster — because
//! a skill belongs to exactly one companion; there is no shared tier to fall back on.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::{CompanionSkillSink, SkillListing};
use nomifun_extension::constants::SKILL_MANIFEST_FILE;
use nomifun_extension::skill_service::{self, SkillPaths, SkillScope};

use crate::collector::SharedConfig;
use crate::registry::CompanionRegistry;
use crate::store::CompanionStore;

pub struct CompanionSkillStoreSink {
    pub store: CompanionStore,
    pub config: SharedConfig,
    pub registry: Arc<CompanionRegistry>,
    pub skill_paths: Arc<SkillPaths>,
}

impl CompanionSkillStoreSink {
    /// The companion whose skills this sink serves, via the ONE owner-resolution
    /// rule ([`CompanionRegistry::resolve_row_owner`]): the explicit default
    /// companion, else the oldest. Reading `default_companion_id` directly would
    /// be a second, divergent rule — and it is unset on most installs, which used
    /// to mean the agent silently saw no self-evolved skills at all.
    async fn owner(&self) -> Option<String> {
        let default_companion_id = self.config.read().await.default_companion_id.clone();
        self.registry
            .resolve_row_owner(default_companion_id.as_deref())
            .await
    }
}

#[async_trait]
impl CompanionSkillSink for CompanionSkillStoreSink {
    async fn active_skills(&self) -> Vec<SkillListing> {
        let Some(owner) = self.owner().await else {
            return Vec::new();
        };
        let skills = self.store.list_skills(&owner).await.unwrap_or_default();
        let mut out = Vec::new();
        for s in skills.into_iter().filter(|s| s.status == "active") {
            let scope = SkillScope::Companion(owner.clone());
            // when_to_use index uses the SKILL.md description (what the skill does).
            if let Ok(dir) = skill_service::skill_dir_for(&self.skill_paths, &scope, &s.skill_name, false) {
                if let Ok((_, desc)) = skill_service::read_skill_info(&dir).await {
                    out.push(SkillListing { name: s.skill_name, when_to_use: desc });
                }
            }
        }
        out
    }

    async fn load_skill_body(&self, name: &str) -> Option<String> {
        let owner = self.owner().await?;
        let dir = skill_service::skill_dir_for(
            &self.skill_paths,
            &SkillScope::Companion(owner.clone()),
            name,
            false,
        )
        .ok()?;
        let body = tokio::fs::read_to_string(dir.join(SKILL_MANIFEST_FILE)).await.ok()?;
        let _ = self
            .store
            .record_skill_usage_by_name(&owner, name, nomifun_common::now_ms())
            .await;
        Some(body)
    }
}
