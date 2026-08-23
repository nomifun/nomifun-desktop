-- Creative Studio template vocabulary migration.
--
-- Migrations 040-047 are published and immutable. This forward migration
-- rebuilds the current Creative Studio objects so the live schema contains
-- only template names and the persisted JSON contracts use the same vocabulary.

DROP TRIGGER IF EXISTS validate_creative_asset_origin_insert;
DROP TRIGGER IF EXISTS validate_creative_asset_origin_update;
DROP TRIGGER IF EXISTS validate_creation_task_input_bindings_insert;
DROP TRIGGER IF EXISTS validate_creation_task_input_bindings_update;
DROP TRIGGER IF EXISTS restrict_workshop_asset_delete_creation_task_refs;
DROP INDEX IF EXISTS idx_workshop_assets_origin_workflow_id;
DROP INDEX IF EXISTS idx_workshop_assets_origin_workflow_run_id;
DROP INDEX IF EXISTS idx_workshop_assets_origin_workflow_step_id;
DROP INDEX IF EXISTS idx_creation_tasks_workflow_id;
DROP INDEX IF EXISTS idx_creation_tasks_workflow_run_id;
DROP INDEX IF EXISTS idx_workshop_assets_origin_canvas_id;

-- Rewrite persisted asset provenance before the new validation triggers are
-- installed. Complete template-step owners retain every identifier; malformed
-- partial owners lose only their invalid ownership fragment.
UPDATE workshop_assets
SET origin = json_remove(
    json_set(
        origin,
        '$.template_id', json_extract(origin, '$.workflow_id'),
        '$.template_run_id', json_extract(origin, '$.workflow_run_id'),
        '$.template_step_id', json_extract(origin, '$.workflow_step_id')
    ),
    '$.workflow_id',
    '$.workflow_run_id',
    '$.workflow_step_id',
    '$.workflowId',
    '$.workflowRunId',
    '$.workflowStepId'
)
WHERE json_type(origin, '$.workflow_id') = 'text'
  AND json_type(origin, '$.workflow_run_id') = 'text'
  AND json_type(origin, '$.workflow_step_id') = 'text';

UPDATE workshop_assets
SET origin = NULLIF(
    json_remove(
        origin,
        '$.workflow_id',
        '$.workflow_run_id',
        '$.workflow_step_id',
        '$.workflowId',
        '$.workflowRunId',
        '$.workflowStepId'
    ),
    '{}'
)
WHERE origin IS NOT NULL
  AND (
      json_type(origin, '$.workflow_id') IS NOT NULL
      OR json_type(origin, '$.workflow_run_id') IS NOT NULL
      OR json_type(origin, '$.workflow_step_id') IS NOT NULL
      OR json_type(origin, '$.workflowId') IS NOT NULL
      OR json_type(origin, '$.workflowRunId') IS NOT NULL
      OR json_type(origin, '$.workflowStepId') IS NOT NULL
  )
  AND NOT (
      json_type(origin, '$.template_id') = 'text'
      AND json_type(origin, '$.template_run_id') = 'text'
      AND json_type(origin, '$.template_step_id') = 'text'
  );

CREATE TABLE creative_studio_templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id TEXT NOT NULL UNIQUE
        CHECK (
            length(template_id) = 36
            AND lower(template_id) = template_id
            AND template_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(template_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    description TEXT NOT NULL CHECK (length(description) <= 2000),
    category TEXT NOT NULL CHECK (length(category) <= 80),
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'public')),
    definition_json TEXT NOT NULL
        CHECK (json_valid(definition_json))
        CHECK (
            json_type(definition_json, '$.id') = 'text'
            AND json_extract(definition_json, '$.id') = template_id
        )
        CHECK (
            json_type(definition_json, '$.revision') = 'integer'
            AND json_extract(definition_json, '$.revision') = revision
        ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
);

INSERT INTO creative_studio_templates (
    id, template_id, revision, name, description, category, visibility,
    definition_json, created_at, updated_at
)
SELECT
    id, workflow_id, revision, name, description, category, visibility,
    definition_json, created_at, updated_at
FROM creative_studio_workflows;

