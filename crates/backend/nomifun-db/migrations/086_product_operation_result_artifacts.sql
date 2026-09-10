-- Persist content-addressed outputs for Build/Import/Export operations.

ALTER TABLE product_operations ADD COLUMN result_artifact_digests_json TEXT NOT NULL
    DEFAULT '{}' CHECK (
        json_valid(result_artifact_digests_json)
        AND json_type(result_artifact_digests_json) = 'object'
    );

ALTER TABLE plugin_ready_candidates ADD COLUMN imported_test_provenance_json TEXT CHECK (
    imported_test_provenance_json IS NULL OR (
        json_valid(imported_test_provenance_json)
        AND json_type(imported_test_provenance_json) = 'object'
    )
);

CREATE TRIGGER trg_product_operation_result_insert_guard
BEFORE INSERT ON product_operations
WHEN NEW.result_artifact_digests_json <> '{}'
BEGIN
    SELECT RAISE(ABORT, 'product operation must begin without result Artifacts');
END;

CREATE TRIGGER trg_plugin_candidate_imported_provenance_guard
BEFORE INSERT ON plugin_ready_candidates
WHEN NEW.imported_test_provenance_json IS NOT NULL
AND NEW.origin_kind <> 'import'
BEGIN
    SELECT RAISE(ABORT, 'only imported Plugin Candidates can carry source Test provenance');
END;

CREATE TRIGGER trg_product_operation_result_guard
BEFORE UPDATE OF result_artifact_digests_json ON product_operations
WHEN (
    NEW.state <> 'succeeded'
    AND NEW.result_artifact_digests_json <> '{}'
)
OR EXISTS (
    SELECT 1
      FROM json_each(NEW.result_artifact_digests_json) entry
     WHERE entry.type <> 'text'
        OR length(entry.key) NOT BETWEEN 1 AND 64
        OR entry.key GLOB '*[^a-z0-9._-]*'
        OR length(entry.value) <> 64
        OR lower(entry.value) <> entry.value
        OR entry.value GLOB '*[^0-9a-f]*'
)
BEGIN
    SELECT RAISE(ABORT, 'product operation result Artifacts must be bounded SHA-256 facts');
END;
