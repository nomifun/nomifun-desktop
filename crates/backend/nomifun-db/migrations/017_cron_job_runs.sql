CREATE TABLE IF NOT EXISTS cron_job_runs (
    id             TEXT    PRIMARY KEY NOT NULL,
    job_id         TEXT    NOT NULL,
    executed_at_ms INTEGER NOT NULL,
    status         TEXT    NOT NULL CHECK(status IN ('ok', 'error', 'skipped', 'missed')),
    created_at_ms  INTEGER NOT NULL,
    FOREIGN KEY (job_id) REFERENCES cron_jobs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_cron_job_runs_job_time
    ON cron_job_runs(job_id, executed_at_ms DESC, created_at_ms DESC);
