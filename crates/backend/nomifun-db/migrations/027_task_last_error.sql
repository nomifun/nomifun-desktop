-- 027 编排任务失败原因持久化。append-only,基线不动。
-- 给 orch_run_tasks 加一个可空列 last_error:worker 永久失败(重试耗尽/不可重试/超时)时,
-- 引擎把失败原因摘要(错误码+消息,或「超时无响应」)落到这一列,供:
--   1) 主管(lead)会话回执/升级时不必回读 worker 会话即可拿到「为什么失败」;
--   2) 诊断读工具(nomi_run_status/result/detail)向主 agent 暴露失败原因。
-- 纯 ADD COLUMN(O(1),不重建表);旧行读回 NULL —— 既有 run/plan 零回归。
ALTER TABLE orch_run_tasks ADD COLUMN last_error TEXT;
