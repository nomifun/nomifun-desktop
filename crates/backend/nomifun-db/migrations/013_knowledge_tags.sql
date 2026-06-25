-- 013_knowledge_tags.sql
-- User-defined tags for knowledge bases. The `tags` column on knowledge_bases
-- stores a JSON array of tag keys assigned to that base (NULL = no tags).
-- The `knowledge_tags` table holds the tag definitions (palette).

ALTER TABLE knowledge_bases ADD COLUMN tags TEXT;  -- JSON array text; NULL = no tags

CREATE TABLE knowledge_tags (
  key        TEXT PRIMARY KEY,
  label      TEXT NOT NULL,
  color      TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
