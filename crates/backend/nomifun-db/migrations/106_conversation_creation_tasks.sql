-- Conversation-owned generation uses the existing task engine.
-- Preserve task identities, remote handles, payloads and all asset constraints.

DROP TRIGGER IF EXISTS validate_creation_task_input_bindings_insert;

DROP TRIGGER IF EXISTS validate_creation_task_input_bindings_update;

DROP TRIGGER IF EXISTS restrict_workshop_asset_delete_creation_task_refs;

DROP TRIGGER IF EXISTS restrict_creation_task_deleted_assets_insert;

DROP TRIGGER IF EXISTS restrict_creation_task_deleted_assets_update;

CREATE TABLE creation_tasks_conversation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    creation_task_id TEXT NOT NULL UNIQUE
        CHECK (
            length(creation_task_id) = 36
            AND lower(creation_task_id) = creation_task_id
            AND creation_task_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(creation_task_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    conversation_id TEXT CHECK (conversation_id IS NULL OR (length(conversation_id)=36 AND lower(conversation_id)=conversation_id AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_id,'-','') NOT GLOB '*[^0-9a-f]*')),
    message_id TEXT CHECK (message_id IS NULL OR (length(message_id)=36 AND lower(message_id)=message_id AND message_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(message_id,'-','') NOT GLOB '*[^0-9a-f]*')),
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
      (conversation_id IS NULL AND message_id IS NULL AND (
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
      )) OR (conversation_id IS NOT NULL AND message_id IS NOT NULL
        AND project_id IS NULL AND workbench_kind IS NULL AND node_id IS NULL
        AND template_id IS NULL AND template_run_id IS NULL AND template_step_id IS NULL)
    )
);

INSERT INTO creation_tasks_conversation (id, creation_task_id, project_id, workbench_kind, template_id, template_run_id, template_step_id, node_id, provider_id, model, capability, params, input_bindings, status, error, result_asset_ids, remote_task_id, attempt, submitted_at, started_at, finished_at, deleted_at, request_fingerprint) SELECT id, creation_task_id, project_id, workbench_kind, template_id, template_run_id, template_step_id, node_id, provider_id, model, capability, params, input_bindings, status, error, result_asset_ids, remote_task_id, attempt, submitted_at, started_at, finished_at, deleted_at, request_fingerprint FROM creation_tasks;

DROP TABLE creation_tasks;

ALTER TABLE creation_tasks_conversation RENAME TO creation_tasks;

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

CREATE TRIGGER restrict_creation_task_deleted_assets_insert
BEFORE INSERT ON creation_tasks
BEGIN
    SELECT RAISE(ABORT, 'creation task references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        WHERE asset.deleted_at IS NOT NULL AND (
            EXISTS (SELECT 1 FROM json_each(NEW.input_bindings) input
                    WHERE json_extract(input.value, '$.asset_id') = asset.asset_id)
            OR EXISTS (SELECT 1 FROM json_each(NEW.result_asset_ids) result
                       WHERE result.value = asset.asset_id)
        )
    );
END;

CREATE TRIGGER restrict_creation_task_deleted_assets_update
BEFORE UPDATE OF input_bindings, result_asset_ids, status ON creation_tasks
BEGIN
    SELECT RAISE(ABORT, 'creation task references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        WHERE asset.deleted_at IS NOT NULL AND (
            EXISTS (
                SELECT 1 FROM json_each(NEW.input_bindings) input
                WHERE json_extract(input.value, '$.asset_id') = asset.asset_id
                  AND (NEW.status IN ('queued', 'running') OR NOT EXISTS (
                      SELECT 1 FROM json_each(OLD.input_bindings) old_input
                      WHERE json_extract(old_input.value, '$.asset_id') = asset.asset_id
                  ))
            )
            OR EXISTS (
                SELECT 1 FROM json_each(NEW.result_asset_ids) result
                WHERE result.value = asset.asset_id
                  AND (NEW.status IN ('queued', 'running') OR NOT EXISTS (
                      SELECT 1 FROM json_each(OLD.result_asset_ids) old_result
                      WHERE old_result.value = asset.asset_id
                  ))
            )
        )
    );
END;

CREATE INDEX idx_creation_tasks_conversation ON creation_tasks(conversation_id, submitted_at, creation_task_id) WHERE conversation_id IS NOT NULL;
