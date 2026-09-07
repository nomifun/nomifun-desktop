ALTER TABLE javascript_runtime_selection
ADD COLUMN selected_executable_path TEXT;

ALTER TABLE javascript_runtime_selection
ADD COLUMN pending_candidate_executable_path TEXT;

-- Migration 068 did not persist executable paths, so an existing fingerprint
-- cannot be proven to identify the same executable after restart. Fail closed
-- by requiring one fresh probe instead of accepting an unverifiable binding.
UPDATE javascript_runtime_selection
SET selected_runtime_json = NULL,
    selected_executable_path = NULL,
    pending_candidate_json = NULL,
    pending_candidate_executable_path = NULL,
    validation_result_json = NULL,
    last_error_code = 'JAVASCRIPT_RUNTIME_RESELECTION_REQUIRED',
    revision = revision + 1
WHERE selected_runtime_json IS NOT NULL
   OR pending_candidate_json IS NOT NULL
   OR validation_result_json IS NOT NULL;
