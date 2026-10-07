# R9 审计记录：在途容量与取消保留名额

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。未提交，保留 R1–R8 改动。

## R9-01：证据与修改

有界 command/outbound 队列并不限制已送达请求。审计了 Actor Submit、MountLoad 合并、
响应完成、服务任务创建/取消及响应 flush 的容量归属。
先声明配置字段供回归编译，但未加入准入限制；三项真实 Node 回归全部失败：

- 工作上限 1 时，已有悬挂调用后仍成功执行 echo。
- 工作上限 2 时，5 个同配置 MountLoad 都被合并等待，无一返回 QueueFull。
- 服务上限 2 时，10 个不同关联的请求全部进入 handler。

现实现 max_pending_requests / max_service_requests（默认各 256，必须非零）：

- 普通请求数量包含合并等待者，从实际 pending/reply 推导，不增加多套可漂移计数器。
- 驻留 Mount 的幂等 Ack 不占新名额；满额只拒绝尚未发送的调用，不关闭整代。
- RequestCancel 有一个独立在途名额；普通请求满额仍可取消，取消悬挂也不能无限累积。
  普通工作与取消相互不占额度，但仍受原有命令/出站队列和截止时间约束；不承诺堵塞管道时能强制发送取消。
- 服务配额覆盖 handler 执行到响应完整 flush，复用 R8 的关联集合。超量在进入 handler 前
  使代际失败并取消已有任务，避免为拒绝响应另建无界队列。
- QueueFull 错误文本统一为请求容量；测试的 Candidate Host 重复配置改为复用 host_config。

此处限制的是 Host 拥有的在途结构，不宣称限制任意插件 JS 内存、驻留资源总数或所有等待调用方的内存。

## 验证

- 初始定向筛选 1 旧测试通过、3 新测试失败；实施后 4/0。
- 另加取消名额独立且有界、零配额拒绝，共新增 5 项回归。
- cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel：110/0
  （Kernel 32，Host 单元 11 + 集成 63，adapter 2 + 2）。
- Node 夹具语法、git diff --check 通过。进程边界检查首次前台调用超时，后台重跑通过。
- 未改 UI，未跑全 Rust workspace、桌面包或 macOS/Linux。

## 接续

R10 转入 Host 文件发布/路径：现有 digest 命名文件直接 create_new 后写入，须检查多调用方
并发发布能否观察半文件及 AlreadyExists。清理证明失败后重启和 JS 其余边界仍待审。
