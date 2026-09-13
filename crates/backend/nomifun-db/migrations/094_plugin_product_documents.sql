-- Canonical plugin workspace documents. Preserve identities, revisions and data.
ALTER TABLE miniapp_product_documents RENAME TO plugin_product_documents;
DROP INDEX idx_miniapp_product_documents_owner;
CREATE INDEX idx_plugin_product_documents_owner ON plugin_product_documents(owner_user_id);

-- Draft documents are mutable authoring data, not signed release artifacts.
UPDATE plugin_product_documents
SET content_json = json_set(
    json_remove(content_json, '$.miniapp_id'),
    '$.plugin_id', json_extract(content_json, '$.miniapp_id')
)
WHERE document_key LIKE 'draft:%'
  AND json_type(content_json, '$.miniapp_id') IS NOT NULL;
