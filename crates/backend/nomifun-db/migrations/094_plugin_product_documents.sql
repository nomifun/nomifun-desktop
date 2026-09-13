-- Canonical plugin workspace documents. Migration 093 already creates the
-- canonical table on a clean start; this migration only normalizes draft
-- payloads that may still carry the old product-qualified association key.
UPDATE plugin_product_documents
SET content_json = json_set(
    json_remove(content_json, '$.plugin_product_id'),
    '$.plugin_id', json_extract(content_json, '$.plugin_product_id')
)
WHERE document_key LIKE 'draft:%'
  AND json_type(content_json, '$.plugin_product_id') IS NOT NULL;
