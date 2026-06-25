-- Migration 002: write-back "eagerness" (回写意识) for knowledge bindings.
--
-- Adds a SECOND, orthogonal write-back axis to `knowledge_bindings`. The
-- existing `writeback_mode` controls WHERE agent writes land (`staged` inbox
-- vs `direct` body); this new `writeback_eagerness` controls HOW EAGERLY the
-- agent writes at all, while write-back is enabled:
--   * 'conservative' — the historical, restrained default: only persist
--     knowledge the model judges clearly worth keeping.
--   * 'aggressive'    — capture anything plausibly relevant to a mounted base
--     without much hesitation; the user prunes later.
-- Both are purely prompt-contract wording (rendered by
-- nomifun-knowledge::context); the column only persists the user's pick.
--
-- Additive on purpose: editing 001_baseline would change its checksum and
-- trip the pre-baseline rebuild path (database.rs), wiping existing dev DBs.
-- SQLite allows ADD COLUMN with a NOT NULL DEFAULT + CHECK; the default
-- satisfies the CHECK for every pre-existing row.
ALTER TABLE knowledge_bindings
    ADD COLUMN writeback_eagerness TEXT NOT NULL DEFAULT 'conservative'
        CHECK(writeback_eagerness IN ('conservative', 'aggressive'));
