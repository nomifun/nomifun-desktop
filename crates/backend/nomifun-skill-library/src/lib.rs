//! Skill library product: built-in and user skills, market discovery, routes,
//! startup materialization, and runtime skill resolution.

pub mod constants;
pub mod error;
pub mod external_paths;
pub mod market;
pub mod skill_routes;
pub mod skill_service;
pub mod startup_materialize;
mod zip_safe;

pub use constants::*;
pub use error::SkillError;
pub use external_paths::ExternalPathsManager;
pub use skill_routes::{SkillRouterState, skill_routes};
pub use skill_service::{
    BUILTIN_SKILLS_ENV_VAR, BuiltinAutoSkillItem, ExternalSkillSource, NamedPath, ResolvedAgentSkill, ScannedSkill,
    SkillListItem, SkillPaths, SkillSource, builtin_skills_corpus, builtin_skills_corpus_fingerprint,
    builtin_skills_materialize_version, delete_skill, detect_and_count_external_skills, detect_common_skill_paths,
    export_skill_with_symlink, get_skill_paths, import_skill, import_skill_with_symlink, link_workspace_skills,
    list_available_skills, list_builtin_auto_skills, materialize_skills_for_agent, read_builtin_rule,
    read_builtin_skill, read_skill_info, resolve_skill_paths, scan_for_skills,
};
pub use startup_materialize::materialize_if_needed;
