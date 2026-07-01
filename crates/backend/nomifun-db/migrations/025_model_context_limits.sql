-- Migration 025: Store per-model context window limits.
-- JSON object: model_id -> context window token count.

ALTER TABLE providers ADD COLUMN model_context_limits TEXT NOT NULL DEFAULT '{}';
