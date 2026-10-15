-- Retire only the removed product policy field. Published migration receipts,
-- canonical Agent facts and all other saved metadata remain unchanged.
-- Reject damaged policies before updating any row rather than allowing a
-- later typed reader to fall back and discard the owner's other settings.
CREATE TEMP TABLE retired_side_question_policy_integrity (
    valid INTEGER NOT NULL CHECK (valid = 1)
);
INSERT INTO retired_side_question_policy_integrity
SELECT CASE WHEN json_valid(behavior_policy) = 1
    THEN CASE WHEN json_type(behavior_policy) = 'object'
        AND (json_type(behavior_policy, '$.supports_side_question') IS NULL
            OR json_type(behavior_policy, '$.supports_side_question') IN ('true', 'false'))
        AND (json_type(behavior_policy, '$.self_identity_sticky') IS NULL
            OR json_type(behavior_policy, '$.self_identity_sticky') IN ('true', 'false'))
        AND (json_type(behavior_policy, '$.session_load_via_meta_field') IS NULL
            OR json_type(behavior_policy, '$.session_load_via_meta_field') IN ('true', 'false'))
        AND NOT EXISTS (SELECT 1 FROM json_each(behavior_policy)
            WHERE key NOT IN ('supports_side_question', 'self_identity_sticky', 'session_load_via_meta_field'))
        AND NOT EXISTS (SELECT 1 FROM json_each(behavior_policy) GROUP BY key HAVING COUNT(*) > 1)
        THEN 1 ELSE 0 END
    ELSE 0 END
FROM agent_metadata WHERE behavior_policy IS NOT NULL;

UPDATE agent_metadata
SET behavior_policy = json_remove(behavior_policy, '$.supports_side_question')
WHERE behavior_policy IS NOT NULL
    AND json_type(behavior_policy, '$.supports_side_question') IS NOT NULL;

-- Duplicate malformed keys must not leave the retired field readable after
-- one removal. A failed check rolls the entire migration back in sqlx.
INSERT INTO retired_side_question_policy_integrity
SELECT CASE WHEN json_type(behavior_policy, '$.supports_side_question') IS NULL THEN 1 ELSE 0 END
FROM agent_metadata WHERE behavior_policy IS NOT NULL;
DROP TABLE retired_side_question_policy_integrity;
