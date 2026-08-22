-- Creative Studio is now the only task-producing product surface. Remove the
-- retired Workshop canvas/global owner branch instead of carrying nullable
-- compatibility columns forward.

CREATE TABLE creation_tasks_v3 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    creation_task_id TEXT NOT NULL UNIQUE
        CHECK (
            length(creation_task_id) = 36
            AND lower(creation_task_id) = creation_task_id
            AND creation_task_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(creation_task_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    project_id TEXT
        CHECK (
            project_id IS NULL
            OR (
                length(project_id) = 36
                AND lower(project_id) = project_id
                AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    workflow_id TEXT
        CHECK (
            workflow_id IS NULL
            OR (
                length(workflow_id) = 36
                AND lower(workflow_id) = workflow_id
                AND workflow_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(workflow_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    workflow_run_id TEXT
        CHECK (
            workflow_run_id IS NULL
            OR (
                length(workflow_run_id) = 36
                AND lower(workflow_run_id) = workflow_run_id
                AND workflow_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(workflow_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    workflow_step_id TEXT
        CHECK (
            workflow_step_id IS NULL
            OR (
                length(workflow_step_id) = 36
                AND lower(workflow_step_id) = workflow_step_id
                AND workflow_step_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(workflow_step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    node_id TEXT
        CHECK (
            node_id IS NULL
            OR (
                length(node_id) = 36
                AND lower(node_id) = node_id
                AND node_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(node_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    provider_id TEXT NOT NULL
        CHECK (
            length(provider_id) = 36
            AND lower(provider_id) = provider_id
            AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    model TEXT NOT NULL,
    capability TEXT NOT NULL,
    params TEXT NOT NULL,
    status TEXT NOT NULL,
    error TEXT,
    result_asset_ids TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(result_asset_ids) AND json_type(result_asset_ids) = 'array'),
    remote_task_id TEXT,
    attempt INTEGER NOT NULL DEFAULT 0,
    submitted_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    request_fingerprint TEXT NOT NULL
        CHECK (
            json_valid(request_fingerprint)
            AND json_type(request_fingerprint) = 'object'
        ),
    CHECK (
        -- Canonical project-node owner.
        (
            project_id IS NOT NULL
            AND node_id IS NOT NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
        )
        OR
        -- Canonical workflow-step owner.
        (
            project_id IS NULL
            AND node_id IS NULL
            AND workflow_id IS NOT NULL
            AND workflow_run_id IS NOT NULL
            AND workflow_step_id IS NOT NULL
        )
    )
);

-- Historical unowned/Workshop rows are intentionally retired. Exact retries
-- for canonical Creative Studio tasks retain their original business IDs,
-- state, remote handles, and request fingerprints.
INSERT INTO creation_tasks_v3 (
    id, creation_task_id, project_id, workflow_id, workflow_run_id,
    workflow_step_id, node_id, provider_id, model, capability, params, status,
    error, result_asset_ids, remote_task_id, attempt, submitted_at, started_at,
    finished_at, request_fingerprint
)
SELECT
    id, creation_task_id, project_id, workflow_id, workflow_run_id,
    workflow_step_id, node_id, provider_id, model, capability, params, status,
    error, result_asset_ids, remote_task_id, attempt, submitted_at, started_at,
    finished_at, request_fingerprint
FROM creation_tasks
WHERE request_fingerprint IS NOT NULL
  AND (
      (
          project_id IS NOT NULL
          AND node_id IS NOT NULL
          AND workflow_id IS NULL
          AND workflow_run_id IS NULL
          AND workflow_step_id IS NULL
      )
      OR
      (
          project_id IS NULL
          AND node_id IS NULL
          AND workflow_id IS NOT NULL
          AND workflow_run_id IS NOT NULL
          AND workflow_step_id IS NOT NULL
      )
  );

DROP TABLE creation_tasks;
ALTER TABLE creation_tasks_v3 RENAME TO creation_tasks;

CREATE INDEX idx_creation_tasks_project_id ON creation_tasks(project_id);
CREATE INDEX idx_creation_tasks_workflow_id ON creation_tasks(workflow_id);
CREATE INDEX idx_creation_tasks_workflow_run_id ON creation_tasks(workflow_run_id);
CREATE INDEX idx_creation_tasks_provider_id ON creation_tasks(provider_id);
CREATE INDEX idx_creation_tasks_result_asset_ids_json ON creation_tasks(result_asset_ids);
CREATE INDEX idx_creation_tasks_status ON creation_tasks(status);
