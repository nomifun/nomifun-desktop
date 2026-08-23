-- Creative Studio Phase 1: standalone workbenches are installation-owned.
--
-- Migrations 037-046 are published and must remain byte-for-byte unchanged.
-- Rebuild creation_tasks once so a standalone owner may omit project_id while
-- preserving historical project_id values as inert provenance on old rows.
-- request_fingerprint is copied verbatim; no JSON rewriting is allowed here.

DROP TRIGGER IF EXISTS restrict_workshop_asset_delete_creation_task_refs;

CREATE TABLE creation_tasks_v5 (
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
    workbench_kind TEXT
        CHECK (workbench_kind IS NULL OR workbench_kind IN ('image', 'video', 'audio')),
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
    input_bindings TEXT
        CHECK (
            input_bindings IS NULL
            OR (json_valid(input_bindings) AND json_type(input_bindings) = 'array')
        ),
    status TEXT NOT NULL,
    error TEXT,
    result_asset_ids TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(result_asset_ids) AND json_type(result_asset_ids) = 'array'),
    remote_task_id TEXT,
    attempt INTEGER NOT NULL DEFAULT 0,
    submitted_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    deleted_at INTEGER
        CHECK (
            deleted_at IS NULL
            OR (
                deleted_at >= 0
                AND deleted_at >= submitted_at
                AND workbench_kind IS NOT NULL
                AND node_id IS NULL
                AND workflow_id IS NULL
                AND workflow_run_id IS NULL
                AND workflow_step_id IS NULL
                AND status IN ('failed', 'canceled', 'succeeded')
            )
        ),
    request_fingerprint TEXT NOT NULL
        CHECK (
            json_valid(request_fingerprint)
            AND json_type(request_fingerprint) = 'object'
        ),
    CHECK (
        -- Canvas node owner. `project_id` is the published storage name for
        -- the canvas business ID until the Canvas facade lands.
        (
            project_id IS NOT NULL
            AND node_id IS NOT NULL
            AND workbench_kind IS NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
        )
        OR
        -- Standalone workbench owner. project_id is optional so new rows do
        -- not create a hidden Canvas; old values remain inert provenance.
        (
            workbench_kind IS NOT NULL
            AND node_id IS NULL
            AND workflow_id IS NULL
            AND workflow_run_id IS NULL
            AND workflow_step_id IS NULL
        )
        OR
        -- Workflow step owner.
        (
            project_id IS NULL
            AND workbench_kind IS NULL
            AND node_id IS NULL
            AND workflow_id IS NOT NULL
            AND workflow_run_id IS NOT NULL
            AND workflow_step_id IS NOT NULL
        )
    )
);

INSERT INTO creation_tasks_v5 (
    id, creation_task_id, project_id, workbench_kind, workflow_id,
    workflow_run_id, workflow_step_id, node_id, provider_id, model,
    capability, params, input_bindings, status, error, result_asset_ids,
    remote_task_id, attempt, submitted_at, started_at, finished_at,
    deleted_at, request_fingerprint
)
SELECT
    id, creation_task_id, project_id, workbench_kind, workflow_id,
    workflow_run_id, workflow_step_id, node_id, provider_id, model,
    capability, params, input_bindings, status, error, result_asset_ids,
    remote_task_id, attempt, submitted_at, started_at, finished_at,
    deleted_at, request_fingerprint
FROM creation_tasks;

DROP TABLE creation_tasks;
ALTER TABLE creation_tasks_v5 RENAME TO creation_tasks;

CREATE INDEX idx_creation_tasks_project_id ON creation_tasks(project_id);
CREATE INDEX idx_creation_tasks_workbench_owner_deleted_page
    ON creation_tasks(
        workbench_kind,
        deleted_at,
        submitted_at DESC,
        creation_task_id DESC
    )
    WHERE workbench_kind IS NOT NULL;
CREATE INDEX idx_creation_tasks_workflow_id ON creation_tasks(workflow_id);
CREATE INDEX idx_creation_tasks_workflow_run_id ON creation_tasks(workflow_run_id);
CREATE INDEX idx_creation_tasks_provider_id ON creation_tasks(provider_id);
CREATE INDEX idx_creation_tasks_input_bindings_json ON creation_tasks(input_bindings);
CREATE INDEX idx_creation_tasks_result_asset_ids_json ON creation_tasks(result_asset_ids);
CREATE INDEX idx_creation_tasks_status ON creation_tasks(status);
CREATE INDEX idx_workshop_assets_origin_canvas_id
    ON workshop_assets(json_extract(origin, '$.canvas_id'))
    WHERE origin IS NOT NULL;

