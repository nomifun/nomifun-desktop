-- A provider model's runtime id is often opaque (for example an inference
-- endpoint id). Keep a generic human-readable label separate from that id.
ALTER TABLE provider_models
ADD COLUMN display_name TEXT
CHECK (
    display_name IS NULL
    OR (
        display_name = trim(display_name)
        AND length(display_name) BETWEEN 1 AND 128
    )
);
