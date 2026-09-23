ALTER TABLE provider_model_capabilities
ADD COLUMN compaction_threshold_pct INTEGER
    CHECK (compaction_threshold_pct IS NULL OR compaction_threshold_pct BETWEEN 50 AND 95);
