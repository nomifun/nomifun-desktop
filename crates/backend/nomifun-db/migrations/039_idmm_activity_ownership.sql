-- IDMM audit records contain private session context. Earlier schemas keyed
-- them only by a polymorphic target handle, which made the cross-session
-- activity feed process-global.  Rebuild the table with an authoritative user
-- owner and retain only rows whose owner can be recovered from the target.

CREATE TABLE idmm_interventions_owned (
    id           TEXT PRIMARY KEY NOT NULL,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE
                         CHECK (length(trim(user_id)) > 0),
    target_kind  TEXT    NOT NULL CHECK (target_kind IN ('conversation', 'terminal')),
    target_id    TEXT    NOT NULL CHECK (length(trim(target_id)) > 0),
    watch        TEXT    NOT NULL,
    at           INTEGER NOT NULL,
    signal       TEXT    NOT NULL,
    tier_used    TEXT    NOT NULL,
    category     TEXT,
    action       TEXT    NOT NULL,
    detail       TEXT,
    reason       TEXT,
    confidence   REAL,
    bypass_model TEXT,
    outcome      TEXT    NOT NULL
);

INSERT INTO idmm_interventions_owned (
    id, user_id, target_kind, target_id, watch, at, signal, tier_used,
    category, action, detail, reason, confidence, bypass_model, outcome
)
SELECT
    i.id,
    CASE
        WHEN i.target_kind = 'conversation' THEN c.user_id
        WHEN i.target_kind = 'terminal' THEN t.user_id
    END,
    i.target_kind, i.target_id, i.watch, i.at, i.signal, i.tier_used,
    i.category, i.action, i.detail, i.reason, i.confidence,
    i.bypass_model, i.outcome
FROM idmm_interventions AS i
LEFT JOIN conversations AS c
    ON i.target_kind = 'conversation'
   AND CAST(c.id AS TEXT) = i.target_id
LEFT JOIN terminal_sessions AS t
    ON i.target_kind = 'terminal'
   AND CAST(t.id AS TEXT) = i.target_id
WHERE CASE
    WHEN i.target_kind = 'conversation' THEN NULLIF(TRIM(c.user_id), '')
    WHEN i.target_kind = 'terminal' THEN NULLIF(TRIM(t.user_id), '')
END IS NOT NULL;

DROP TABLE idmm_interventions;
ALTER TABLE idmm_interventions_owned RENAME TO idmm_interventions;

CREATE INDEX idx_idmm_interventions_owner_target
    ON idmm_interventions(user_id, target_kind, target_id, at DESC, id DESC);
CREATE INDEX idx_idmm_interventions_owner_activity
    ON idmm_interventions(user_id, at DESC, id DESC);
CREATE INDEX idx_idmm_interventions_at
    ON idmm_interventions(at);

-- `user_id` is not caller-supplied metadata: it must equal the authoritative
-- owner of the polymorphic target.  Keep this in the database as the final
-- fail-closed boundary so a future repository/raw-SQL caller cannot forge an
-- activity row for another account or retain a row for an already-gone target.
CREATE TRIGGER idmm_intervention_target_owner_insert_guard
BEFORE INSERT ON idmm_interventions
BEGIN
    SELECT CASE
        WHEN NEW.target_kind = 'conversation'
         AND NOT EXISTS (
             SELECT 1
             FROM conversations AS conversation
             WHERE CAST(conversation.id AS TEXT) = NEW.target_id
               AND conversation.user_id = NEW.user_id
         )
        THEN RAISE(ABORT, 'IDMM conversation target owner mismatch')
        WHEN NEW.target_kind = 'terminal'
         AND NOT EXISTS (
             SELECT 1
             FROM terminal_sessions AS terminal
             WHERE CAST(terminal.id AS TEXT) = NEW.target_id
               AND terminal.user_id = NEW.user_id
         )
        THEN RAISE(ABORT, 'IDMM terminal target owner mismatch')
    END;
END;

-- Intervention rows are append-only audit facts.  Correcting an outcome or
-- owner creates a new row; no field (including non-identity detail) may be
-- rewritten after publication.
CREATE TRIGGER idmm_intervention_audit_immutable
BEFORE UPDATE ON idmm_interventions
BEGIN
    SELECT RAISE(ABORT, 'IDMM intervention audit rows are immutable');
END;
