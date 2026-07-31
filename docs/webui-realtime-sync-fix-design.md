# WebUI/Desktop 实时同步统一修复设计（2026-07-31）

## 确认的根因（多 agent 对抗验证后）

**排除项**（已验证不是原因）：事件名/负载契约不匹配；单管理员部署的 user_id 身份链断裂；
Caddy/反向代理（默认部署无代理）；写回解耦/update_plan 投影等近期提交。

**确认的缺陷簇**（每条均有代码证据）：

### 传输层（客户端 httpBridge.ts — 主因簇）
1. **无活性看门狗**：半开 socket 永远 readyState=OPEN；无人监控 `wsLastActivityAt`；
   服务端 30s 应用层 ping 停止后客户端毫无反应 → 全部实时域静默冻结（Docker NAT/conntrack
   回收、容器重启无 RST、睡眠唤醒均触发）。httpBridge.ts:779。
2. **ensureWs 状态机漏洞**：最后一个订阅者退订把 `wsReconnectAttempt` 清零但不关 socket；
   零订阅窗口内的 close 不调度重连；重新挂载后以 wasReconnect=false 打开 → `ws.reconnected`
   不触发 → 丢帧窗口不可见。httpBridge.ts:951-964。
3. **wsMappedEmitter 变换异常被空 catch 吞掉**：一个字段差异 = 该事件流 100% 静默失效，
   无任何日志。httpBridge.ts:883-890。
4. **无 visibility/focus 兜底**。
5. 旧桥 browser.ts 第二条 /ws socket：1008 后永久停止重连。

### 服务端（Rust）
6. **WS 连接钉死握手时的 JWT**：heartbeat 每 30s 重验原始 token（manager.rs:280），
   Cookie 滑动续期不更新它；原 token 过期 → 服务端发 auth-expired + 1008 →
   客户端清 auth 状态强制登出（会话实际有效）。
7. **用户事件总线 lag 只补偿 browser inventory**：forward_user_events 丢弃的
   turn.completed / message.stream / conversation.listChanged / terminal.* 没有任何
   恢复信号（routes.rs:80-99 只发 browser.inventory.changed）。
8. **PER_CONNECTION_BUFFER=64 + try_send 断连无重放**：流式突发在慢链路（Docker LAN）
   撑满 64 槽 → 服务端主动断连"以便 durable resync"——但 resync 依赖的客户端恢复（上面 1/2）是坏的。

### 渲染层商店覆盖缺口
9. `ws.reconnected` 有恢复：消息窗口、终端网格、终端会话列表、浏览器 inventory。
   **无恢复**：会话列表 store、requirements、cron、会话 artifacts、待确认权限卡。
10. **转录刷新只依赖 WS turn.completed 帧**（hooks.ts:1153）：HTTP 兜底轮询
    （reconcileConversationTurnAfterStreamTerminal）只降 spinner 从不拉消息 →
    截图完全吻合：spinner 结束、回复不渲染、F5 后出现。

## 统一机制设计

**一个客户端恢复信号：合成事件 `ws.reconnected`**，由四类触发源统一供给：
- A. close→open 重连（修复 gap-flag：close 一律置位，open 时置位即触发，不再依赖 attempt 计数）；
- B. 活性看门狗（>75s 无入站帧即强制回收 socket → 走 A）；
- C. 页面回到可见且 socket 死/陈旧（立即回收 → 走 A）；
- D. 服务端 lag 广播新增通用 `sync.resync-required` 帧 → 客户端收到后本地触发 `ws.reconnected`
  （复用同一恢复管道，无需 socket 重建）。

**服务端语义修正**：
- heartbeat 发现握手 token 过期 → 关闭码改用 **4409 TokenAged**（连接级、非认证级），
  客户端仅重连（用滑动续期后的新 cookie 重新握手）；真正的会话失效由握手 1008 权威判定。
- 用户总线 lag → 除 browser.inventory.changed（兼容）外广播 `sync.resync-required`。
- 缓冲扩容：PER_CONNECTION_BUFFER 64→256，用户/实例总线 256→1024（降低断连频率；
  断连后的恢复由上面机制兜底）。

**商店层**：把缺失的 5 个 store 接上 `ws.reconnected`；
转录窗口新增本地事件 `conversation.turn.settled`（由 reconcile onIdle 发出）触发重拉，
使 WS 帧全丢时 HTTP 轮询也能带回回复内容。

桌面（Tauri loopback WS）与 WebUI 共用同一 httpBridge 代码路径 → 语义天然一致。
