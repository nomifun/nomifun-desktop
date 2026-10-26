-- Compact only manual-office persistence. Migration 016 and all canonical
-- Agent facts keep their original bytes and authority. No mutation is replayed.
CREATE TEMP TABLE office_compact_integrity(valid INTEGER NOT NULL CHECK(valid=1));
INSERT INTO office_compact_integrity SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM schema_metadata)
    OR EXISTS(SELECT 1 FROM schema_metadata WHERE singleton_key='canonical'
      AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
      AND canonical_schema_manifest_digest IN (
        '0ef8e3eeec9835238c865441913383bb80880bf89e3f0d78b538780c7534bdfc',
        '565ff15ff4419e0fad8cf0fbcaa7ade3bd0f4fb50263851efed93f7a39921430'))
    THEN 1 ELSE 0 END;
-- A Draft is disposable only with proof that no execution fact ever existed.
INSERT INTO office_compact_integrity SELECT CASE WHEN NOT EXISTS(
    SELECT 1 FROM office_operations op WHERE op.status='draft' AND (
      EXISTS(SELECT 1 FROM office_operation_items i WHERE i.operation_id=op.operation_id AND (i.admitted<>0 OR i.status<>'queued'))
      OR EXISTS(SELECT 1 FROM office_operation_attempts a WHERE a.operation_id=op.operation_id)
      OR EXISTS(SELECT 1 FROM office_operation_observations o WHERE o.operation_id=op.operation_id))) THEN 1 ELSE 0 END;
INSERT INTO office_compact_integrity SELECT CASE WHEN NOT EXISTS(
    SELECT 1 FROM office_operation_items i LEFT JOIN office_operations op ON op.operation_id=i.operation_id
      WHERE op.operation_id IS NULL OR (op.status<>'draft' AND i.admitted<>1)) THEN 1 ELSE 0 END;
INSERT INTO office_compact_integrity SELECT CASE WHEN NOT EXISTS(
    SELECT 1 FROM office_operation_attempts a LEFT JOIN office_operation_items i ON i.item_id=a.item_id
      WHERE i.item_id IS NULL OR i.operation_id<>a.operation_id)
    AND NOT EXISTS(SELECT 1 FROM office_operation_observations o LEFT JOIN office_operation_items i ON i.item_id=o.item_id
      WHERE i.item_id IS NULL OR i.operation_id<>o.operation_id)
    AND NOT EXISTS(SELECT 1 FROM office_operation_attempts a JOIN office_operation_observations o ON a.attempt_id=o.observation_id)
    THEN 1 ELSE 0 END;
INSERT INTO office_compact_integrity SELECT CASE WHEN NOT EXISTS(
    SELECT 1 FROM office_recent_resources r LEFT JOIN office_connections c ON c.connection_id=r.connection_id
      WHERE c.connection_id IS NULL OR c.owner_id<>r.owner_id
      OR COALESCE(json_type(r.resource_json,'$.kind')='text'
        AND json_extract(r.resource_json,'$.kind') IN ('document','doc','base','table','bitable')
        AND json_type(r.resource_json,'$.id')='text' AND length(json_extract(r.resource_json,'$.id'))>0
        AND json_type(r.resource_json,'$.title')='text'
        AND COALESCE(json_type(r.resource_json,'$.url'),'null') IN ('null','text')
        AND COALESCE(json_type(r.resource_json,'$.base_token'),'null') IN ('null','text')
        AND COALESCE(json_type(r.resource_json,'$.table_id'),'null') IN ('null','text'),0)<>1)
    THEN 1 ELSE 0 END;

ALTER TABLE office_connections ADD COLUMN recent_resources_json TEXT NOT NULL DEFAULT '[]'
    CHECK(json_valid(recent_resources_json) AND json_type(recent_resources_json)='array');
-- Navigation is configuration, not a cached remote document or record snapshot.
UPDATE office_connections SET recent_resources_json=COALESCE((
    SELECT json_group_array(json(locator)) FROM (
      SELECT json_object('kind',json_extract(resource_json,'$.kind'),'id',json_extract(resource_json,'$.id'),
        'title',json_extract(resource_json,'$.title'),'url',json_extract(resource_json,'$.url'),
        'base_token',json_extract(resource_json,'$.base_token'),'table_id',json_extract(resource_json,'$.table_id'),
        'opened_at',last_opened_at) AS locator
      FROM office_recent_resources WHERE owner_id=office_connections.owner_id AND connection_id=office_connections.connection_id
      ORDER BY last_opened_at DESC,id DESC LIMIT 100)), '[]');

