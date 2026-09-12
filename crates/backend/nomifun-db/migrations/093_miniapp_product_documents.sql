-- Personal library organization and recoverable authoring documents. Product
-- source/releases remain owned by the existing MiniApp application service.
CREATE TABLE miniapp_product_documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    document_key TEXT NOT NULL CHECK (length(document_key) BETWEEN 1 AND 100),
    revision INTEGER NOT NULL CHECK (revision > 0),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE (owner_user_id, document_key)
);
CREATE INDEX idx_miniapp_product_documents_owner ON miniapp_product_documents(owner_user_id);
