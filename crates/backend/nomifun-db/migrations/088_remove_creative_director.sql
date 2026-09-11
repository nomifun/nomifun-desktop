-- Retire the Creative Studio Director feature without making an otherwise
-- healthy Canvas unreadable. Remove Director nodes and every edge touching
-- them, normalize the former timeline panel to History, and discard only the
-- hidden text sidecars that were owned exclusively by those nodes.

CREATE TEMP TABLE retired_creative_director_sidecars (
    asset_id TEXT PRIMARY KEY
) WITHOUT ROWID;

INSERT OR IGNORE INTO retired_creative_director_sidecars (asset_id)
SELECT json_extract(node.value, '$.data.sceneId')
FROM creative_studio_projects AS project,
     json_each(project.document_json, '$.nodes') AS node
WHERE json_extract(node.value, '$.type') = 'director'
  AND json_type(node.value, '$.data.sceneId') = 'text';

UPDATE creative_studio_projects
SET document_json = json_set(
        document_json,
        '$.nodes', json((
            SELECT json_group_array(json(node.value))
            FROM json_each(document_json, '$.nodes') AS node
            WHERE json_extract(node.value, '$.type') <> 'director'
        )),
        '$.connections', json((
            SELECT json_group_array(json(connection.value))
            FROM json_each(document_json, '$.connections') AS connection
            WHERE json_extract(connection.value, '$.sourceNodeId') NOT IN (
                    SELECT json_extract(node.value, '$.id')
                    FROM json_each(document_json, '$.nodes') AS node
                    WHERE json_extract(node.value, '$.type') = 'director'
                )
              AND json_extract(connection.value, '$.targetNodeId') NOT IN (
                    SELECT json_extract(node.value, '$.id')
                    FROM json_each(document_json, '$.nodes') AS node
                    WHERE json_extract(node.value, '$.type') = 'director'
                )
        )),
        '$.panels.bottom.activeView', 'history'
    ),
    node_count = (
        SELECT count(*)
        FROM json_each(document_json, '$.nodes') AS node
        WHERE json_extract(node.value, '$.type') <> 'director'
    ),
    connection_count = (
        SELECT count(*)
        FROM json_each(document_json, '$.connections') AS connection
        WHERE json_extract(connection.value, '$.sourceNodeId') NOT IN (
                SELECT json_extract(node.value, '$.id')
                FROM json_each(document_json, '$.nodes') AS node
                WHERE json_extract(node.value, '$.type') = 'director'
            )
          AND json_extract(connection.value, '$.targetNodeId') NOT IN (
                SELECT json_extract(node.value, '$.id')
                FROM json_each(document_json, '$.nodes') AS node
                WHERE json_extract(node.value, '$.type') = 'director'
            )
    ),
    revision = revision + 1
WHERE json_extract(document_json, '$.panels.bottom.activeView') = 'timeline'
   OR EXISTS (
        SELECT 1
        FROM json_each(document_json, '$.nodes') AS node
        WHERE json_extract(node.value, '$.type') = 'director'
    );

DELETE FROM workshop_assets
WHERE kind = 'text'
  AND in_library = 0
  AND rel_path IS NULL
  AND thumb_rel_path IS NULL
  AND asset_id IN (SELECT asset_id FROM retired_creative_director_sidecars)
  AND NOT EXISTS (
      SELECT 1
      FROM creative_studio_projects AS project
      WHERE instr(project.document_json, '"' || workshop_assets.asset_id || '"') > 0
  )
  AND NOT EXISTS (
      SELECT 1
      FROM creation_tasks AS task
      WHERE EXISTS (
          SELECT 1
          FROM json_each(task.input_bindings) AS input
          WHERE json_extract(input.value, '$.asset_id') = workshop_assets.asset_id
      )
         OR EXISTS (
          SELECT 1
          FROM json_each(task.result_asset_ids) AS result
          WHERE result.value = workshop_assets.asset_id
      )
  )
  AND NOT EXISTS (
      SELECT 1
      FROM creative_studio_templates AS template
      WHERE instr(template.definition_json, '"' || workshop_assets.asset_id || '"') > 0
  )
  AND NOT EXISTS (
      SELECT 1
      FROM creative_studio_template_runs AS run
      WHERE instr(run.aggregate_json, '"' || workshop_assets.asset_id || '"') > 0
  );

DROP TABLE retired_creative_director_sidecars;
