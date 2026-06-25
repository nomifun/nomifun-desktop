-- 002_companion_rename.sql
-- 「pet」域整体更名为「companion」(数字伙伴)。001_baseline 已冻结(sqlx 校验和)，
-- 故所有列/值的前向更名集中在本迁移：新库 = 001 建 pet_* 列后由本迁移改为 companion_*；
-- 既有库 = 本迁移把存量数据迁到 companion 名。代码侧标识符已全部改为 companion。

-- 1) assistant_plugins.pet_id -> companion_id（该列无索引/约束依赖，直接改名）。
ALTER TABLE assistant_plugins RENAME COLUMN pet_id TO companion_id;

-- 2) knowledge_bindings：target_pet_id -> target_companion_id 且 kind 'pet' -> 'companion'。
--    CHECK 约束内嵌字面量 'pet' 与列名，SQLite 无法 ALTER CHECK，需整表重建。
--    knowledge_binding_bases 通过 ON DELETE CASCADE 挂在本表下：重建期先备份、后恢复，
--    binding_id 全程保留以维持外键引用；对 foreign_keys 开/关两态都正确(DELETE+INSERT 幂等)。
CREATE TEMP TABLE _kbb_backup AS SELECT * FROM knowledge_binding_bases;

CREATE TABLE knowledge_bindings_new (
    binding_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    target_kind         TEXT    NOT NULL,
    target_workpath     TEXT,
    target_conv_id      INTEGER,
    target_term_id      INTEGER,
    target_companion_id TEXT,
    enabled             INTEGER NOT NULL DEFAULT 0,
    writeback           INTEGER NOT NULL DEFAULT 0,
    writeback_mode      TEXT    NOT NULL DEFAULT 'staged'
                                CHECK(writeback_mode IN ('staged', 'direct')),
    writeback_eagerness TEXT    NOT NULL DEFAULT 'conservative'
                                CHECK(writeback_eagerness IN ('conservative', 'aggressive')),
    updated_at          INTEGER NOT NULL,
    FOREIGN KEY (target_conv_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (target_term_id) REFERENCES terminal_sessions(id) ON DELETE CASCADE,
    CHECK (
        (target_kind = 'workpath'     AND target_workpath IS NOT NULL
            AND target_conv_id IS NULL AND target_term_id IS NULL AND target_companion_id IS NULL)
     OR (target_kind = 'conversation' AND target_conv_id  IS NOT NULL
            AND target_workpath IS NULL AND target_term_id IS NULL AND target_companion_id IS NULL)
     OR (target_kind = 'terminal'     AND target_term_id  IS NOT NULL
            AND target_workpath IS NULL AND target_conv_id IS NULL AND target_companion_id IS NULL)
     OR (target_kind = 'companion'    AND target_companion_id IS NOT NULL
            AND target_workpath IS NULL AND target_conv_id IS NULL AND target_term_id IS NULL)
    )
);

INSERT INTO knowledge_bindings_new
    (binding_id, target_kind, target_workpath, target_conv_id, target_term_id, target_companion_id, enabled, writeback, writeback_mode, writeback_eagerness, updated_at)
SELECT binding_id,
       CASE WHEN target_kind = 'pet' THEN 'companion' ELSE target_kind END,
       target_workpath, target_conv_id, target_term_id, target_pet_id,
       enabled, writeback, writeback_mode, writeback_eagerness, updated_at
FROM knowledge_bindings;

DROP TABLE knowledge_bindings;
ALTER TABLE knowledge_bindings_new RENAME TO knowledge_bindings;

-- 恢复子表（CASCADE 可能已清空，或 FK 关闭时仍在）：清后按备份重灌，保证幂等无重复。
DELETE FROM knowledge_binding_bases;
INSERT INTO knowledge_binding_bases SELECT * FROM _kbb_backup;
DROP TABLE _kbb_backup;

CREATE UNIQUE INDEX IF NOT EXISTS uq_kb_binding_workpath  ON knowledge_bindings(target_workpath)     WHERE target_workpath     IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_kb_binding_conv      ON knowledge_bindings(target_conv_id)      WHERE target_conv_id      IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_kb_binding_term      ON knowledge_bindings(target_term_id)      WHERE target_term_id      IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_kb_binding_companion ON knowledge_bindings(target_companion_id) WHERE target_companion_id IS NOT NULL;

-- 3) conversations.extra：petCompanion -> companionSession（保留 JSON 布尔型，后端 as_bool 依赖），
--    petId -> companionId。companion 会话两键恒同时存在；先迁真值行，再清理任何遗留旧键。
UPDATE conversations
SET extra = json_remove(
              json_set(
                json_set(extra, '$.companionId', json_extract(extra, '$.petId')),
                '$.companionSession', json('true')
              ),
              '$.petId', '$.petCompanion')
WHERE json_valid(extra) AND json_extract(extra, '$.petCompanion') = 1;

UPDATE conversations
SET extra = json_remove(extra, '$.petCompanion', '$.petId')
WHERE json_valid(extra) AND (json_extract(extra, '$.petCompanion') IS NOT NULL OR json_extract(extra, '$.petId') IS NOT NULL);
