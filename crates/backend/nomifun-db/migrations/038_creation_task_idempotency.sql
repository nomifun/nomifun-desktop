-- Canonical Creative Studio task ownership and durable request idempotency.
--
-- Legacy Workshop tasks keep `canvas_id` and no request fingerprint. New
-- Creative Studio tasks use `project_id`, never `canvas_id`, and use their
-- canonical UUIDv7 Idempotency-Key as `creation_task_id`.

ALTER TABLE creation_tasks ADD COLUMN project_id TEXT
    CHECK (
        -- Canonical ownership never aliases the retired Workshop canvas.
        (project_id IS NULL OR canvas_id IS NULL)
        AND (
            project_id IS NULL
            OR (
                length(project_id) = 36
                AND lower(project_id) = project_id
                AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        )
    );

-- This is the canonical serialized request, not a lossy or collision-prone
-- process-local hash. Exact equality therefore proves that a repeated key is
-- the same logical submission.
ALTER TABLE creation_tasks ADD COLUMN request_fingerprint TEXT
    CHECK (
        -- A fingerprint marks the canonical path. It is inseparable from
        -- project + node ownership; legacy/global tasks keep all three null.
        (
            project_id IS NULL
            AND request_fingerprint IS NULL
        )
        OR (
            project_id IS NOT NULL
            AND canvas_id IS NULL
            AND node_id IS NOT NULL
            AND request_fingerprint IS NOT NULL
            AND json_valid(request_fingerprint)
            AND json_type(request_fingerprint) = 'object'
        )
    );

CREATE INDEX idx_creation_tasks_project_id ON creation_tasks(project_id);
