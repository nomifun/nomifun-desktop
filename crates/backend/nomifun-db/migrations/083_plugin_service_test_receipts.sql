-- Immutable owner-scoped Plugin Service Test receipt history.
--
-- Each receipt freezes the exact Ready Release and Product/config/credential
-- revisions tested by one transient Service Host run. Retests append a new
-- row; the repository replaces only the current Ready record's receipt
-- reference. Permanent Plugin deletion removes the aggregate's receipt
-- history explicitly. No physical foreign keys or triggers are used.

CREATE TABLE plugin_service_test_receipts (
    id                                  INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt_id                          TEXT NOT NULL UNIQUE CHECK (
        length(receipt_id) = 36
        AND lower(receipt_id) = receipt_id
        AND receipt_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(receipt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id                       TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id                          TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    release_id                          TEXT NOT NULL CHECK (
        length(release_id) = 36
        AND lower(release_id) = release_id
        AND release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    release_digest                      TEXT NOT NULL CHECK (
        length(release_digest) = 64
        AND lower(release_digest) = release_digest
        AND release_digest NOT GLOB '*[^0-9a-f]*'
    ),
    service_run_key                     TEXT NOT NULL CHECK (
        length(service_run_key) = 64
        AND lower(service_run_key) = service_run_key
        AND service_run_key NOT GLOB '*[^0-9a-f]*'
    ),
    outcome                             TEXT NOT NULL CHECK (
        outcome IN ('passed', 'failed', 'needs_test_input')
    ),
    error_code                          TEXT CHECK (
        error_code IS NULL OR (
            length(error_code) BETWEEN 1 AND 256
            AND error_code NOT GLOB '*[^!-~]*'
        )
    ),
    receipt_digest                      TEXT NOT NULL CHECK (
        length(receipt_digest) = 64
        AND lower(receipt_digest) = receipt_digest
        AND receipt_digest NOT GLOB '*[^0-9a-f]*'
    ),
    runtime_fingerprint_digest          TEXT NOT NULL CHECK (
        length(runtime_fingerprint_digest) = 64
        AND lower(runtime_fingerprint_digest) = runtime_fingerprint_digest
        AND runtime_fingerprint_digest NOT GLOB '*[^0-9a-f]*'
    ),
    resolved_test_input_digest          TEXT NOT NULL CHECK (
        length(resolved_test_input_digest) = 64
        AND lower(resolved_test_input_digest) = resolved_test_input_digest
        AND resolved_test_input_digest NOT GLOB '*[^0-9a-f]*'
    ),
    tested_product_revision             INTEGER NOT NULL CHECK (
        tested_product_revision >= 1
    ),
    tested_pointer_revision             INTEGER NOT NULL CHECK (
        tested_pointer_revision >= 1
    ),
    tested_config_revision              INTEGER NOT NULL CHECK (
        tested_config_revision >= 1
    ),
    tested_credential_bindings_revision INTEGER NOT NULL CHECK (
        tested_credential_bindings_revision >= 1
    ),
    receipt_json                        TEXT NOT NULL CHECK (
        json_valid(receipt_json)
        AND json_type(receipt_json) = 'object'
    ),
    issued_at_ms                        INTEGER NOT NULL CHECK (issued_at_ms > 0),
    UNIQUE (owner_user_id, plugin_product_id, receipt_id)
);

CREATE INDEX idx_plugin_service_test_receipts_owner_user_id
    ON plugin_service_test_receipts(owner_user_id);
CREATE INDEX idx_plugin_service_test_receipts_plugin_product_id
    ON plugin_service_test_receipts(plugin_product_id);
CREATE INDEX idx_plugin_service_test_receipts_release_id
    ON plugin_service_test_receipts(release_id);
CREATE INDEX idx_plugin_service_test_receipts_current
    ON plugin_service_test_receipts(
        owner_user_id,
        plugin_product_id,
        release_id,
        release_digest,
        issued_at_ms DESC
    );