CREATE TRIGGER validate_creation_task_input_bindings_insert
BEFORE INSERT ON creation_tasks
WHEN NEW.input_bindings IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid creation task input binding')
    WHERE EXISTS (
        SELECT 1
        FROM json_each(NEW.input_bindings) AS input
        WHERE json_type(input.value) IS NOT 'object'
           OR json_type(input.value, '$.asset_id') IS NOT 'text'
           OR length(json_extract(input.value, '$.asset_id')) <> 36
           OR lower(json_extract(input.value, '$.asset_id')) <> json_extract(input.value, '$.asset_id')
           OR json_extract(input.value, '$.asset_id') NOT GLOB '????????-????-7???-[89ab]???-????????????'
           OR replace(json_extract(input.value, '$.asset_id'), '-', '') GLOB '*[^0-9a-f]*'
           OR json_type(input.value, '$.kind') IS NOT 'text'
           OR json_extract(input.value, '$.kind') NOT IN ('image', 'video', 'audio', 'text')
           OR json_type(input.value, '$.role') IS NOT 'text'
           OR json_extract(input.value, '$.role') NOT IN (
                'reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'
           )
           OR (SELECT COUNT(*) FROM json_each(input.value)) <> 3
           OR EXISTS (
                SELECT 1 FROM json_each(input.value) AS field
                WHERE field.key NOT IN ('asset_id', 'kind', 'role')
           )
    );
END;

CREATE TRIGGER validate_creation_task_input_bindings_update
BEFORE UPDATE OF input_bindings ON creation_tasks
WHEN NEW.input_bindings IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid creation task input binding')
    WHERE EXISTS (
        SELECT 1
        FROM json_each(NEW.input_bindings) AS input
        WHERE json_type(input.value) IS NOT 'object'
           OR json_type(input.value, '$.asset_id') IS NOT 'text'
           OR length(json_extract(input.value, '$.asset_id')) <> 36
           OR lower(json_extract(input.value, '$.asset_id')) <> json_extract(input.value, '$.asset_id')
           OR json_extract(input.value, '$.asset_id') NOT GLOB '????????-????-7???-[89ab]???-????????????'
           OR replace(json_extract(input.value, '$.asset_id'), '-', '') GLOB '*[^0-9a-f]*'
           OR json_type(input.value, '$.kind') IS NOT 'text'
           OR json_extract(input.value, '$.kind') NOT IN ('image', 'video', 'audio', 'text')
           OR json_type(input.value, '$.role') IS NOT 'text'
           OR json_extract(input.value, '$.role') NOT IN (
                'reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'
           )
           OR (SELECT COUNT(*) FROM json_each(input.value)) <> 3
           OR EXISTS (
                SELECT 1 FROM json_each(input.value) AS field
                WHERE field.key NOT IN ('asset_id', 'kind', 'role')
           )
    );
END;

-- Standalone asset origins are installation-owned and contain only
-- workbench_kind (plus descriptive/task metadata). Historical standalone
-- origins may still carry project_id and remain readable; the owner branch
-- below intentionally treats that field as inert provenance.
DROP TRIGGER IF EXISTS validate_creative_asset_origin_insert;
DROP TRIGGER IF EXISTS validate_creative_asset_origin_update;

