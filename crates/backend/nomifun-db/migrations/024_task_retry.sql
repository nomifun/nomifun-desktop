-- 024 编排任务瞬时错误自动重试地基。append-only。
-- 给 orch_run_tasks 加一列,支撑「可重试的模型商错误(限流/超时)不打死整个 run,而是
-- 引擎内有界退避自动重试」:
--   next_retry_at = 该 pending 任务在此 epoch-ms 之前不参与派发(退避中)。
--                   NULL = 不退避(正常 pending 立即可派发) —— 旧行/新建任务皆为 NULL,
--                   `list_ready_tasks` 对 NULL 行为不变,既有 run/plan 零回归。
-- worker 撞「可重试」错误时,引擎把任务置回 pending + attempt+1 + next_retry_at=now+退避,
-- run 仍保持 running;退避到点后任务自然重回 ready 集被重新派发,直至成功或耗尽尝试次数。
ALTER TABLE orch_run_tasks ADD COLUMN next_retry_at INTEGER;
