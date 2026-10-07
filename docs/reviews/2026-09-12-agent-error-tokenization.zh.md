# R23 审计记录：Agent 错误诊断分词

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R23-01：分词后不能再寻找带空格的 Bearer 前缀

真实 sanitize_error_detail 回归在旧实现泄漏 unknown-secret。split_whitespace
之后 starts_with("bearer ") 永远为 false。改为识别 Bearer token 后清理下一词，
保持该状态仅属于当前行；覆盖混合大小写、制表符、HTML 清理、引号/括号包装。
不删除后续失败原因，不把 bearer-like 当认证方案，也不跨行吞掉无凭据诊断。

- 新增 2 项测试，其中 1 项旧实现失败，另 1 项原行为即通过；最终 send_error 38/0。
- 删除不可命中的空格前缀分支。未扩展为全 Agent 模块完成；其他分词脱敏、错误
  分类/权限/执行/生命周期路径仍按模块台账继续。

## R20-02 的额外证据

卡住的 model-invoke probe_400_on_json_task_stays_unhealthy 单独运行 1/0（1.27s）；
同一份代码的完整 lib 测试使用 --test-threads=4 为 396/0（37.04s）。这支持默认
高并发下存在资源争用的判断，尚未定位具体热点，不能用降低并发宣称修复了根因。
保留 R20-02 待审，后续验证可用 4 线程避免重复长时间争用。
