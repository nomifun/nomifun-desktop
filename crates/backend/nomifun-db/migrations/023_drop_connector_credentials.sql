-- The Feishu knowledge connector was removed before it ever shipped a working
-- integration: no code reads `connector_credentials` or the
-- `extra.source.credentialRef` / `scope` / `sync` keys anymore, and
-- `KnowledgeSource` deserializes with `deny_unknown_fields`. Drop the storage
-- and scrub any connector-shaped source configs so remaining rows keep
-- deserializing against the URL-only source contract.
UPDATE knowledge_bases
SET extra = json_remove(extra, '$.source')
WHERE json_extract(extra, '$.source.kind') IS NOT NULL
  AND json_extract(extra, '$.source.kind') <> 'url';

UPDATE knowledge_bases
SET extra = json_remove(extra, '$.source.credentialRef', '$.source.scope', '$.source.sync')
WHERE json_extract(extra, '$.source') IS NOT NULL;

DROP INDEX idx_knowledge_bases_extra_credential_ref;
DROP TABLE connector_credentials;
