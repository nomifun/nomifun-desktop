-- First-class mutation authority for knowledge-base filesystem trees.
--
-- Older builds stored this authority in the open-ended `extra` JSON object.
-- Bases created before that metadata existed must remain editable: those
-- registrations predate the read-only product mode and previously exposed the
-- complete file CRUD surface. Explicit valid choices are retained. Once the
-- value has been projected into the constrained column, only that migrated key
-- is removed from `extra`; every other metadata field remains intact.

ALTER TABLE knowledge_bases
    ADD COLUMN tree_access TEXT NOT NULL DEFAULT 'editable'
    CHECK (tree_access IN ('editable', 'read_only'));

UPDATE knowledge_bases
SET tree_access = CASE
        WHEN json_valid(extra) THEN
            CASE
                WHEN json_type(extra) <> 'object' THEN 'read_only'
                WHEN json_type(extra, '$.tree_access') IS NULL THEN 'editable'
                WHEN json_type(extra, '$.tree_access') = 'text' THEN
                    CASE json_extract(extra, '$.tree_access')
                        WHEN 'read_only' THEN 'read_only'
                        WHEN 'editable' THEN 'editable'
                        ELSE 'read_only'
                    END
                ELSE 'read_only'
            END
        ELSE 'read_only'
    END,
    extra = CASE
        WHEN json_valid(extra) THEN
            CASE
                WHEN json_type(extra) = 'object'
                    THEN json_remove(extra, '$.tree_access')
                ELSE extra
            END
        ELSE extra
    END;
