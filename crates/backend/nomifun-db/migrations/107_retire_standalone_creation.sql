-- One-way retirement of standalone image/video/audio workbench ownership.
-- Task IDs, provider handles, payloads, artifacts and retirement tombstones survive.
-- The installation singleton is the only authority for these formerly global tasks.
CREATE TEMP TABLE migration_creation_owner_guard (valid INTEGER NOT NULL CHECK (valid = 1));
INSERT INTO migration_creation_owner_guard(valid)
SELECT CASE WHEN NOT EXISTS (SELECT 1 FROM creation_tasks WHERE workbench_kind IS NOT NULL)
  OR EXISTS (SELECT 1 FROM installation_identity i JOIN users u ON u.user_id = i.owner_user_id WHERE i.singleton_key = 'installation')
  THEN 1 ELSE 0 END;

CREATE TEMP TABLE migration_creation_sessions (
  workbench_kind TEXT PRIMARY KEY, conversation_id TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
INSERT INTO migration_creation_sessions
SELECT workbench_kind, substr(MIN(creation_task_id), 1, 15) || substr(lower(hex(randomblob(2))), 2, 3) || '-8' || substr(lower(hex(randomblob(2))), 2, 3) || '-' || lower(hex(randomblob(6))), MIN(submitted_at),
  MAX(COALESCE(finished_at, started_at, submitted_at))
FROM creation_tasks WHERE workbench_kind IS NOT NULL GROUP BY workbench_kind;

CREATE TEMP TABLE migration_creation_messages (
  creation_task_id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, message_id TEXT NOT NULL UNIQUE
);
INSERT INTO migration_creation_messages
SELECT t.creation_task_id, s.conversation_id, substr(t.creation_task_id, 1, 15) || substr(lower(hex(randomblob(2))), 2, 3) || '-8' || substr(lower(hex(randomblob(2))), 2, 3) || '-' || lower(hex(randomblob(6)))
FROM creation_tasks t JOIN migration_creation_sessions s USING (workbench_kind);

INSERT INTO conversations(conversation_id, user_id, name, type, extra, status, source, created_at, updated_at)
SELECT s.conversation_id, i.owner_user_id,
  CASE s.workbench_kind WHEN 'image' THEN '图片生成历史' WHEN 'video' THEN '视频生成历史' ELSE '语音生成历史' END,
  'nomi', json_object('creation_history_import', s.workbench_kind), 'finished', 'nomifun', s.created_at, s.updated_at
FROM migration_creation_sessions s CROSS JOIN installation_identity i WHERE i.singleton_key = 'installation';

-- The retired aggregate had no chat turns: restore only the actual task prompt,
-- never infer historic Agent identity or merge an unrelated existing session.
INSERT INTO messages(message_id, conversation_id, type, content, position, status, hidden, created_at)
SELECT m.message_id, m.conversation_id, 'text',
  json_object('content', COALESCE(json_extract(t.params, '$.prompt'), json_extract(t.params, '$.text'), ''),
    'creation', json_object('creation_task_id', t.creation_task_id,
      'imported_owner', json_object('workbench_kind', t.workbench_kind, 'project_id', t.project_id))),
  'right', 'finish', CASE WHEN t.deleted_at IS NULL THEN 0 ELSE 1 END, t.submitted_at
FROM creation_tasks t JOIN migration_creation_messages m USING (creation_task_id)
ORDER BY t.submitted_at, t.creation_task_id;

-- Origin is provenance, not ownership: other canvas/template references remain intact.
UPDATE workshop_assets SET origin = json_set(json_remove(origin, '$.workbench_kind', '$.canvas_id', '$.project_id'),
  '$.historical_origin', json(origin),
  '$.conversation_id', (SELECT conversation_id FROM migration_creation_messages m WHERE m.creation_task_id = json_extract(workshop_assets.origin, '$.creation_task_id')),
  '$.message_id', (SELECT message_id FROM migration_creation_messages m WHERE m.creation_task_id = json_extract(workshop_assets.origin, '$.creation_task_id')))
WHERE json_extract(origin, '$.creation_task_id') IN (SELECT creation_task_id FROM migration_creation_messages);

-- Preserve unsupported standalone provenance that has no surviving task as an
-- archival object. It must not remain a live owner branch or invent a chat turn.
UPDATE workshop_assets SET origin = json_set(
  json_remove(origin, '$.workbench_kind', '$.canvas_id', '$.project_id'),
  '$.historical_origin', json(origin))
WHERE json_type(origin, '$.workbench_kind') IS NOT NULL;

DROP TRIGGER IF EXISTS validate_creation_task_input_bindings_insert;

DROP TRIGGER IF EXISTS validate_creation_task_input_bindings_update;

DROP TRIGGER IF EXISTS restrict_workshop_asset_delete_creation_task_refs;

DROP TRIGGER IF EXISTS restrict_creation_task_deleted_assets_insert;

DROP TRIGGER IF EXISTS restrict_creation_task_deleted_assets_update;

CREATE TABLE creation_tasks_unified (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    creation_task_id TEXT NOT NULL UNIQUE
        CHECK (
            length(creation_task_id) = 36
            AND lower(creation_task_id) = creation_task_id
            AND creation_task_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(creation_task_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    conversation_id TEXT CHECK (conversation_id IS NULL OR (
        length(conversation_id) = 36 AND lower(conversation_id) = conversation_id
        AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    message_id TEXT CHECK (message_id IS NULL OR (
        length(message_id) = 36 AND lower(message_id) = message_id
        AND message_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(message_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
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
                AND conversation_id IS NOT NULL AND message_id IS NOT NULL
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
            AND template_id IS NULL
            AND template_run_id IS NULL
            AND template_step_id IS NULL
        )
        OR
        -- Template step owner.
        (
            project_id IS NULL
            AND node_id IS NULL
            AND template_id IS NOT NULL
            AND template_run_id IS NOT NULL
            AND template_step_id IS NOT NULL
        )
      )) OR (conversation_id IS NOT NULL AND message_id IS NOT NULL
        AND project_id IS NULL AND node_id IS NULL
        AND template_id IS NULL AND template_run_id IS NULL AND template_step_id IS NULL)
    )
);

INSERT INTO creation_tasks_unified (id, creation_task_id, conversation_id, message_id, project_id, template_id, template_run_id, template_step_id, node_id, provider_id, model, capability, params, input_bindings, status, error, result_asset_ids, remote_task_id, attempt, submitted_at, started_at, finished_at, deleted_at, request_fingerprint)
SELECT t.id, t.creation_task_id, COALESCE(m.conversation_id, t.conversation_id), COALESCE(m.message_id, t.message_id), CASE WHEN m.creation_task_id IS NOT NULL THEN NULL ELSE t.project_id END, t.template_id, t.template_run_id, t.template_step_id, t.node_id, t.provider_id, t.model, t.capability, t.params, t.input_bindings, t.status, t.error, t.result_asset_ids, t.remote_task_id, t.attempt, t.submitted_at, t.started_at, t.finished_at, t.deleted_at, CASE WHEN m.creation_task_id IS NULL THEN t.request_fingerprint ELSE json_set(t.request_fingerprint, '$.owner', json_object('kind', 'conversation_turn', 'conversation_id', m.conversation_id, 'message_id', m.message_id)) END
FROM creation_tasks t LEFT JOIN migration_creation_messages m USING (creation_task_id);

DROP TABLE creation_tasks;

ALTER TABLE creation_tasks_unified RENAME TO creation_tasks;

CREATE INDEX idx_creation_tasks_project_id ON creation_tasks(project_id);

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
CREATE INDEX idx_creation_tasks_message ON creation_tasks(message_id) WHERE message_id IS NOT NULL;
CREATE INDEX idx_workshop_assets_origin_conversation ON workshop_assets(json_extract(origin, '$.conversation_id'));
CREATE INDEX idx_workshop_assets_origin_message ON workshop_assets(json_extract(origin, '$.message_id'));

DROP TABLE migration_creation_messages;
DROP TABLE migration_creation_sessions;
DROP TABLE migration_creation_owner_guard;

-- Replace the active origin union; old migration files remain immutable history.
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
       OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
       OR json_type(NEW.origin, '$.templateId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateStepId') IS NOT NULL;

    SELECT RAISE(ABORT, 'invalid creative asset origin conversation_id')
    WHERE json_type(NEW.origin, '$.conversation_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.conversation_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.conversation_id')) = 36
        AND lower(json_extract(NEW.origin, '$.conversation_id')) = json_extract(NEW.origin, '$.conversation_id')
        AND json_extract(NEW.origin, '$.conversation_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.conversation_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin message_id')
    WHERE json_type(NEW.origin, '$.message_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.message_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.message_id')) = 36
        AND lower(json_extract(NEW.origin, '$.message_id')) = json_extract(NEW.origin, '$.message_id')
        AND json_extract(NEW.origin, '$.message_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.message_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin canvas_id')
    WHERE json_type(NEW.origin, '$.canvas_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.canvas_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.canvas_id')) = 36
        AND lower(json_extract(NEW.origin, '$.canvas_id')) = json_extract(NEW.origin, '$.canvas_id')
        AND json_extract(NEW.origin, '$.canvas_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.canvas_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.project_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.project_id')) = 36
        AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
        AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin node_id')
    WHERE json_type(NEW.origin, '$.node_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.node_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.node_id')) = 36
        AND lower(json_extract(NEW.origin, '$.node_id')) = json_extract(NEW.origin, '$.node_id')
        AND json_extract(NEW.origin, '$.node_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.node_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_id')
    WHERE json_type(NEW.origin, '$.template_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_id')) = json_extract(NEW.origin, '$.template_id')
        AND json_extract(NEW.origin, '$.template_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_run_id')
    WHERE json_type(NEW.origin, '$.template_run_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_run_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_run_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_run_id')) = json_extract(NEW.origin, '$.template_run_id')
        AND json_extract(NEW.origin, '$.template_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_step_id')
    WHERE json_type(NEW.origin, '$.template_step_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_step_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_step_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_step_id')) = json_extract(NEW.origin, '$.template_step_id')
        AND json_extract(NEW.origin, '$.template_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset conversation/canvas/template owner branch')
    WHERE (json_type(NEW.origin, '$.conversation_id') IS NOT NULL
        OR json_type(NEW.origin, '$.message_id') IS NOT NULL
        OR json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_step_id') IS NOT NULL)
    AND NOT (
        (json_type(NEW.origin, '$.conversation_id') IS 'text'
            AND json_type(NEW.origin, '$.message_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS 'text'
            AND json_type(NEW.origin, '$.template_run_id') IS 'text'
            AND json_type(NEW.origin, '$.template_step_id') IS 'text')
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
       OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
       OR json_type(NEW.origin, '$.templateId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateStepId') IS NOT NULL;

    SELECT RAISE(ABORT, 'invalid creative asset origin conversation_id')
    WHERE json_type(NEW.origin, '$.conversation_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.conversation_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.conversation_id')) = 36
        AND lower(json_extract(NEW.origin, '$.conversation_id')) = json_extract(NEW.origin, '$.conversation_id')
        AND json_extract(NEW.origin, '$.conversation_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.conversation_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin message_id')
    WHERE json_type(NEW.origin, '$.message_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.message_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.message_id')) = 36
        AND lower(json_extract(NEW.origin, '$.message_id')) = json_extract(NEW.origin, '$.message_id')
        AND json_extract(NEW.origin, '$.message_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.message_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin canvas_id')
    WHERE json_type(NEW.origin, '$.canvas_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.canvas_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.canvas_id')) = 36
        AND lower(json_extract(NEW.origin, '$.canvas_id')) = json_extract(NEW.origin, '$.canvas_id')
        AND json_extract(NEW.origin, '$.canvas_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.canvas_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.project_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.project_id')) = 36
        AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
        AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin node_id')
    WHERE json_type(NEW.origin, '$.node_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.node_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.node_id')) = 36
        AND lower(json_extract(NEW.origin, '$.node_id')) = json_extract(NEW.origin, '$.node_id')
        AND json_extract(NEW.origin, '$.node_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.node_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_id')
    WHERE json_type(NEW.origin, '$.template_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_id')) = json_extract(NEW.origin, '$.template_id')
        AND json_extract(NEW.origin, '$.template_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_run_id')
    WHERE json_type(NEW.origin, '$.template_run_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_run_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_run_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_run_id')) = json_extract(NEW.origin, '$.template_run_id')
        AND json_extract(NEW.origin, '$.template_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_step_id')
    WHERE json_type(NEW.origin, '$.template_step_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_step_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_step_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_step_id')) = json_extract(NEW.origin, '$.template_step_id')
        AND json_extract(NEW.origin, '$.template_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset conversation/canvas/template owner branch')
    WHERE (json_type(NEW.origin, '$.conversation_id') IS NOT NULL
        OR json_type(NEW.origin, '$.message_id') IS NOT NULL
        OR json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_step_id') IS NOT NULL)
    AND NOT (
        (json_type(NEW.origin, '$.conversation_id') IS 'text'
            AND json_type(NEW.origin, '$.message_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS 'text'
            AND json_type(NEW.origin, '$.template_run_id') IS 'text'
            AND json_type(NEW.origin, '$.template_step_id') IS 'text')
    );
END;
