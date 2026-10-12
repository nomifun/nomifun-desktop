-- Published receipts remain unchanged. File-backed startup protects the retired
-- works before this transaction. No Agent facts or immutable artifacts change.
DROP TABLE plugin_library_state;
DROP TABLE plugin_credential_bindings;
DROP TABLE plugin_grants;
DROP TABLE plugin_mutations;
DROP TRIGGER trg_plugin_drafts_clear_base_before_plugin_delete;
DROP TABLE plugin_drafts;
DROP TABLE plugins;
DROP TABLE plugin_artifacts;

CREATE TEMP TABLE app_hosting_schema_integrity (violations INTEGER CHECK (violations = 0));
INSERT INTO app_hosting_schema_integrity SELECT COUNT(*) FROM pragma_foreign_key_check;
DROP TABLE app_hosting_schema_integrity;
