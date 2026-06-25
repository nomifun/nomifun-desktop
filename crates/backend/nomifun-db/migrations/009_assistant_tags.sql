-- 009_assistant_tags.sql
-- Two-dimension tagging for assistants (audience / scenario).
-- Per-assistant tag keys live as JSON arrays on the assistants table
-- (mirrors enabled_skills). The vocabulary's user-created entries live in
-- assistant_tags; built-in seed tags ship in tags.json (no rows here).

ALTER TABLE assistants ADD COLUMN audience_tags TEXT;
ALTER TABLE assistants ADD COLUMN scenario_tags TEXT;

-- User-created tag vocabulary only. Built-in seed tags are served from the
-- embedded tags.json manifest and merged at the service layer, so they are
-- NOT rows here. `dimension` is 'audience' | 'scenario'. No FK to assistants
-- (assistants reference tags by key in their JSON arrays; deletion cleanup is
-- done by the service).
CREATE TABLE IF NOT EXISTS assistant_tags (
    key         TEXT PRIMARY KEY,
    dimension   TEXT NOT NULL CHECK (dimension IN ('audience', 'scenario')),
    label       TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_assistant_tags_dimension ON assistant_tags(dimension, sort_order);
