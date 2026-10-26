-- Manual cloud-office configuration and business receipts. No Agent facts are
-- converted, rewritten or imported. The only supported metadata evolution is
-- this exact current-generation additive extension.
CREATE TEMP TABLE office_hub_metadata_guard(valid INTEGER NOT NULL CHECK(valid=1));
INSERT INTO office_hub_metadata_guard SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM schema_metadata)
    OR EXISTS(SELECT 1 FROM schema_metadata WHERE singleton_key='canonical'
        AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
        AND canonical_schema_manifest_digest IN (
          'ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9',
          '0ef8e3eeec9835238c865441913383bb80880bf89e3f0d78b538780c7534bdfc'))
    THEN 1 ELSE 0 END;
DROP TABLE office_hub_metadata_guard;

CREATE TABLE office_connections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id TEXT NOT NULL CHECK (length(connection_id) = 36 AND lower(connection_id) = connection_id AND connection_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    owner_id TEXT NOT NULL CHECK (length(owner_id) = 36 AND lower(owner_id) = owner_id AND owner_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 200),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    profile TEXT,
    executable TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE office_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36 AND lower(operation_id) = operation_id AND operation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    owner_id TEXT NOT NULL CHECK (length(owner_id) = 36 AND lower(owner_id) = owner_id AND owner_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    connection_id TEXT NOT NULL CHECK (length(connection_id) = 36 AND lower(connection_id) = connection_id AND connection_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    request_id TEXT NOT NULL CHECK (length(request_id) = 36 AND lower(request_id) = request_id AND request_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(request_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    actor_json TEXT NOT NULL CHECK(json_valid(actor_json) AND json_type(actor_json)='object'),
    profile TEXT NOT NULL CHECK(length(profile) BETWEEN 1 AND 255),
    digest TEXT NOT NULL CHECK(length(digest)=64 AND digest NOT GLOB '*[^0-9a-f]*'),
    status TEXT NOT NULL CHECK(status IN ('draft','queued','running','succeeded','partial','not_executed','unknown','accepted_pending_confirmation')),
    required_permissions_json TEXT NOT NULL CHECK(json_valid(required_permissions_json) AND json_type(required_permissions_json)='array'),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(owner_id,request_id)
);
CREATE INDEX idx_office_operations_owner_history ON office_operations(owner_id,created_at DESC,operation_id DESC);
CREATE TABLE office_operation_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL CHECK (length(item_id) = 36 AND lower(item_id) = item_id AND item_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(item_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36 AND lower(operation_id) = operation_id AND operation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    client_item_id TEXT NOT NULL CHECK(length(client_item_id) BETWEEN 1 AND 255),
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    mutation_json TEXT NOT NULL CHECK(json_valid(mutation_json) AND json_type(mutation_json)='object'),
    target_key TEXT NOT NULL CHECK(length(target_key)=64),
    admitted INTEGER NOT NULL DEFAULT 0 CHECK(admitted IN (0,1)),
    status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','not_executed','unknown','accepted_pending_confirmation')),
    remote_id TEXT,
    remote_url TEXT,
    message TEXT,
    retryable INTEGER NOT NULL DEFAULT 0 CHECK(retryable IN (0,1)),
    UNIQUE(operation_id,client_item_id)
);
CREATE UNIQUE INDEX idx_office_items_pending_target ON office_operation_items(target_key)
    WHERE admitted=1 AND status IN ('queued','running','unknown','accepted_pending_confirmation');
CREATE TABLE office_operation_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_id TEXT NOT NULL CHECK (length(attempt_id) = 36 AND lower(attempt_id) = attempt_id AND attempt_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(attempt_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36 AND lower(operation_id) = operation_id AND operation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    item_id TEXT NOT NULL CHECK (length(item_id) = 36 AND lower(item_id) = item_id AND item_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(item_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    number INTEGER NOT NULL CHECK(number>0),
    client_token TEXT NOT NULL UNIQUE,
    mutation_json TEXT NOT NULL CHECK(json_valid(mutation_json) AND json_type(mutation_json)='object'),
    status TEXT NOT NULL CHECK(status IN ('running','succeeded','not_executed','unknown','accepted_pending_confirmation')),
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    message TEXT,
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    UNIQUE(item_id,number)
);
CREATE TABLE office_operation_observations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    observation_id TEXT NOT NULL CHECK (length(observation_id) = 36 AND lower(observation_id) = observation_id AND observation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(observation_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36 AND lower(operation_id) = operation_id AND operation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    item_id TEXT NOT NULL CHECK (length(item_id) = 36 AND lower(item_id) = item_id AND item_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(item_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    kind TEXT NOT NULL CHECK(kind IN ('original_receipt','target_readback','inconclusive','error')),
    message TEXT NOT NULL,
    data_json TEXT CHECK(data_json IS NULL OR json_valid(data_json)),
    confirmed INTEGER NOT NULL CHECK(confirmed IN (0,1)),
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_office_observations_item ON office_operation_observations(item_id,created_at,id);
CREATE TABLE office_recent_resources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recent_resource_id TEXT NOT NULL CHECK (length(recent_resource_id) = 36 AND lower(recent_resource_id) = recent_resource_id AND recent_resource_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(recent_resource_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    owner_id TEXT NOT NULL CHECK (length(owner_id) = 36 AND lower(owner_id) = owner_id AND owner_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    connection_id TEXT NOT NULL CHECK (length(connection_id) = 36 AND lower(connection_id) = connection_id AND connection_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    resource_key TEXT NOT NULL CHECK(length(resource_key)=64),
    resource_json TEXT NOT NULL CHECK(json_valid(resource_json) AND json_type(resource_json)='object'),
    last_opened_at INTEGER NOT NULL,
    UNIQUE(owner_id,connection_id,resource_key)
);
CREATE INDEX idx_office_recent_owner ON office_recent_resources(owner_id,connection_id,last_opened_at DESC,id DESC);

UPDATE schema_metadata SET canonical_schema_manifest_digest='0ef8e3eeec9835238c865441913383bb80880bf89e3f0d78b538780c7534bdfc'
WHERE singleton_key='canonical'
  AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
  AND canonical_schema_manifest_digest='ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9';
