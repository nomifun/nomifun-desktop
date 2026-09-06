-- JavaScript Runtime selection is a global host concern, not Plugin state.
-- Absence of this singleton row means an empty RuntimeSelectionRecord at
-- revision zero.

CREATE TABLE javascript_runtime_selection (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key            TEXT NOT NULL UNIQUE CHECK (
        singleton_key = 'javascript_runtime_selection'
    ),
    selected_runtime_json    TEXT CHECK (
        selected_runtime_json IS NULL OR (
            json_valid(selected_runtime_json)
            AND json_type(selected_runtime_json) = 'object'
        )
    ),
    pending_candidate_json   TEXT CHECK (
        pending_candidate_json IS NULL OR (
            json_valid(pending_candidate_json)
            AND json_type(pending_candidate_json) = 'object'
        )
    ),
    validation_result_json   TEXT CHECK (
        validation_result_json IS NULL OR (
            json_valid(validation_result_json)
            AND json_type(validation_result_json) = 'object'
        )
    ),
    last_error_code          TEXT CHECK (
        last_error_code IS NULL OR (
            length(last_error_code) BETWEEN 1 AND 256
            AND last_error_code NOT GLOB '*[^!-~]*'
        )
    ),
    non_recommended_warning_acknowledged_json TEXT NOT NULL DEFAULT '[]' CHECK (
        json_valid(non_recommended_warning_acknowledged_json)
        AND json_type(non_recommended_warning_acknowledged_json) = 'array'
    ),
    revision                 INTEGER NOT NULL CHECK (revision >= 1),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= 0),
    CHECK (
        validation_result_json IS NULL
        OR pending_candidate_json IS NOT NULL
    )
);
