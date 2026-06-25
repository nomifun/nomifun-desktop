-- 010_skill_tags.sql
-- Per-skill tag assignments. Skills are filesystem folders (not DB rows),
-- keyed by their unique `name`. Tags are decoupled from skill files so ANY
-- source (builtin/custom/extension/external) is taggable. Built-in seed
-- assignments ship in skill-tags.json and are merged at the route layer;
-- this table holds user assignments/overrides only. JSON-array TEXT columns
-- mirror the assistants' audience_tags/scenario_tags storage. Tag keys
-- reference the shared vocabulary (assistant_tags + tags.json).
CREATE TABLE IF NOT EXISTS skill_tags (
    skill_name    TEXT PRIMARY KEY,
    audience_tags TEXT,
    scenario_tags TEXT,
    updated_at    INTEGER NOT NULL
);