CREATE INDEX idx_creative_studio_templates_updated
    ON creative_studio_templates(updated_at DESC, id DESC);
CREATE INDEX idx_creative_studio_templates_category
    ON creative_studio_templates(category, updated_at DESC, id DESC);

CREATE TABLE creative_studio_template_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_run_id TEXT NOT NULL UNIQUE
        CHECK (
            length(template_run_id) = 36
            AND lower(template_run_id) = template_run_id
            AND template_run_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(template_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    template_id TEXT NOT NULL
        CHECK (
            length(template_id) = 36
            AND lower(template_id) = template_id
            AND template_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(template_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    template_revision INTEGER NOT NULL CHECK (template_revision >= 1),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    status TEXT NOT NULL
        CHECK (status IN (
            'requested', 'awaiting-review', 'queued', 'running',
            'succeeded', 'failed', 'cancelled'
        )),
    step_ids_json TEXT NOT NULL
        CHECK (
            json_valid(step_ids_json)
            AND json_type(step_ids_json) = 'array'
            AND json_array_length(step_ids_json) BETWEEN 1 AND 128
        ),
    aggregate_json TEXT NOT NULL
        CHECK (json_valid(aggregate_json) AND json_type(aggregate_json) = 'object')
        CHECK (json_extract(aggregate_json, '$.kind') = 'nomifun.creative-studio.template-run')
        CHECK (json_extract(aggregate_json, '$.version') = 1)
        CHECK (json_extract(aggregate_json, '$.revision') = revision)
        CHECK (json_extract(aggregate_json, '$.templateSnapshot.id') = template_id)
        CHECK (json_extract(aggregate_json, '$.templateSnapshot.revision') = template_revision)
        CHECK (json_extract(aggregate_json, '$.request.id') = template_run_id)
        CHECK (json_extract(aggregate_json, '$.request.templateId') = template_id)
        CHECK (json_extract(aggregate_json, '$.request.templateRevision') = template_revision)
        CHECK (json_extract(aggregate_json, '$.record.requestId') = template_run_id)
        CHECK (json_extract(aggregate_json, '$.record.templateId') = template_id)
        CHECK (json_extract(aggregate_json, '$.record.status') = status),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
);

INSERT INTO creative_studio_template_runs (
    id, template_run_id, template_id, template_revision, revision, status,
    step_ids_json, aggregate_json, created_at, updated_at
)
SELECT
    old.id,
    old.workflow_run_id,
    old.workflow_id,
    old.workflow_revision,
    old.revision,
    old.status,
    old.step_ids_json,
    json_set(
        json_remove(
            json_remove(
                json_remove(
                    json_remove(
                        old.aggregate_json,
                        '$.workflowSnapshot'
                    ),
                    '$.request.workflowId',
                    '$.request.workflowRevision',
                    '$.record.workflowId'
                ),
                '$.kind'
            ),
            '$.promptDrafts'
        ),
        '$.kind', 'nomifun.creative-studio.template-run',
        '$.templateSnapshot', json(json_extract(old.aggregate_json, '$.workflowSnapshot')),
        '$.request.templateId', json_extract(old.aggregate_json, '$.request.workflowId'),
        '$.request.templateRevision', json_extract(old.aggregate_json, '$.request.workflowRevision'),
        '$.record.templateId', json_extract(old.aggregate_json, '$.record.workflowId'),
        '$.promptDrafts', COALESCE(
            (
                SELECT json_group_array(json(json_set(
                    json_remove(draft.value, '$.workflowId'),
                    '$.templateId', json_extract(draft.value, '$.workflowId')
                )))
                FROM json_each(old.aggregate_json, '$.promptDrafts') AS draft
            ),
            '[]'
        )
    ),
    old.created_at,
    old.updated_at
FROM creative_studio_workflow_runs AS old;

CREATE INDEX idx_creative_template_runs_template_id
    ON creative_studio_template_runs(template_id, updated_at DESC, id DESC);
CREATE INDEX idx_creative_template_runs_status
    ON creative_studio_template_runs(status, updated_at DESC, id DESC);

DROP TABLE creative_studio_workflow_runs;
DROP TABLE creative_studio_workflows;

DROP TRIGGER IF EXISTS restrict_workshop_asset_delete_creation_task_refs;

CREATE TABLE creation_tasks_v6 (
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
    template_id TEXT
        CHECK (
            template_id IS NULL
            OR (
                length(template_id) = 36
                AND lower(template_id) = template_id
                AND template_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(template_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    template_run_id TEXT
        CHECK (
            template_run_id IS NULL
            OR (
                length(template_run_id) = 36
                AND lower(template_run_id) = template_run_id
                AND template_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(template_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    template_step_id TEXT
        CHECK (
            template_step_id IS NULL
            OR (
                length(template_step_id) = 36
                AND lower(template_step_id) = template_step_id
                AND template_step_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(template_step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
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
                AND template_id IS NULL
                AND template_run_id IS NULL
                AND template_step_id IS NULL
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
            AND template_id IS NULL
            AND template_run_id IS NULL
            AND template_step_id IS NULL
        )
        OR
        -- Standalone workbench owner. project_id is optional so new rows do
        -- not create a hidden Canvas; old values remain inert provenance.
        (
            workbench_kind IS NOT NULL
            AND node_id IS NULL
            AND template_id IS NULL
            AND template_run_id IS NULL
            AND template_step_id IS NULL
        )
        OR
        -- Template step owner.
        (
            project_id IS NULL
            AND workbench_kind IS NULL
            AND node_id IS NULL
            AND template_id IS NOT NULL
            AND template_run_id IS NOT NULL
            AND template_step_id IS NOT NULL
        )
    )
);

INSERT INTO creation_tasks_v6 (
    id, creation_task_id, project_id, workbench_kind, template_id,
    template_run_id, template_step_id, node_id, provider_id, model,
    capability, params, input_bindings, status, error, result_asset_ids,
    remote_task_id, attempt, submitted_at, started_at, finished_at,
    deleted_at, request_fingerprint
)
SELECT
    id, creation_task_id, project_id, workbench_kind, workflow_id,
    workflow_run_id, workflow_step_id, node_id, provider_id, model,
    capability, params, input_bindings, status, error, result_asset_ids,
    remote_task_id, attempt, submitted_at, started_at, finished_at,
    deleted_at,
    CASE
        WHEN workflow_id IS NOT NULL
         AND workflow_run_id IS NOT NULL
         AND workflow_step_id IS NOT NULL
        THEN json_set(
            json_remove(
                request_fingerprint,
                '$.owner.workflow_id',
                '$.owner.workflow_run_id',
                '$.owner.workflow_step_id'
            ),
            '$.owner.kind', 'template_step',
            '$.owner.template_id', workflow_id,
            '$.owner.template_run_id', workflow_run_id,
            '$.owner.template_step_id', workflow_step_id
        )
        ELSE request_fingerprint
    END
FROM creation_tasks;

DROP TABLE creation_tasks;
ALTER TABLE creation_tasks_v6 RENAME TO creation_tasks;

CREATE INDEX idx_creation_tasks_project_id ON creation_tasks(project_id);
CREATE INDEX idx_creation_tasks_workbench_owner_deleted_page
    ON creation_tasks(
        workbench_kind,
        deleted_at,
        submitted_at DESC,
        creation_task_id DESC
    )
    WHERE workbench_kind IS NOT NULL;
CREATE INDEX idx_creation_tasks_template_id ON creation_tasks(template_id);
CREATE INDEX idx_creation_tasks_template_run_id ON creation_tasks(template_run_id);
CREATE INDEX idx_creation_tasks_provider_id ON creation_tasks(provider_id);
CREATE INDEX idx_creation_tasks_input_bindings_json ON creation_tasks(input_bindings);
CREATE INDEX idx_creation_tasks_result_asset_ids_json ON creation_tasks(result_asset_ids);
CREATE INDEX idx_creation_tasks_status ON creation_tasks(status);
CREATE INDEX idx_workshop_assets_origin_canvas_id
    ON workshop_assets(json_extract(origin, '$.canvas_id'))
    WHERE origin IS NOT NULL;
CREATE INDEX idx_workshop_assets_origin_template_id
    ON workshop_assets(json_extract(origin, '$.template_id'))
    WHERE origin IS NOT NULL;
CREATE INDEX idx_workshop_assets_origin_template_run_id
    ON workshop_assets(json_extract(origin, '$.template_run_id'))
    WHERE origin IS NOT NULL;
CREATE INDEX idx_workshop_assets_origin_template_step_id
    ON workshop_assets(json_extract(origin, '$.template_step_id'))
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
       OR json_type(NEW.origin, '$.templateId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateStepId') IS NOT NULL;

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
    SELECT RAISE(ABORT, 'invalid creative asset origin template_id')
    WHERE json_type(NEW.origin, '$.template_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.template_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.template_id')) = 36
          AND lower(json_extract(NEW.origin, '$.template_id')) = json_extract(NEW.origin, '$.template_id')
          AND json_extract(NEW.origin, '$.template_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.template_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin template_run_id')
    WHERE json_type(NEW.origin, '$.template_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.template_run_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.template_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.template_run_id')) = json_extract(NEW.origin, '$.template_run_id')
          AND json_extract(NEW.origin, '$.template_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.template_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin template_step_id')
    WHERE json_type(NEW.origin, '$.template_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.template_step_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.template_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.template_step_id')) = json_extract(NEW.origin, '$.template_step_id')
          AND json_extract(NEW.origin, '$.template_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.template_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );

    SELECT RAISE(ABORT, 'invalid creative asset origin workbench_kind')
    WHERE json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workbench_kind') IS 'text'
          AND json_extract(NEW.origin, '$.workbench_kind') IN ('image', 'video', 'audio')
      );

    SELECT RAISE(ABORT, 'invalid creative asset Canvas/standalone/template owner branch')
    WHERE (
        json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
        OR json_type(NEW.origin, '$.template_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_step_id') IS NOT NULL
    ) AND NOT (
        (
            json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workbench_kind') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.template_id') IS 'text'
            AND json_type(NEW.origin, '$.template_run_id') IS 'text'
            AND json_type(NEW.origin, '$.template_step_id') IS 'text'
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
       OR json_type(NEW.origin, '$.templateId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateStepId') IS NOT NULL;

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
    SELECT RAISE(ABORT, 'invalid creative asset origin template_id')
    WHERE json_type(NEW.origin, '$.template_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.template_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.template_id')) = 36
          AND lower(json_extract(NEW.origin, '$.template_id')) = json_extract(NEW.origin, '$.template_id')
          AND json_extract(NEW.origin, '$.template_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.template_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin template_run_id')
    WHERE json_type(NEW.origin, '$.template_run_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.template_run_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.template_run_id')) = 36
          AND lower(json_extract(NEW.origin, '$.template_run_id')) = json_extract(NEW.origin, '$.template_run_id')
          AND json_extract(NEW.origin, '$.template_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.template_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );
    SELECT RAISE(ABORT, 'invalid creative asset origin template_step_id')
    WHERE json_type(NEW.origin, '$.template_step_id') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.template_step_id') IS 'text'
          AND length(json_extract(NEW.origin, '$.template_step_id')) = 36
          AND lower(json_extract(NEW.origin, '$.template_step_id')) = json_extract(NEW.origin, '$.template_step_id')
          AND json_extract(NEW.origin, '$.template_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
          AND replace(json_extract(NEW.origin, '$.template_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
      );

    SELECT RAISE(ABORT, 'invalid creative asset origin workbench_kind')
    WHERE json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
      AND NOT (
          json_type(NEW.origin, '$.workbench_kind') IS 'text'
          AND json_extract(NEW.origin, '$.workbench_kind') IN ('image', 'video', 'audio')
      );

    SELECT RAISE(ABORT, 'invalid creative asset Canvas/standalone/template owner branch')
    WHERE (
        json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
        OR json_type(NEW.origin, '$.template_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_step_id') IS NOT NULL
    ) AND NOT (
        (
            json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.workbench_kind') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.workbench_kind') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL
        )
        OR (
            json_type(NEW.origin, '$.template_id') IS 'text'
            AND json_type(NEW.origin, '$.template_run_id') IS 'text'
            AND json_type(NEW.origin, '$.template_step_id') IS 'text'
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
