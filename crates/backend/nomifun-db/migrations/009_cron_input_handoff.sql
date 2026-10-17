-- Input handoff closes one scheduled invocation without declaring failure or
-- replaying its effects. Canonical cancellation is its own nonfailure terminal. Preserve existing rows and AUTOINCREMENT high-water marks.
-- The migration owner suspends and restores FK enforcement for replacements.
CREATE TEMP TABLE cron_wait_sequences (name TEXT PRIMARY KEY, seq INTEGER NOT NULL);
INSERT INTO cron_wait_sequences SELECT name, seq FROM sqlite_sequence
WHERE name IN ('cron_job_runs', 'cron_jobs', 'cron_run_reservations');

CREATE TABLE cron_job_runs_next (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_job_run_id TEXT NOT NULL UNIQUE
                    CHECK (
                        length(cron_job_run_id) = 36
                        AND lower(cron_job_run_id) = cron_job_run_id
                        AND cron_job_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(cron_job_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    cron_job_id     TEXT NOT NULL
                    CHECK (
                        length(cron_job_id) = 36
                        AND lower(cron_job_id) = cron_job_id
                        AND cron_job_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(cron_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    executed_at_ms  INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('ok', 'error', 'skipped', 'missed', 'input_required', 'cancelled')),
    created_at_ms   INTEGER NOT NULL
);
INSERT INTO cron_job_runs_next SELECT * FROM cron_job_runs;
DROP TABLE cron_job_runs;
ALTER TABLE cron_job_runs_next RENAME TO cron_job_runs;

CREATE TABLE cron_jobs_next (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_job_id          TEXT NOT NULL UNIQUE
                         CHECK (
                             length(cron_job_id) = 36
                             AND lower(cron_job_id) = cron_job_id
                             AND cron_job_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(cron_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    user_id              TEXT NOT NULL,
    name                 TEXT NOT NULL,
    enabled              INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    schedule_kind        TEXT NOT NULL CHECK (schedule_kind IN ('at', 'every', 'cron')),
    schedule_value       TEXT NOT NULL,
    schedule_tz          TEXT,
    schedule_description TEXT,
    payload_message      TEXT NOT NULL,
    execution_mode       TEXT NOT NULL DEFAULT 'existing'
                         CHECK (execution_mode IN ('existing', 'new_conversation')),
    agent_config         TEXT CHECK (
                             agent_config IS NULL
                             OR (json_valid(agent_config) AND json_type(agent_config) = 'object')
                         ),
    preset_id            TEXT,
    preset_revision      INTEGER,
    agent_snapshot      TEXT,
    conversation_id      TEXT,
    conversation_title   TEXT,
    agent_type           TEXT NOT NULL,
    created_by           TEXT NOT NULL CHECK (created_by IN ('user', 'agent')),
    skill_content        TEXT,
    description          TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    next_run_at          INTEGER,
    last_run_at          INTEGER,
    last_status          TEXT CHECK (last_status IN ('ok', 'error', 'skipped', 'missed', 'input_required', 'cancelled')),
    last_error           TEXT,
    run_count            INTEGER NOT NULL DEFAULT 0,
    retry_count          INTEGER NOT NULL DEFAULT 0,
    max_retries          INTEGER NOT NULL DEFAULT 3 CHECK (max_retries >= 0), schedule_revision INTEGER NOT NULL DEFAULT 1
        CHECK (schedule_revision > 0),
    CHECK (conversation_id IS NULL OR (length(conversation_id) = 36 AND lower(conversation_id) = conversation_id AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (preset_id IS NULL OR (length(preset_id) = 36 AND lower(preset_id) = preset_id AND preset_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(preset_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);
INSERT INTO cron_jobs_next SELECT * FROM cron_jobs;
DROP TABLE cron_jobs;
ALTER TABLE cron_jobs_next RENAME TO cron_jobs;

CREATE TABLE cron_run_reservations_next (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_job_run_id     TEXT NOT NULL UNIQUE
                        CHECK (
                            length(cron_job_run_id) = 36
                            AND lower(cron_job_run_id) = cron_job_run_id
                            AND cron_job_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(cron_job_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    cron_job_id         TEXT NOT NULL
                        CHECK (
                            length(cron_job_id) = 36
                            AND lower(cron_job_id) = cron_job_id
                            AND cron_job_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(cron_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    trigger_kind        TEXT NOT NULL CHECK (trigger_kind IN ('scheduled', 'run_now')),
    operation_key       TEXT NOT NULL UNIQUE CHECK (length(operation_key) BETWEEN 1 AND 1024),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) BETWEEN 1 AND 1024),
    schedule_revision   INTEGER CHECK (schedule_revision > 0),
    planned_at_ms       INTEGER,
    status              TEXT NOT NULL DEFAULT 'reserved'
                        CHECK (status IN ('reserved', 'ok', 'error', 'skipped', 'missed', 'input_required', 'cancelled')),
    conversation_id     TEXT,
    result_error        TEXT,
    created_at_ms       INTEGER NOT NULL,
    updated_at_ms       INTEGER NOT NULL,
    settled_at_ms       INTEGER, job_projection_state TEXT NOT NULL DEFAULT 'legacy_unknown'
        CHECK (job_projection_state IN ('legacy_unknown', 'pending', 'applied')), job_projected_at_ms INTEGER,
    CHECK (
        (trigger_kind = 'scheduled' AND schedule_revision IS NOT NULL AND planned_at_ms IS NOT NULL)
        OR
        (trigger_kind = 'run_now' AND schedule_revision IS NULL AND planned_at_ms IS NULL)
    ),
    CHECK (
        (status = 'reserved' AND settled_at_ms IS NULL)
        OR
        (status <> 'reserved' AND settled_at_ms IS NOT NULL)
    ),
    CHECK (
        conversation_id IS NULL
        OR (
            length(conversation_id) = 36
            AND lower(conversation_id) = conversation_id
            AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    )
);
INSERT INTO cron_run_reservations_next SELECT * FROM cron_run_reservations;
DROP TABLE cron_run_reservations;
ALTER TABLE cron_run_reservations_next RENAME TO cron_run_reservations;
CREATE INDEX idx_cron_job_runs_cron_job_id ON cron_job_runs(cron_job_id);
CREATE INDEX idx_cron_jobs_next_run ON cron_jobs(enabled, next_run_at);
CREATE INDEX idx_cron_jobs_owner_conversation ON cron_jobs(user_id, conversation_id, created_at);
CREATE INDEX idx_cron_run_reservations_cron_job_id ON cron_run_reservations(cron_job_id, status, created_at_ms, id);
CREATE INDEX idx_cron_run_reservations_projection ON cron_run_reservations(status, job_projection_state, created_at_ms);
CREATE UNIQUE INDEX uq_cron_run_reservations_scheduled_occurrence ON cron_run_reservations(cron_job_id, schedule_revision, planned_at_ms) WHERE trigger_kind = 'scheduled';

UPDATE sqlite_sequence SET seq = MAX(seq, COALESCE((SELECT old.seq FROM cron_wait_sequences old WHERE old.name = sqlite_sequence.name), 0))
WHERE name IN ('cron_job_runs', 'cron_jobs', 'cron_run_reservations');
DROP TABLE cron_wait_sequences;
CREATE TEMP TABLE cron_wait_integrity (valid INTEGER NOT NULL CHECK(valid = 1));
INSERT INTO cron_wait_integrity SELECT CASE WHEN COUNT(*) = 0 THEN 1 ELSE 0 END FROM pragma_foreign_key_check;
DROP TABLE cron_wait_integrity;
