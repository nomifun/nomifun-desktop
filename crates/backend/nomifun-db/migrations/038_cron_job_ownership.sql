-- Cron jobs are user-owned aggregates.  Before this hard cut, ownership was
-- guessed at request/event/execution time from an optional Conversation and
-- fell back to the system user.  That made unbound jobs global and allowed a
-- caller to address another user's job by id.  Materialize the owner once and
-- make every runtime/read boundary use it.
--
-- `run_migrations` disables FK enforcement and enables legacy_alter_table
-- outside each sqlx transaction, so this standard SQLite table rebuild does
-- not cascade-delete conversations/runs or rewrite their FK targets.

-- Refuse ambiguous/corrupt historical bindings instead of choosing an owner.
CREATE TABLE cron_job_ownership_migration_guard (
    ok INTEGER NOT NULL CHECK (ok = 1)
);

-- A direct target must exist.
INSERT INTO cron_job_ownership_migration_guard(ok)
SELECT CASE WHEN EXISTS (
    SELECT 1 FROM conversations conversation
    WHERE conversation.id = job.conversation_id
) THEN 1 ELSE 0 END
FROM cron_jobs job
WHERE job.conversation_id IS NOT NULL;

-- Every Conversation that points back to one job must have one owner, and it
-- must agree with the job's direct target owner when both halves are present.
INSERT INTO cron_job_ownership_migration_guard(ok)
SELECT CASE WHEN COUNT(DISTINCT conversation.user_id) = 1 THEN 1 ELSE 0 END
FROM cron_jobs job
JOIN conversations conversation ON conversation.cron_job_id = job.id
GROUP BY job.id;

INSERT INTO cron_job_ownership_migration_guard(ok)
SELECT CASE WHEN direct.user_id = inverse.user_id THEN 1 ELSE 0 END
FROM cron_jobs job
JOIN conversations direct ON direct.id = job.conversation_id
JOIN conversations inverse ON inverse.cron_job_id = job.id;

-- Truly unbound legacy jobs are the sole compatibility case: migrate them to
-- the system account exactly once. Runtime code has no equivalent fallback.
INSERT OR IGNORE INTO users (
    id, username, password_hash, created_at, updated_at
)
SELECT
    'system_default_user', 'system_default_user', '', 0, 0
WHERE EXISTS (
    SELECT 1
    FROM cron_jobs job
    WHERE job.conversation_id IS NULL
      AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE conversation.cron_job_id = job.id
      )
);

INSERT INTO cron_job_ownership_migration_guard(ok)
SELECT CASE WHEN EXISTS (
    SELECT 1 FROM users WHERE id = 'system_default_user'
) THEN 1 ELSE 0 END
WHERE EXISTS (
    SELECT 1
    FROM cron_jobs job
    WHERE job.conversation_id IS NULL
      AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE conversation.cron_job_id = job.id
      )
);

CREATE TABLE cron_jobs_owned (
    id                   TEXT    PRIMARY KEY NOT NULL,
    user_id              TEXT    NOT NULL CHECK(length(trim(user_id)) > 0),
    name                 TEXT    NOT NULL,
    enabled              INTEGER NOT NULL DEFAULT 1,
    schedule_kind        TEXT    NOT NULL CHECK(schedule_kind IN ('at', 'every', 'cron')),
    schedule_value       TEXT    NOT NULL,
    schedule_tz          TEXT,
    schedule_description TEXT,
    payload_message      TEXT    NOT NULL,
    execution_mode       TEXT    NOT NULL DEFAULT 'existing'
                                 CHECK(execution_mode IN ('existing', 'new_conversation')),
    agent_config         TEXT,
    conversation_id      INTEGER,
    conversation_title   TEXT,
    agent_type           TEXT    NOT NULL,
    created_by           TEXT    NOT NULL CHECK(created_by IN ('user', 'agent')),
    skill_content        TEXT,
    description          TEXT,
    target_kind          TEXT    NOT NULL DEFAULT 'agent',
    terminal_mode        TEXT,
    terminal_session_id  INTEGER,
    terminal_command     TEXT,
    terminal_args        TEXT,
    terminal_script      TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    next_run_at          INTEGER,
    last_run_at          INTEGER,
    last_status          TEXT    CHECK(last_status IN ('ok', 'error', 'skipped', 'missed')),
    last_error           TEXT,
    run_count            INTEGER NOT NULL DEFAULT 0,
    retry_count          INTEGER NOT NULL DEFAULT 0,
    max_retries          INTEGER NOT NULL DEFAULT 3,
    preset_id            TEXT,
    preset_revision      INTEGER,
    preset_snapshot      TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE SET NULL,
    FOREIGN KEY (terminal_session_id) REFERENCES terminal_sessions(id) ON DELETE SET NULL
);

