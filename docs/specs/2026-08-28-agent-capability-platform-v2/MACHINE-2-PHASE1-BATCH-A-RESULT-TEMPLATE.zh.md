# Machine 2 Batch A 结果回传模板

> 本文件是回传格式模板，不是状态台账，也不记录凭据、主机地址或私密信息。
>
> 机器 2 不应直接修改本模板；请复制到临时目录、GitHub PR 描述或 issue comment，
> 回传完成后由主机写入实际提交摘要。

## 基线

- `source_branch`：
- `integration_branch`：
- `base_sha`：执行时解析的 `origin/rf/agent-capability-platform-v2`
- `batch_result_sha`：
- `integration_method`：ordinary commit / fast-forward merge

## Writer 结果

### Session Data

- 任务：`SL-S2-05`、`SL-S2-06`
- 独立提交：
- changed paths：
- 验证命令与结果：
- 未运行项及准确原因：
- blocker：
- 是否触碰中央路径：是 / 否

### SSH Owner

- 任务：`SL-S3-09`
- 独立提交：
- changed paths：
- 验证命令与结果：
- 未运行项及准确原因：
- blocker：
- 是否触碰中央路径：是 / 否

### Sidecar Upstream Spike

- 任务：`SL-S2-10`
- 独立提交：
- changed paths：
- upstream checkout/commit：
- 实际调用 trace：
- 验证命令与结果：
- 未运行项及准确原因：
- patch/no-patch 结论：
- blocker：
- 是否触碰中央路径：是 / 否

## 合流与安全检查

- integration branch 最终 SHA：
- `git diff --check`：
- `git status --short`：
- secret scan：
- 未修改的中央路径：
- 需要主机完成的最小接线：
- 机器 2 相比单 SSH lane 实际并发完成的任务：

## 阻塞处理

只记录首个完整失败、环境/harness 原因和人工替代步骤。不要重复重试，不要构造
mock PASS、synthetic evidence 或绕过产品语义的测试。
