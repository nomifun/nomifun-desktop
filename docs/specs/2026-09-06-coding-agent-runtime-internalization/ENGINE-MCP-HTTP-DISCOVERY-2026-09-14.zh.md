# 共享 MCP HTTP 目录发现（2026-09-14，未验证）

## 本轮实现

继续 CAR-03 / CAR-07，将设置页 HTTP MCP 目录发现迁入与执行相同的 `McpSession`。
此前 HTTP 设置路径只读取第一页 tools/list、忽略 initialized 通知失败、对响应正文
调用无界 text()，且未关闭已分配的 HTTP Session。现在删除这些重复协议实现，SSE
目录发现也复用同一个 `owner_discovery` 网络入口，stdio 保持受管进程入口。

- 初始化版本、JSON-RPC ID/结果结构、initialized 确认、完整目录分页及工具 schema
  采用现有执行协议检查；最多 32 页、1024 个工具、8 MiB 合计目录数据。重复名称、
  循环 cursor、目录变化及不完整协议不产生成功目录。
- Streamable HTTP 的 JSON 与 SSE 响应使用已有有界解析器、server ping/不支持方法
  明确拒绝策略。不会为了目录发现开放 sampling、elicitation 或 workspace roots。
- 一次协议截止时间覆盖初始化到最后一页，不因分页重置。正常返回、协议失败、超时
  后都进入独立 2 秒清理阶段；已分配 HTTP Session 的 DELETE 不支持/失败/超时则报告
  清理失败，不用目录成功遮盖。无 Session ID 时没有推断出来的清理目标。
- SSE 仍是明确 legacy 协议，使用原来的同源 endpoint、HTTPS 域名限制及有界流。
  关闭本地流不宣称远端进程停止；不重连、不重放。
- 网络发现请求头最多 128 个/64 KiB，拒绝畸形、大小写重复、Host/连接分帧/会话/
  重放控制头；不再静默忽略坏配置。所有自定义值标记 sensitive。接受既有设置/OAuth
  提供的认证材料，只用于本次发现，不等于为 Agent 工具授予执行权限。
- HTTP 401 保留有界、经过现有诊断清理的 WWW-Authenticate，避免无终止错误正文
  吞掉认证提示；设置页继续映射 needs_auth。清理未知仍优先报告清理失败。
- 默认网络 client 保持禁用重定向；注入 client 的调用方仍必须满足同一契约。

## 架构与限制

平台继续持有 MCP transport、凭据与资源权限；Nomi/Coding/社区 Engine 消费同一
Kernel 端口，不复制自己的网络协议 owner。本次发现不调用 tools/call，不修改 Agent
revision/Session binding，也不是 Engine 动态安装。注册仍只允许源码开发后随应用打包。

清理预算并不保证用户直接丢弃整个发现 future 后还能执行远端 DELETE；没有新增后台
重试或远端退出证明。网络握手/会话分配可能影响服务状态，目录发现不是效果回滚。
如果在收到 Session ID 前断连，也不能凭空证明远端没有分配资源。MCP 持久资源/订阅、
经授权 server-initiated 生命周期、新协议及其他生态生命周期仍待实现。

## 状态

Coding `host2-coding-loop38` / Nomi `host21`，两者构建摘要均包含新共享发现源码。
上一切片 Coding 搜索命中指令加载一并保留，详见
[CODING-SEARCH-INSTRUCTIONS-2026-09-14.zh.md](CODING-SEARCH-INSTRUCTIONS-2026-09-14.zh.md)。

按用户要求，未运行构建、测试、设置页连接验证、外部模型/服务/设备调用或数据库迁移。
删除旧协议时同步移除了只覆盖已删除 helper 的测试，保留结果/认证/进程提示测试源码；
不将旧测试记录作为本轮证据。仅本地源码修改和局部 rustfmt，没有 commit/push。
整体 CAR 与 Engine 全能力完善目标仍未完成。
