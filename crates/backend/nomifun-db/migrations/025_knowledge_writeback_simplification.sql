-- 暂存回写已移除：回写只有直写一种落点，开关由 knowledge_bindings.writeback 承担，
-- 所以 writeback_mode 整列失意。回写意识的值域从 conservative|aggressive 改为
-- manual|auto，且语义升级为真实的行为差异（manual 不触发回合末自动抽取）。
--
-- SQLite 无法 ALTER 或 DROP 一条 CHECK 约束，因此值域变更走 ADD → UPDATE → DROP →
-- RENAME（模板见 020_channel_owner_domain.sql）。CHECK 与 DEFAULT 必须同时改：
-- sqlite_conversation.rs 的 INSERT 不点名这些列，依赖 DDL 默认值，只收窄 CHECK 而
-- 留旧 DEFAULT 会让那些 INSERT 全部 CHECK 违例。
--
-- 不重建 knowledge_bindings 表：重建必须逐字复现三处 UUIDv7 GLOB CHECK 与四个部分
-- 唯一索引，而 id_schema_contract 在每次打开库时校验索引的规范化 WHERE 谓词文本。
-- DROP COLUMN 在此合法 —— 两列各自只被自身列级 CHECK 引用，无索引、无生成列。

ALTER TABLE knowledge_bindings ADD COLUMN writeback_eagerness_v2 TEXT NOT NULL
    DEFAULT 'manual' CHECK (writeback_eagerness_v2 IN ('manual', 'auto'));

UPDATE knowledge_bindings SET writeback_eagerness_v2 =
    CASE writeback_eagerness WHEN 'aggressive' THEN 'auto' ELSE 'manual' END;

ALTER TABLE knowledge_bindings DROP COLUMN writeback_eagerness;
ALTER TABLE knowledge_bindings RENAME COLUMN writeback_eagerness_v2 TO writeback_eagerness;
ALTER TABLE knowledge_bindings DROP COLUMN writeback_mode;

-- preset_knowledge_policy 带第二条、完全独立的 eagerness CHECK；漏掉它，新值会被
-- 永久拒绝。eagerness 可空（NULL = 未指定，继承挂载设置），所以 ELSE NULL 保留该
-- 语义而不硬塞默认值。它的 mode 列（无 CHECK，值域 'inherit'|'staged'|'direct'）在
-- 没有"模式"维度后整体失意，一并删除。

ALTER TABLE preset_knowledge_policy ADD COLUMN eagerness_v2 TEXT
    CHECK (eagerness_v2 IS NULL OR eagerness_v2 IN ('manual', 'auto'));

UPDATE preset_knowledge_policy SET eagerness_v2 =
    CASE eagerness
        WHEN 'aggressive' THEN 'auto'
        WHEN 'conservative' THEN 'manual'
        ELSE NULL
    END;

ALTER TABLE preset_knowledge_policy DROP COLUMN eagerness;
ALTER TABLE preset_knowledge_policy RENAME COLUMN eagerness_v2 TO eagerness;
ALTER TABLE preset_knowledge_policy DROP COLUMN mode;
