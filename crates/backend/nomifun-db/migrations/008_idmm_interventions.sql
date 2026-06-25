-- IDMM 决策记录:把原本仅存于内存(100 条环形缓冲、重启即丢、前端零渲染)的
-- 介入审计落独立表,使决策"看得见、可追溯"。激进淘汰(数据不重要,只留一点):
-- 每 target 仅留最近 30 条 + 全局 TTL 48h + 全局硬上限兜底;target_id 多态
-- (会话 TEXT / 终端 INTEGER),不设双 FK,删会话/终端时由应用层级联清理。
CREATE TABLE IF NOT EXISTS idmm_interventions (
    id           TEXT PRIMARY KEY NOT NULL,   -- idmmrec_{uuidv7}
    target_kind  TEXT    NOT NULL,            -- 'conversation' | 'terminal'
    target_id    TEXT    NOT NULL,
    watch        TEXT    NOT NULL,            -- 'fault'(provider/agent 故障)| 'decision'(其余),由信号种类推导
    at           INTEGER NOT NULL,            -- epoch ms
    signal       TEXT    NOT NULL,            -- stall_class: provider_error/idle/decision/open_question/scheduled
    tier_used    TEXT    NOT NULL,            -- rule | sidecar | rule_fallback
    category     TEXT,                        -- option/open_question/permission/fault
    action       TEXT    NOT NULL,            -- retry/answer_choice/send_text/confirm/wait/stop
    detail       TEXT,                        -- 选了什么/答了什么(截断 ≤2000 字符)
    reason       TEXT,                        -- 思路(模型 reason 或规则解释;非选项分支的描述串也落这里)
    confidence   REAL,                        -- 模型置信度(规则档 NULL)
    bypass_model TEXT,                        -- provider/model(规则档 NULL)
    outcome      TEXT    NOT NULL             -- 规范枚举 applied/resolved/failed/halted/skipped(Phase1 发 applied|halted)
);
CREATE INDEX IF NOT EXISTS idx_idmm_interventions_target
    ON idmm_interventions(target_kind, target_id, at DESC);
CREATE INDEX IF NOT EXISTS idx_idmm_interventions_at
    ON idmm_interventions(at);
