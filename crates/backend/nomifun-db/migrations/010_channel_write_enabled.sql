-- Migration 009: external-channel write re-enable toggle for knowledge bindings.
--
-- P1/P2 hard-disable knowledge write-back for external IM channel master-agent
-- sessions (discord/slack/lark/…) by default: an unattended bot writing to a
-- shared knowledge base is a standing risk. This column lets the user opt a
-- specific binding back in. When enabled, channel writes are still forced to
-- STAGED placement (review inbox) — never direct — so the human review gate
-- remains the safety net (enforced in `resolve_write_policy`).
--
-- Additive on purpose: editing 001_baseline would change its checksum and trip
-- the pre-baseline rebuild path (database.rs), wiping existing dev DBs. SQLite
-- allows ADD COLUMN with a NOT NULL DEFAULT; the default (0 = disabled)
-- preserves the prior behavior for every pre-existing row.
ALTER TABLE knowledge_bindings
    ADD COLUMN channel_write_enabled INTEGER NOT NULL DEFAULT 0;
