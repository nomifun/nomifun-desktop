-- The existing install journal also owns the draft association. The Plugin
-- pointer and its source draft must commit or roll back in the same transaction.
ALTER TABLE plugin_mutations ADD COLUMN draft_association_json TEXT CHECK (
    draft_association_json IS NULL OR (
        json_valid(draft_association_json) AND json_type(draft_association_json) = 'object'
        AND json_type(draft_association_json, '$.draft_id') IS 'text'
        AND length(json_extract(draft_association_json, '$.draft_id')) = 36
        AND lower(json_extract(draft_association_json, '$.draft_id')) = json_extract(draft_association_json, '$.draft_id')
        AND json_extract(draft_association_json, '$.draft_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(draft_association_json, '$.draft_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
        AND json_type(draft_association_json, '$.expected_revision') IS 'integer'
        AND json_extract(draft_association_json, '$.expected_revision') > 0
    )
);