CREATE TRIGGER validate_creative_asset_origin_insert
BEFORE INSERT ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.task_id') IS NOT NULL
       OR json_type(NEW.origin, '$.providerId') IS NOT NULL
       OR json_type(NEW.origin, '$.canvasId') IS NOT NULL
       OR json_type(NEW.origin, '$.nodeId') IS NOT NULL
       OR json_type(NEW.origin, '$.creationTaskId') IS NOT NULL
       OR json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workbenchKind') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowStepId') IS NOT NULL;

    SELECT RAISE(ABORT, 'invalid creative asset origin Canvas identifier')
    WHERE json_type(NEW.origin, '$.canvas_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.canvas_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.canvas_id')) = 36
          AND lower(json_extract(NEW.origin, '$.canvas_id')) = json_extract(NEW.origin, '$.canvas_id')
          AND json_extract(NEW.origin, '$.canvas_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.canvas_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin legacy Canvas compatibility identifier')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.project_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.project_id')) = 36
          AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
          AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_id')
    WHERE json_type(NEW.origin, '$.workflow_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_id')) = json_extract(NEW.origin, '$.workflow_id')
          AND json_extract(NEW.origin, '$.workflow_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_run_id')
    WHERE json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_run_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_run_id')) = json_extract(NEW.origin, '$.workflow_run_id')
          AND json_extract(NEW.origin, '$.workflow_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_step_id')
    WHERE json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_step_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_step_id')) = json_extract(NEW.origin, '$.workflow_step_id')
          AND json_extract(NEW.origin, '$.workflow_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );

    SELECT RAISE(ABORT, 'invalid creative asset origin workbench_kind')
    WHERE json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workbench_kind') IS 'text'
          AND json_extract(NEW.origin, '$.workbench_kind') IN ('image', 'video', 'audio')
      );

    SELECT RAISE(ABORT, 'invalid creative asset Canvas/standalone/workflow owner branch')
    WHERE (
        json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
    ) AND NOT (
        (
            json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workbench_kind') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workflow_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_run_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_step_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
        )
    );
END;

CREATE TRIGGER validate_creative_asset_origin_update
BEFORE UPDATE OF origin ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.task_id') IS NOT NULL
       OR json_type(NEW.origin, '$.providerId') IS NOT NULL
       OR json_type(NEW.origin, '$.canvasId') IS NOT NULL
       OR json_type(NEW.origin, '$.nodeId') IS NOT NULL
       OR json_type(NEW.origin, '$.creationTaskId') IS NOT NULL
       OR json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workbenchKind') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.workflowStepId') IS NOT NULL;

    SELECT RAISE(ABORT, 'invalid creative asset origin Canvas identifier')
    WHERE json_type(NEW.origin, '$.canvas_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.canvas_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.canvas_id')) = 36
          AND lower(json_extract(NEW.origin, '$.canvas_id')) = json_extract(NEW.origin, '$.canvas_id')
          AND json_extract(NEW.origin, '$.canvas_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.canvas_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin legacy Canvas compatibility identifier')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.project_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.project_id')) = 36
          AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
          AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_id')
    WHERE json_type(NEW.origin, '$.workflow_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_id')) = json_extract(NEW.origin, '$.workflow_id')
          AND json_extract(NEW.origin, '$.workflow_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_run_id')
    WHERE json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_run_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_run_id')) = json_extract(NEW.origin, '$.workflow_run_id')
          AND json_extract(NEW.origin, '$.workflow_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin workflow_step_id')
    WHERE json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workflow_step_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.workflow_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.workflow_step_id')) = json_extract(NEW.origin, '$.workflow_step_id')
          AND json_extract(NEW.origin, '$.workflow_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.workflow_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );

    SELECT RAISE(ABORT, 'invalid creative asset origin workbench_kind')
    WHERE json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workbench_kind') IS 'text'
          AND json_extract(NEW.origin, '$.workbench_kind') IN ('image', 'video', 'audio')
      );

    SELECT RAISE(ABORT, 'invalid creative asset Canvas/standalone/workflow owner branch')
    WHERE (
        json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workflow_step_id') IS NOT NULL
    ) AND NOT (
        (
            json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workbench_kind') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_run_id') IS NULL
            AND json_type(NEW.origin, '$.workflow_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workflow_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_run_id') IS 'text'
            AND json_type(NEW.origin, '$.workflow_step_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
        )
    );
END;

CREATE TRIGGER restrict_workshop_asset_delete_creation_task_refs
BEFORE DELETE ON workshop_assets
WHEN EXISTS (
    SELECT 1
    FROM creation_tasks task
    WHERE EXISTS (
        SELECT 1 FROM json_each(task.input_bindings) input
        WHERE json_extract(input.value, '$.asset_id') = OLD.asset_id
    ) OR EXISTS (
        SELECT 1 FROM json_each(task.result_asset_ids) result
        WHERE result.value = OLD.asset_id
    )
)
BEGIN
    SELECT RAISE(ABORT, 'workshop asset is referenced by creation task input or result');
END;
