# R34 CLI 命令循环与退出清理

关联已有问题 R30-02，不重复登记问题编号。全局审计尚未完成。

## 已核对与修改

- 已阅读 CLI 全部生产代码、Cargo 清单及测试。将首条消息前后的两个命令循环合成一个；首条消息前的 Stop 也进入统一清理，Ping/SetConfig 不再提前关闭 MCP 注册阶段。
- 活跃请求收到命令通道 EOF 时取消请求并收尾；只有动态 MCP 注册成功才置 has_mcp。JSON 模式是持续宿主协议，EOF 表示客户端断开，不是单次管道提示词结束。
- 连续 SetConfig 使用最多 32 条的有界队列按接收顺序应用，避免后一个局部更新覆盖整个前一个更新。超限显式拒绝，不置换已接受更新；不在 CLI 重写引擎的 thinking/budget 依赖和无效值规则。
- terminal/JSON 共用私有 shutdown_runtime，依次运行停止 hook、关闭引擎进程、尝试关闭全部 MCP manager；清理失败聚合返回，不再只记日志后成功退出。
- 仅为真实生命周期测试增加私有 reader 注入入口；生产仍在 bootstrap 后启动 stdin reader，没有新增后台任务或协议架构。

## 验证

`cargo test -p nomi-cli -- --test-threads=4`：最终 12/0。四个新增用例覆盖配置顺序、超限不置换、首消息前 Stop 执行真实 stop hook、活跃本地 HTTP 请求收到 EOF 后取消并执行 stop hook。

生命周期测试使用真实 bootstrap/engine、临时目录和本地 TCP listener，不调用外部模型；配置直接构造，不解析用户全局配置。命令来自注入通道，不是实际 OS stdin。测试在修改后加入，没有旧实现红→绿证据。

## 保留范围

- ProtocolSink 的 OutputSink 接口不返回错误，多处直接 writer.emit 忽略错误，emit_json_turn_result 的 I/O 错误未向调用方保留。仍需局部、明确的输出失败取消/返回契约，不据此重构全仓 OutputSink。
- bootstrap 后 init_session 的提前失败尚未统一清理（terminal/JSON 两处）。
- MCP connect_all 在命令循环内等待，连接过程中 Stop/EOF 不能立即打断。活跃请求仍静默忽略其他 Message/AddMcpServer 等命令。
- REPL 使用阻塞 stdin，空行即退出；实际 OS stdin 关闭、broken stdout、真实 MCP 进程清理失败未运行故障验证。
- CLI 标记部分完成；本批未重复无修改 UI 和其他平台验证。