CREATE TABLE office_operation_receipts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt_id TEXT NOT NULL CHECK (length(receipt_id) = 36 AND lower(receipt_id) = receipt_id AND receipt_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(receipt_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    item_id TEXT NOT NULL CHECK (length(item_id) = 36 AND lower(item_id) = item_id AND item_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(item_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    record_kind TEXT NOT NULL CHECK(record_kind IN ('attempt','observation')),
    number INTEGER,
    client_token TEXT UNIQUE,
    mutation_json TEXT CHECK(mutation_json IS NULL OR (json_valid(mutation_json) AND json_type(mutation_json)='object')),
    status TEXT CHECK(status IS NULL OR status IN ('running','succeeded','not_executed','unknown','accepted_pending_confirmation')),
    occurred_at INTEGER NOT NULL,
    settled_at INTEGER,
    message TEXT,
    data_json TEXT CHECK(data_json IS NULL OR json_valid(data_json)),
    CHECK(COALESCE((record_kind='attempt' AND number>0 AND client_token IS NOT NULL AND mutation_json IS NOT NULL AND status IS NOT NULL)
      OR (record_kind='observation' AND number IS NULL AND client_token IS NULL AND mutation_json IS NULL
        AND status IS NULL AND settled_at IS NULL AND message IS NOT NULL AND data_json IS NOT NULL
        AND json_type(data_json,'$.kind')='text' AND json_extract(data_json,'$.kind') IN ('original_receipt','target_readback','inconclusive','error')
        AND json_type(data_json,'$.confirmed') IN ('true','false')),0)=1),
    UNIQUE(item_id,number)
);
INSERT INTO office_operation_receipts(receipt_id,item_id,record_kind,number,client_token,mutation_json,status,occurred_at,settled_at,message,data_json)
    SELECT attempt_id,item_id,'attempt',number,client_token,mutation_json,status,started_at,finished_at,message,result_json FROM office_operation_attempts ORDER BY id;
INSERT INTO office_operation_receipts(receipt_id,item_id,record_kind,occurred_at,message,data_json)
    SELECT observation_id,item_id,'observation',created_at,message,
      json_object('kind',kind,'confirmed',json(CASE confirmed WHEN 1 THEN 'true' ELSE 'false' END),'data',json(data_json))
    FROM office_operation_observations ORDER BY id;
INSERT INTO office_compact_integrity SELECT CASE WHEN
    (SELECT COUNT(*) FROM office_operation_receipts WHERE record_kind='attempt')=(SELECT COUNT(*) FROM office_operation_attempts)
    AND (SELECT COUNT(*) FROM office_operation_receipts WHERE record_kind='observation')=(SELECT COUNT(*) FROM office_operation_observations)
    AND NOT EXISTS(SELECT 1 FROM office_operation_attempts a JOIN office_operation_receipts r ON r.receipt_id=a.attempt_id
      WHERE r.mutation_json IS NOT a.mutation_json OR r.data_json IS NOT a.result_json OR r.client_token IS NOT a.client_token
        OR r.status IS NOT a.status OR r.settled_at IS NOT a.finished_at
        OR r.item_id IS NOT a.item_id OR r.number IS NOT a.number OR r.occurred_at IS NOT a.started_at OR r.message IS NOT a.message)
    THEN 1 ELSE 0 END;

CREATE TABLE office_operations_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36 AND lower(operation_id) = operation_id AND operation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    owner_id TEXT NOT NULL CHECK (length(owner_id) = 36 AND lower(owner_id) = owner_id AND owner_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    connection_id TEXT NOT NULL CHECK (length(connection_id) = 36 AND lower(connection_id) = connection_id AND connection_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    request_id TEXT NOT NULL CHECK (length(request_id) = 36 AND lower(request_id) = request_id AND request_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(request_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    actor_json TEXT NOT NULL CHECK(json_valid(actor_json) AND json_type(actor_json)='object'),
    profile TEXT NOT NULL CHECK(length(profile) BETWEEN 1 AND 255),
    digest TEXT NOT NULL CHECK(length(digest)=64 AND digest NOT GLOB '*[^0-9a-f]*'),
    status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','partial','not_executed','unknown','accepted_pending_confirmation')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(owner_id,request_id)
);
INSERT INTO office_operations_next(id,operation_id,owner_id,connection_id,request_id,actor_json,profile,digest,status,created_at,updated_at)
    SELECT id,operation_id,owner_id,connection_id,request_id,actor_json,profile,digest,status,created_at,updated_at
    FROM office_operations WHERE status<>'draft';
CREATE TABLE office_operation_items_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL CHECK (length(item_id) = 36 AND lower(item_id) = item_id AND item_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(item_id, '-', '') NOT GLOB '*[^0-9a-f]*') UNIQUE,
    operation_id TEXT NOT NULL CHECK (length(operation_id) = 36 AND lower(operation_id) = operation_id AND operation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    client_item_id TEXT NOT NULL CHECK(length(client_item_id) BETWEEN 1 AND 255),
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    mutation_json TEXT NOT NULL CHECK(json_valid(mutation_json) AND json_type(mutation_json)='object'),
    target_key TEXT NOT NULL CHECK(length(target_key)=64),
    status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','not_executed','unknown','accepted_pending_confirmation')),
    remote_id TEXT,
    remote_url TEXT,
    message TEXT,
    UNIQUE(operation_id,client_item_id)
);
INSERT INTO office_operation_items_next(id,item_id,operation_id,client_item_id,ordinal,mutation_json,target_key,status,remote_id,remote_url,message)
    SELECT i.id,i.item_id,i.operation_id,i.client_item_id,i.ordinal,i.mutation_json,i.target_key,i.status,i.remote_id,i.remote_url,i.message
    FROM office_operation_items i JOIN office_operations op ON op.operation_id=i.operation_id WHERE op.status<>'draft';
INSERT INTO office_compact_integrity SELECT CASE WHEN
    (SELECT COUNT(*) FROM office_operations_next)=(SELECT COUNT(*) FROM office_operations WHERE status<>'draft')
    AND (SELECT COUNT(*) FROM office_operation_items_next)=(SELECT COUNT(*) FROM office_operation_items i JOIN office_operations op ON op.operation_id=i.operation_id WHERE op.status<>'draft')
    AND NOT EXISTS(SELECT 1 FROM office_operation_receipts r LEFT JOIN office_operation_items_next i ON i.item_id=r.item_id WHERE i.item_id IS NULL)
    THEN 1 ELSE 0 END;
-- Preserve AUTOINCREMENT high-water marks even when discarded Drafts owned the
-- greatest local row id. Business receipt identities are copied unchanged.
INSERT INTO sqlite_sequence(name,seq) SELECT 'office_operations_next',seq FROM sqlite_sequence
    WHERE name='office_operations' AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='office_operations_next');
UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='office_operations'),seq))
    WHERE name='office_operations_next';
INSERT INTO sqlite_sequence(name,seq) SELECT 'office_operation_items_next',seq FROM sqlite_sequence
    WHERE name='office_operation_items' AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='office_operation_items_next');
UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='office_operation_items'),seq))
    WHERE name='office_operation_items_next';
DROP TABLE office_operation_attempts;
DROP TABLE office_operation_observations;
DROP TABLE office_recent_resources;
DROP TABLE office_operation_items;
DROP TABLE office_operations;
ALTER TABLE office_operations_next RENAME TO office_operations;
ALTER TABLE office_operation_items_next RENAME TO office_operation_items;
CREATE INDEX idx_office_operations_owner_history ON office_operations(owner_id,created_at DESC,operation_id DESC);
CREATE UNIQUE INDEX idx_office_items_pending_target ON office_operation_items(target_key)
    WHERE status IN ('queued','running','unknown','accepted_pending_confirmation');
UPDATE schema_metadata SET canonical_schema_manifest_digest='565ff15ff4419e0fad8cf0fbcaa7ade3bd0f4fb50263851efed93f7a39921430'
    WHERE singleton_key='canonical' AND data_generation=7 AND migration_head=1 AND projection_schema_version=1
      AND canonical_schema_manifest_digest='0ef8e3eeec9835238c865441913383bb80880bf89e3f0d78b538780c7534bdfc';
DROP TABLE office_compact_integrity;
