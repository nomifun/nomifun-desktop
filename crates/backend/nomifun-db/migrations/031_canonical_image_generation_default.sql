-- Replace the retired tool-owned image model preference with the model-owned
-- task default.  A future/canonical value wins if both keys are present.
--
-- First sanitize a canonical key that may have been written by a beta build or
-- by hand.  Leaving malformed JSON here would make Provider deletion fail
-- closed forever when it scans reference-bearing preferences.
DELETE FROM client_preferences
WHERE key = 'models.default.imageGeneration'
  AND CASE
      WHEN json_valid(value) = 0 THEN 1
      WHEN json_type(value) <> 'object' THEN 1
      WHEN COALESCE(json_type(value, '$.provider_id'), '') <> 'text' THEN 1
      WHEN COALESCE(json_type(value, '$.model'), '') <> 'text' THEN 1
      WHEN length(json_extract(value, '$.provider_id')) = 0 THEN 1
      WHEN trim(
          json_extract(value, '$.provider_id'),
          char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)
      ) <> json_extract(value, '$.provider_id') THEN 1
      WHEN length(json_extract(value, '$.model')) = 0 THEN 1
      WHEN trim(
          json_extract(value, '$.model'),
          char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)
      ) <> json_extract(value, '$.model') THEN 1
      WHEN NOT EXISTS (
          SELECT 1
          FROM providers provider
          WHERE provider.provider_id = json_extract(value, '$.provider_id')
      ) THEN 1
      ELSE 0
  END = 1;

-- Rebuild every surviving canonical value to its closed shape, dropping beta
-- or tool-era fields while preserving the canonical row's precedence/time.
UPDATE client_preferences
SET value = json_object(
    'provider_id', json_extract(value, '$.provider_id'),
    'model', json_extract(value, '$.model')
)
WHERE key = 'models.default.imageGeneration';

-- Only legacy values that satisfy the old public shape and still point at an
-- existing Provider are carried forward. Rebuilding the object deliberately
-- drops the obsolete `switch` field (and any other tool-era fields).
INSERT INTO client_preferences (key, value, updated_at)
SELECT
    'models.default.imageGeneration',
    json_object(
        'provider_id', json_extract(legacy.value, '$.provider_id'),
        'model', json_extract(legacy.value, '$.model')
    ),
    legacy.updated_at
FROM client_preferences legacy
WHERE legacy.key = 'tools.imageGenerationModel'
  AND CASE
      WHEN json_valid(legacy.value) = 0 THEN 0
      WHEN json_type(legacy.value) <> 'object' THEN 0
      WHEN COALESCE(json_type(legacy.value, '$.provider_id'), '') <> 'text' THEN 0
      WHEN COALESCE(json_type(legacy.value, '$.model'), '') <> 'text' THEN 0
      WHEN length(json_extract(legacy.value, '$.provider_id')) = 0 THEN 0
      WHEN trim(
          json_extract(legacy.value, '$.provider_id'),
          char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)
      ) <> json_extract(legacy.value, '$.provider_id') THEN 0
      WHEN length(json_extract(legacy.value, '$.model')) = 0 THEN 0
      WHEN trim(
          json_extract(legacy.value, '$.model'),
          char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)
      ) <> json_extract(legacy.value, '$.model') THEN 0
      WHEN NOT EXISTS (
          SELECT 1
          FROM providers provider
          WHERE provider.provider_id = json_extract(legacy.value, '$.provider_id')
      ) THEN 0
      ELSE 1
  END = 1
ON CONFLICT(key) DO NOTHING;

-- Invalid legacy values are intentionally discarded too: after this migration
-- there is exactly one authority and no runtime reader may fall back to the old
-- tool-owned key.
DELETE FROM client_preferences
WHERE key = 'tools.imageGenerationModel';
