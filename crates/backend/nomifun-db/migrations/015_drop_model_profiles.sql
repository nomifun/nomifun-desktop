-- provider_models (migration 014) is now the authoritative per-model store;
-- every Rust consumer has switched. Remove the superseded table.
DROP INDEX idx_model_profiles_provider_id;
DROP TABLE model_profiles;
