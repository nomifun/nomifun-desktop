-- Per-user Kylon routing and durable bridge jobs for the sales workspace.
--
-- Kylon credentials themselves stay in the worker's isolated tenant HOME.
-- This database stores only non-secret routing identifiers.  Every application
-- query is additionally scoped by the authenticated NomiFun user id.
CREATE TABLE sales_kylon_bindings (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      TEXT NOT NULL UNIQUE
                 CHECK (
                     length(user_id) = 36
                     AND lower(user_id) = user_id
                     AND user_id GLOB '????????-????-7???-[89ab]???-????????????'
                     AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                 ),
    workspace_id TEXT NOT NULL DEFAULT '' CHECK (length(workspace_id) <= 128),
    room_id      TEXT NOT NULL DEFAULT '' CHECK (length(room_id) <= 128),
    agent_id     TEXT NOT NULL DEFAULT '' CHECK (length(agent_id) <= 128),
    enabled      INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE INDEX idx_sales_kylon_bindings_user_id
    ON sales_kylon_bindings(user_id);

CREATE TABLE sales_kylon_jobs (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    kylon_job_id          TEXT NOT NULL UNIQUE
                          CHECK (
                              length(kylon_job_id) = 36
                              AND lower(kylon_job_id) = kylon_job_id
                              AND kylon_job_id GLOB '????????-????-7???-[89ab]???-????????????'
                              AND replace(kylon_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                          ),
    user_id               TEXT NOT NULL
                          CHECK (
                              length(user_id) = 36
                              AND lower(user_id) = user_id
                              AND user_id GLOB '????????-????-7???-[89ab]???-????????????'
                              AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                          ),
    task_id               TEXT NOT NULL CHECK (length(task_id) BETWEEN 1 AND 128),
    status                TEXT NOT NULL
                          CHECK (status IN ('dispatching', 'running', 'completed', 'failed')),
    remote_thread_id      TEXT CHECK (remote_thread_id IS NULL OR length(remote_thread_id) <= 128),
    snapshot_json         TEXT NOT NULL DEFAULT '{}'
                          CHECK (json_valid(snapshot_json) AND json_type(snapshot_json) = 'object'),
    error                 TEXT,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    UNIQUE(user_id, task_id)
);

CREATE INDEX idx_sales_kylon_jobs_user_id
    ON sales_kylon_jobs(user_id);
CREATE INDEX idx_sales_kylon_jobs_user_status
    ON sales_kylon_jobs(user_id, status, updated_at);