INSERT INTO cron_jobs_owned (
    id, user_id, name, enabled, schedule_kind, schedule_value, schedule_tz,
    schedule_description, payload_message, execution_mode, agent_config,
    conversation_id, conversation_title, agent_type, created_by, skill_content,
    description, target_kind, terminal_mode, terminal_session_id,
    terminal_command, terminal_args, terminal_script, created_at, updated_at,
    next_run_at, last_run_at, last_status, last_error, run_count, retry_count,
    max_retries, preset_id, preset_revision, preset_snapshot
)
SELECT
    job.id,
    COALESCE(
        direct.user_id,
        (
            SELECT inverse.user_id
            FROM conversations inverse
            WHERE inverse.cron_job_id = job.id
            ORDER BY inverse.id
            LIMIT 1
        ),
        'system_default_user'
    ),
    job.name, job.enabled, job.schedule_kind, job.schedule_value,
    job.schedule_tz, job.schedule_description, job.payload_message,
    job.execution_mode, job.agent_config, job.conversation_id,
    job.conversation_title, job.agent_type, job.created_by, job.skill_content,
    job.description, job.target_kind, job.terminal_mode,
    job.terminal_session_id, job.terminal_command, job.terminal_args,
    job.terminal_script, job.created_at, job.updated_at, job.next_run_at,
    job.last_run_at, job.last_status, job.last_error, job.run_count,
    job.retry_count, job.max_retries, job.preset_id, job.preset_revision,
    job.preset_snapshot
FROM cron_jobs job
LEFT JOIN conversations direct ON direct.id = job.conversation_id;

DROP TABLE cron_jobs;
ALTER TABLE cron_jobs_owned RENAME TO cron_jobs;

-- Artifact cards are part of the same aggregate boundary: a cron action must
-- never update a card that belongs to another user's Conversation. Historical
-- mismatches are ambiguous ownership and therefore abort the migration.
INSERT INTO cron_job_ownership_migration_guard(ok)
SELECT CASE WHEN EXISTS (
    SELECT 1
    FROM conversations conversation
    JOIN cron_jobs job ON job.id = artifact.cron_job_id
    WHERE conversation.id = artifact.conversation_id
      AND conversation.user_id = job.user_id
) THEN 1 ELSE 0 END
FROM conversation_artifacts artifact
WHERE artifact.cron_job_id IS NOT NULL;

DROP TABLE cron_job_ownership_migration_guard;

CREATE INDEX idx_cron_jobs_user ON cron_jobs(user_id, created_at);
CREATE INDEX idx_cron_jobs_conversation ON cron_jobs(conversation_id);
CREATE INDEX idx_cron_jobs_user_conversation ON cron_jobs(user_id, conversation_id, created_at);
CREATE INDEX idx_cron_jobs_next_run ON cron_jobs(next_run_at) WHERE enabled = 1;
CREATE INDEX idx_cron_jobs_agent_type ON cron_jobs(agent_type);
CREATE INDEX idx_cron_jobs_terminal_session ON cron_jobs(terminal_session_id);
CREATE INDEX idx_cron_jobs_preset_id ON cron_jobs(preset_id);

-- Ownership is immutable. Moving a job means creating a new aggregate.
CREATE TRIGGER cron_job_owner_immutable
BEFORE UPDATE OF user_id ON cron_jobs
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'cron job owner is immutable');
END;

-- Both halves of the optional Conversation binding must remain same-owner.
CREATE TRIGGER cron_job_conversation_owner_insert
BEFORE INSERT ON cron_jobs
WHEN NEW.conversation_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM conversations conversation
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'cron job conversation owner mismatch');
END;

CREATE TRIGGER cron_job_conversation_owner_update
BEFORE UPDATE OF conversation_id ON cron_jobs
WHEN NEW.conversation_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM conversations conversation
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'cron job conversation owner mismatch');
END;

CREATE TRIGGER conversation_cron_job_owner_insert
BEFORE INSERT ON conversations
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM cron_jobs job
     WHERE job.id = NEW.cron_job_id
       AND job.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation cron job owner mismatch');
END;

CREATE TRIGGER conversation_cron_job_owner_update
BEFORE UPDATE OF cron_job_id, user_id ON conversations
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM cron_jobs job
     WHERE job.id = NEW.cron_job_id
       AND job.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation cron job owner mismatch');
END;

CREATE TRIGGER conversation_artifact_cron_job_owner_insert
BEFORE INSERT ON conversation_artifacts
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM conversations conversation
     JOIN cron_jobs job ON job.id = NEW.cron_job_id
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = job.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation artifact cron job owner mismatch');
END;

CREATE TRIGGER conversation_artifact_cron_job_owner_update
BEFORE UPDATE OF conversation_id, cron_job_id ON conversation_artifacts
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM conversations conversation
     JOIN cron_jobs job ON job.id = NEW.cron_job_id
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = job.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation artifact cron job owner mismatch');
END;

CREATE TRIGGER conversation_cron_artifact_owner_immutable
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT OLD.user_id
 AND EXISTS (
     SELECT 1 FROM conversation_artifacts artifact
     WHERE artifact.conversation_id = OLD.id
       AND artifact.cron_job_id IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'cron artifact conversation owner is immutable');
END;
