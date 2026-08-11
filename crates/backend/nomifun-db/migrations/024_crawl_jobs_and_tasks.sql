-- Crawl jobs and their durable URL frontier.
--
-- The frontier carries the same claim protocol as AutoWork (migrations
-- 005/009/010): monotonic `claim_generation` for audit, unforgeable
-- `claim_token` as the fencing capability, and a renewable lease so a dead
-- worker's task returns to the queue without any liveness probe.

CREATE TABLE crawl_jobs (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id               TEXT NOT NULL UNIQUE
                         CHECK (
                             length(job_id) = 36
                             AND lower(job_id) = job_id
                             AND job_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    user_id              TEXT NOT NULL
                         CHECK (
                             length(user_id) = 36
                             AND lower(user_id) = user_id
                             AND user_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    name                 TEXT NOT NULL,
    seeds                TEXT NOT NULL
                         CHECK (json_valid(seeds) AND json_type(seeds) = 'array'),
    scope                TEXT NOT NULL DEFAULT '{}'
                         CHECK (json_valid(scope) AND json_type(scope) = 'object'),
    max_depth            INTEGER NOT NULL DEFAULT 3 CHECK (max_depth >= 0),
    max_urls             INTEGER NOT NULL DEFAULT 10000 CHECK (max_urls > 0),
    render_mode          TEXT NOT NULL DEFAULT 'auto'
                         CHECK (render_mode IN ('auto', 'http', 'browser')),
    concurrency          INTEGER NOT NULL DEFAULT 4
                         CHECK (concurrency BETWEEN 1 AND 64),
    per_host_concurrency INTEGER NOT NULL DEFAULT 2
                         CHECK (per_host_concurrency BETWEEN 1 AND 16),
    delay_ms             INTEGER NOT NULL DEFAULT 500 CHECK (delay_ms >= 0),
    respect_robots       INTEGER NOT NULL DEFAULT 1
                         CHECK (respect_robots IN (0, 1)),
    user_agent           TEXT,
    sink                 TEXT NOT NULL DEFAULT '{}'
                         CHECK (json_valid(sink) AND json_type(sink) = 'object'),
    status               TEXT NOT NULL DEFAULT 'draft'
                         CHECK (status IN ('draft', 'running', 'paused', 'done', 'failed', 'cancelled')),
    error_detail         TEXT,
    started_at           INTEGER,
    finished_at          INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE INDEX idx_crawl_jobs_user_id ON crawl_jobs(user_id);
CREATE INDEX idx_crawl_jobs_status ON crawl_jobs(status);

CREATE TABLE crawl_tasks (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id          TEXT NOT NULL UNIQUE
                     CHECK (
                         length(task_id) = 36
                         AND lower(task_id) = task_id
                         AND task_id GLOB '????????-????-7???-[89ab]???-????????????'
                         AND replace(task_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                     ),
    job_id           TEXT NOT NULL
                     CHECK (
                         length(job_id) = 36
                         AND lower(job_id) = job_id
                         AND job_id GLOB '????????-????-7???-[89ab]???-????????????'
                         AND replace(job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                     ),
    parent_task_id   TEXT
                     CHECK (
                         parent_task_id IS NULL
                         OR (
                             length(parent_task_id) = 36
                             AND lower(parent_task_id) = parent_task_id
                             AND parent_task_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(parent_task_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         )
                     ),
    url              TEXT NOT NULL,
    -- sha256 of the normalized URL; the dedup invariant is physical, not advisory.
    url_fingerprint  TEXT NOT NULL
                     CHECK (
                         length(url_fingerprint) = 64
                         AND lower(url_fingerprint) = url_fingerprint
                         AND url_fingerprint NOT GLOB '*[^0-9a-f]*'
                     ),
    -- Denormalized so the per-host concurrency gate can filter inside the claim SELECT.
    host             TEXT NOT NULL,
    depth            INTEGER NOT NULL DEFAULT 0 CHECK (depth >= 0),
    priority         INTEGER NOT NULL DEFAULT 0,
    status           TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending', 'in_progress', 'done', 'failed', 'skipped')),
    attempt_count    INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    claim_generation INTEGER NOT NULL DEFAULT 0 CHECK (claim_generation >= 0),
    claim_token      TEXT
                     CHECK (
                         claim_token IS NULL
                         OR (
                             length(claim_token) = 64
                             AND lower(claim_token) = claim_token
                             AND claim_token NOT GLOB '*[^0-9a-f]*'
                         )
                     ),
    owner_node_id    TEXT,
    claimed_at       INTEGER,
    lease_expires_at INTEGER,
    http_status      INTEGER,
    content_hash     TEXT
                     CHECK (
                         content_hash IS NULL
                         OR (
                             length(content_hash) = 64
                             AND lower(content_hash) = content_hash
                             AND content_hash NOT GLOB '*[^0-9a-f]*'
                         )
                     ),
    etag             TEXT,
    last_modified    TEXT,
    error_code       TEXT,
    error_detail     TEXT,
    completed_at     INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    UNIQUE (job_id, url_fingerprint)
);

CREATE INDEX idx_crawl_tasks_job_id ON crawl_tasks(job_id);
CREATE INDEX idx_crawl_tasks_parent_task_id ON crawl_tasks(parent_task_id);
-- Drives the claim SELECT: narrow to one job's pending rows, then order by policy.
CREATE INDEX idx_crawl_tasks_claim_order
    ON crawl_tasks(job_id, status, priority DESC, depth ASC, created_at ASC);
-- Drives lease reaping across all jobs.
CREATE INDEX idx_crawl_tasks_lease
    ON crawl_tasks(status, lease_expires_at)
    WHERE status = 'in_progress';
CREATE INDEX idx_crawl_tasks_host ON crawl_tasks(job_id, host, status);

-- `done` and `skipped` are absorbing. `failed` stays reopenable so a human can
-- replay a poisoned URL after fixing scope or credentials.
CREATE TRIGGER trg_crawl_tasks_absorb_terminal
BEFORE UPDATE OF status ON crawl_tasks
FOR EACH ROW
WHEN OLD.status IN ('done', 'skipped')
 AND NEW.status IS NOT OLD.status
BEGIN
    SELECT RAISE(ABORT, 'completed or skipped crawl task status is immutable');
END;

-- An in-progress row is executable authority; it can only be minted by the
-- atomic pending->in_progress claim, never inserted directly.
CREATE TRIGGER trg_crawl_tasks_in_progress_insert_guard
BEFORE INSERT ON crawl_tasks
FOR EACH ROW
WHEN NEW.status = 'in_progress'
BEGIN
    SELECT RAISE(ABORT, 'in-progress crawl task may only be entered by atomically claiming a pending row');
END;

CREATE TRIGGER trg_crawl_tasks_in_progress_update_guard
BEFORE UPDATE ON crawl_tasks
FOR EACH ROW
WHEN NEW.status = 'in_progress'
 AND (
        NEW.claim_generation IS NULL
        OR NEW.claim_generation <= 0
        OR NEW.claim_token IS NULL
        OR NEW.owner_node_id IS NULL
        OR NEW.claimed_at IS NULL
        OR NEW.lease_expires_at IS NULL
        OR NEW.lease_expires_at <= NEW.claimed_at
        OR (
            OLD.status = 'in_progress'
            AND (
                NEW.claim_generation IS NOT OLD.claim_generation
                OR NEW.claim_token IS NOT OLD.claim_token
                OR NEW.owner_node_id IS NOT OLD.owner_node_id
                OR NEW.claimed_at IS NOT OLD.claimed_at
                OR NEW.attempt_count IS NOT OLD.attempt_count
            )
        )
        OR (
            OLD.status IN ('pending', 'failed')
            AND (
                OLD.claim_token IS NOT NULL
                OR NEW.claim_generation IS NOT OLD.claim_generation + 1
                OR NEW.attempt_count IS NOT OLD.attempt_count + 1
            )
        )
        OR OLD.status IS NULL
        OR OLD.status NOT IN ('pending', 'in_progress', 'failed')
 )
BEGIN
    SELECT RAISE(ABORT, 'in-progress crawl task requires a fresh generation, capability, owner, and lease');
END;

-- Pending is non-authority. A requeue must erase the prior capability before
-- the row becomes selectable again, otherwise a zombie worker could still
-- match on the stale token.
CREATE TRIGGER trg_crawl_tasks_pending_insert_guard
BEFORE INSERT ON crawl_tasks
FOR EACH ROW
WHEN NEW.status = 'pending'
 AND (
        NEW.claim_token IS NOT NULL
        OR NEW.owner_node_id IS NOT NULL
        OR NEW.claimed_at IS NOT NULL
        OR NEW.lease_expires_at IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'pending crawl task cannot carry execution authority');
END;

CREATE TRIGGER trg_crawl_tasks_pending_update_guard
BEFORE UPDATE ON crawl_tasks
FOR EACH ROW
WHEN NEW.status = 'pending'
 AND (
        NEW.claim_token IS NOT NULL
        OR NEW.owner_node_id IS NOT NULL
        OR NEW.claimed_at IS NOT NULL
        OR NEW.lease_expires_at IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'pending crawl task cannot carry execution authority');
END;

-- Settling a task must also surrender the capability, so a late duplicate
-- submission from the same generation cannot match.
CREATE TRIGGER trg_crawl_tasks_settled_release_guard
BEFORE UPDATE ON crawl_tasks
FOR EACH ROW
WHEN NEW.status IN ('done', 'failed', 'skipped')
 AND (
        NEW.claim_token IS NOT NULL
        OR NEW.owner_node_id IS NOT NULL
        OR NEW.lease_expires_at IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'settled crawl task must release its claim capability');
END;
