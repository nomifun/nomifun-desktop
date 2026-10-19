-- URL-only tokens have no verifiable owner, issuer, resource or client identity.
-- Retire those credentials; keep server configuration and all other domains.
CREATE TEMP TABLE mcp_oauth_metadata_guard (valid INTEGER NOT NULL CHECK(valid=1));
INSERT INTO mcp_oauth_metadata_guard SELECT CASE WHEN NOT EXISTS (SELECT 1 FROM schema_metadata)
    OR EXISTS (SELECT 1 FROM schema_metadata WHERE singleton_key='canonical'
        AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
        AND canonical_schema_manifest_digest IN ('181fba4223793ec7cfd298a8a3d026925c54b622172d1c962d5bf70816e1a019','ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9'))
    THEN 1 ELSE 0 END;
DROP TABLE mcp_oauth_metadata_guard;
DROP TABLE oauth_tokens;

CREATE TABLE mcp_oauth_clients (
    client_ref TEXT PRIMARY KEY NOT NULL CHECK (
        length(client_ref) = 36 AND lower(client_ref) = client_ref
        AND client_ref GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(client_ref, '-', '') NOT GLOB '*[^0-9a-f]*'),
    owner_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    issuer TEXT NOT NULL CHECK (length(issuer) BETWEEN 1 AND 4096),
    profile_key TEXT NOT NULL CHECK (length(profile_key) BETWEEN 1 AND 1024),
    registration_kind TEXT NOT NULL CHECK (registration_kind IN ('dynamic','static','cimd')),
    client_id TEXT NOT NULL CHECK (length(client_id) BETWEEN 1 AND 4096),
    token_endpoint_auth_method TEXT NOT NULL CHECK (token_endpoint_auth_method IN ('none','client_secret_basic','client_secret_post')),
    redirect_uris_json TEXT NOT NULL CHECK (json_valid(redirect_uris_json) AND json_type(redirect_uris_json)='array'),
    client_secret_encrypted TEXT CHECK (client_secret_encrypted IS NULL OR length(client_secret_encrypted)>0),
    client_secret_expires_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (client_ref,owner_id,issuer),
    CHECK (length(owner_id) = 36 AND lower(owner_id) = owner_id AND owner_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK ((token_endpoint_auth_method='none' AND client_secret_encrypted IS NULL) OR (token_endpoint_auth_method<>'none' AND client_secret_encrypted IS NOT NULL))
);
CREATE INDEX idx_mcp_oauth_clients_profile ON mcp_oauth_clients(owner_id,issuer,profile_key,created_at DESC,client_ref DESC);

CREATE TABLE mcp_oauth_grants (
    grant_id TEXT PRIMARY KEY NOT NULL CHECK (
        length(grant_id) = 36 AND lower(grant_id) = grant_id
        AND grant_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(grant_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    owner_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    server_id TEXT NOT NULL REFERENCES mcp_servers(mcp_server_id) ON DELETE CASCADE,
    connection_config_ref TEXT NOT NULL CHECK (length(connection_config_ref) BETWEEN 1 AND 4096),
    issuer TEXT NOT NULL CHECK (length(issuer) BETWEEN 1 AND 4096),
    resource TEXT NOT NULL CHECK (length(resource) BETWEEN 1 AND 4096),
    client_ref TEXT NOT NULL,
    scopes_json TEXT NOT NULL CHECK (json_valid(scopes_json) AND json_type(scopes_json)='array'),
    token_encrypted TEXT NOT NULL CHECK (length(token_encrypted)>0),
    expires_at INTEGER,
    obtained_at INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision>0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (owner_id,server_id),
    FOREIGN KEY (client_ref,owner_id,issuer) REFERENCES mcp_oauth_clients(client_ref,owner_id,issuer) ON DELETE RESTRICT,
    CHECK (length(owner_id) = 36 AND lower(owner_id) = owner_id AND owner_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK (length(server_id) = 36 AND lower(server_id) = server_id AND server_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(server_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK (length(client_ref) = 36 AND lower(client_ref) = client_ref AND client_ref GLOB '????????-????-7???-[89ab]???-????????????' AND replace(client_ref, '-', '') NOT GLOB '*[^0-9a-f]*')
);
CREATE INDEX idx_mcp_oauth_grants_client ON mcp_oauth_grants(client_ref);

-- Preserve canonical Agent facts while extending the manifest with the two
-- non-Agent configuration owners. Unknown metadata continues to fail closed.
UPDATE schema_metadata SET canonical_schema_manifest_digest='ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9'
WHERE singleton_key='canonical'
  AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
  AND canonical_schema_manifest_digest='181fba4223793ec7cfd298a8a3d026925c54b622172d1c962d5bf70816e1a019';

CREATE TEMP TABLE mcp_oauth_integrity (valid INTEGER NOT NULL CHECK(valid=1));
INSERT INTO mcp_oauth_integrity SELECT CASE WHEN COUNT(*)=0 THEN 1 ELSE 0 END FROM pragma_foreign_key_check;
DROP TABLE mcp_oauth_integrity;
