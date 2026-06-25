-- 005_terminal_scrollback.sql
-- 终端 scrollback 跨重启持久化:把原本仅存于内存(256KB 有界缓冲)的输出历史
-- 落到独立表,使应用重启后仍能回放历史显示(配合 boot 对账把幽灵 running
-- 行改成 exited,前端即出现 relaunch 入口并回放这段历史)。
--
-- 用独立表而非给 terminal_sessions 加列:list 查询天然不拉这块大数据(保持
-- 列表轻量),且 ON DELETE CASCADE 随会话删除自动清理。
-- 写入由后端去抖驱动(仅脏会话、~5s 一次 + 进程退出时),绝不每输出块写。

CREATE TABLE IF NOT EXISTS terminal_scrollback (
    session_id  INTEGER PRIMARY KEY NOT NULL,
    data        BLOB    NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES terminal_sessions(id) ON DELETE CASCADE
);
