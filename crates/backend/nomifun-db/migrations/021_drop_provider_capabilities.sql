-- providers.capabilities was the last legacy provider-level model-surface
-- column: migration 016 already dropped the models array + five per-model
-- maps, and every Rust reader now projects the per-model surface from
-- provider_models rows. The wire keeps accepting `capabilities` for request
-- compat, but the value is ignored and `ProviderResponse.capabilities` is
-- always []. The baseline DDL defines no index, constraint, or generated
-- column over this column, so SQLite DROP COLUMN is legal here.
ALTER TABLE providers DROP COLUMN capabilities;
