# R8 审计记录：运行时协议身份、交付与错误隔离

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)，保留 R1–R7 未提交改动。

## R8-01：已复现问题

核对 supervisor::handle_frame、respond_to_service、服务 JoinSet、出站队列完成路径，以及
plugin_n1 的请求/响应验证契约。使用合法握手的真实 Node 协议夹具，4 个新增测试在原实现全部失败：

- Shared Host 接收 candidate_test 角色的服务请求，进入了真实 handler；协议值本身合法不等于绑定当前实例。
- 两个仍在途的相同服务 request_id 都执行了 handler。
- 无效成功响应在 validate_for 前就移除了 pending，调用方仅收到 RequestChannelClosed，丢失整代失败原因。
- 公开失败状态暴露四类不可信正文：未知响应 request_id、非法版本字符串、超时服务 ID、HostShutdown 拒绝消息。

## 修复与清理

- 服务准入先比较当前 role + generation，再执行现有契约和 Mount 绑定校验，错误角色不会进入 handler。
- 记录尚未完成的服务关联 ID；直到响应全部写入并 flush 后才释放。出站队列返回完成关联，
  不在 handler 一完成就提前移除，覆盖响应背压窗口；重复在途 ID 通过正常代际失败取消现有任务。
- 响应先用仍保留的 pending 验证；只有成功验证后才取走等待者。协议失败和清理失败时由 fail_all
  向原始/合并等待者统一交付 HostFailure；删除清理失败分支中不再需要的重新插入。
- HostShutdown 已由公共契约保证成功只能为 Ack，因此删除重复的非 Ack 回退校验；
  请求级拒绝仍保留给发起方，但公开代际状态使用固定原因，不传播任意拒绝文本。
- 未知响应、请求/响应契约失败、服务超时也使用固定安全原因；不在公共错误中引用 peer 提供的 ID/版本。

没有引入所有历史 ID 的无限 tombstone 集合。此处保证的是未完成服务关联不重复；响应已经本地
发送完成后的历史重放不被宣称为持久化 exactly-once。协议/业务若需要持久防重放，须另外定义契约。

## 验证

本批新增 6 项：5 项 Node 集成、1 项分段写入关联单元；4 项有红→绿证据。
补充验证停止响应也能交付整代失败、服务关联仅在完整帧写完后返回且只返回一次。

- 初始协议回归：修复前 4 失败；修复后 4/0。
- cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel：105/0
  （Kernel 32、Host 单元 10 + 集成 59、adapter 单元 2 + 集成 2）。
- 进程边界、Node 夹具语法和 git diff --check 通过。
- 未改 UI，未跑全 Rust workspace、桌面包或其他操作系统。

## 接续

R9 检查在途请求/服务配额：有界命令/出站队列并未自动限制已经送达后长期等待的 pending、
合并等待者或 handler。还需确保耗尽普通工作配额后仍可取消，而不是把控制路径一起堵死。
Host 的文件发布/路径、清理证明失败后的重启策略和 JS 其余边界仍待审；不标记整个 Host 完成。
未提交、未推送，既有 .githooks/ 保持不变。
