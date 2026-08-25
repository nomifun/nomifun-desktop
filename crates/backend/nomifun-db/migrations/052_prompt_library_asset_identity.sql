-- Namespaced identity for prompt-library items explicitly materialized in My
-- Assets. Historical rows used only `prompt_catalog_id` and may already contain
-- duplicates. They are intentionally neither rewritten nor deleted: project
-- documents or task history may still reference any one of those asset IDs.
-- The partial index therefore covers only v52 writers that provide both new
-- fields, while the repository keeps recognizing legacy catalog rows.

CREATE TRIGGER validate_prompt_library_asset_origin_insert
BEFORE INSERT ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid prompt library asset origin identity')
    WHERE (
        (json_type(NEW.origin, '$.prompt_library_source') IS NULL)
            <> (json_type(NEW.origin, '$.prompt_library_id') IS NULL)
    ) OR (
        json_type(NEW.origin, '$.prompt_library_source') IS NOT NULL
        AND NOT (
            NEW.kind = 'text'
            AND json_type(NEW.origin, '$.prompt_library_source') = 'text'
            AND json_extract(NEW.origin, '$.prompt_library_source') IN ('catalog', 'preset')
            AND json_type(NEW.origin, '$.prompt_library_id') = 'text'
            AND length(json_extract(NEW.origin, '$.prompt_library_id')) BETWEEN 1 AND 255
            AND trim(json_extract(NEW.origin, '$.prompt_library_id')) =
                json_extract(NEW.origin, '$.prompt_library_id')
        )
    );

    SELECT RAISE(ABORT, 'invalid catalog prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'catalog'
      AND NOT (
          json_type(NEW.origin, '$.prompt_catalog_id') = 'text'
          AND json_extract(NEW.origin, '$.prompt_catalog_id') =
              json_extract(NEW.origin, '$.prompt_library_id')
      );

    SELECT RAISE(ABORT, 'invalid preset prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'preset'
      AND json_type(NEW.origin, '$.prompt_catalog_id') IS NOT NULL;
END;

CREATE TRIGGER validate_prompt_library_asset_origin_update
BEFORE UPDATE OF origin, kind ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid prompt library asset origin identity')
    WHERE (
        (json_type(NEW.origin, '$.prompt_library_source') IS NULL)
            <> (json_type(NEW.origin, '$.prompt_library_id') IS NULL)
    ) OR (
        json_type(NEW.origin, '$.prompt_library_source') IS NOT NULL
        AND NOT (
            NEW.kind = 'text'
            AND json_type(NEW.origin, '$.prompt_library_source') = 'text'
            AND json_extract(NEW.origin, '$.prompt_library_source') IN ('catalog', 'preset')
            AND json_type(NEW.origin, '$.prompt_library_id') = 'text'
            AND length(json_extract(NEW.origin, '$.prompt_library_id')) BETWEEN 1 AND 255
            AND trim(json_extract(NEW.origin, '$.prompt_library_id')) =
                json_extract(NEW.origin, '$.prompt_library_id')
        )
    );

    SELECT RAISE(ABORT, 'invalid catalog prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'catalog'
      AND NOT (
          json_type(NEW.origin, '$.prompt_catalog_id') = 'text'
          AND json_extract(NEW.origin, '$.prompt_catalog_id') =
              json_extract(NEW.origin, '$.prompt_library_id')
      );

    SELECT RAISE(ABORT, 'invalid preset prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'preset'
      AND json_type(NEW.origin, '$.prompt_catalog_id') IS NOT NULL;
END;

CREATE UNIQUE INDEX uq_workshop_assets_prompt_library_identity
    ON workshop_assets(
        json_extract(origin, '$.prompt_library_source'),
        json_extract(origin, '$.prompt_library_id')
    )
    WHERE kind = 'text'
      AND json_type(origin, '$.prompt_library_source') = 'text'
      AND json_type(origin, '$.prompt_library_id') = 'text';
