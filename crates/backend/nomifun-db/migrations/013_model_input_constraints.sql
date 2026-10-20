-- User-authored model input limits are configuration, not Agent Session data.
ALTER TABLE provider_model_capabilities ADD COLUMN input_constraints TEXT;
