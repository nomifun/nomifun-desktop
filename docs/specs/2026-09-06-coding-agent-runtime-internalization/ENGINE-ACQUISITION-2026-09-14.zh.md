# 冷构建任务所有权（slice86，未验证）

本地 `rf/agent-capability-platform-v2`，未 commit/push；未运行测试、构建、服务、
模型或迁移。只做源码阅读/实施和两个小模块的局部 rustfmt。

## 修复的问题

Slice85 关闭屏障等待构建持有的准入读锁，但 get/create 内层 future 仍由调用者
直接驱动。若请求 task 被丢弃，工厂可能中途丢弃且释放读锁/每 Session gate，
留下空 OnceCell；之后空槽不能证明工厂没有生成进程或其他资源。

## 已写入的处理

- 普通获取、preparation 和 turn 三个生产入口统一使用 `acquisition::run`。
  开始前取得 owned 准入读锁，随后无 await 地将锁和构建 future 交给 Registry
  持有的任务。调用者只等待 oneshot，不获得 abort handle。
- 每个等待者使用父取消 token 的 child；丢弃等待会取消该 child，不取消父 token
  或其他等待者。已开始的工厂继续被驱动到结果，复用原构建取消/精确清理逻辑。
  未开始的获取可直接拒绝，普通显式取消仍等待现有清理结果。
- 全局 acquisition guard 在异常退出时先设置 uncertainty 和永久关闭准入，再
  释放读锁；每 Session gate 内另有 unwind fence，确保 panic 标记早于 gate 释放，
  避免同一 Session 的后继获取抢先把空槽当作无资源状态。
- 已关闭状态在取得 Session gate、配置解析后及槽复用循环中重查；新构建在
  异步模型配置确认完成后再次检查取消/关闭，必要时走原 exact-slot teardown。
- Registry 存在异常 acquisition 时，空槽清理拒绝释放槽和 workspace lease；
  正常已初始化实例仍可通过原清理证明退出。进程级关闭最后检查 uncertainty，
  不能因各映射暂时为空而宣称成功。没有自动清除此不确定状态或重放工厂。

原有 single-flight、每 Session gate、turn generation、workspace lease、model
binding、Engine exact binding 与工厂内 Plugin task-local 装配仍由原 Registry 处理。
不是新的 Session 协调器，也不改变 Nomi/Coding/社区 Engine 的循环策略。

## 重要边界

1. 工厂正常返回 Err 仍必须自行结清部分资源；它不能用 Err 隐藏尚存的进程。
   Panic/任务异常退出不按普通 Err 处理。panic=abort/进程强制终止不可能由此模块
   转为本进程的安全退出证明。
2. 结果投递的最后一步若与等待者丢弃竞争，成功实例仍留在精确登记槽中；不会
   因接收者丢失就按 Session ID 杀掉可能已被后继采用的实例。原 owner teardown /
   进程关闭继续拥有该槽。这不意味着丢弃结果已完成 turn admission 或持久回执结算。
3. 永不返回的构建仍可能使关闭 pending；宿主超时只能保留资源，不能释放数据库
   或声称无副作用。异常 acquisition 的关闭是保守拒绝，不提供新的人工解除入口。
4. 将构建移到拥有的 task 不保留任意调用者私有 task-local；正式工厂依赖应通过
   options 和已准入端口显式传递。生产 Nomi Plugin scope 仍在原工厂调用内装配。

官方源码身份为 Coding `host2-coding-loop86` / Nomi `host56`，摘要包含新 acquisition
模块。未取得取消/并发/锁顺序/panic/丢弃等待者/真实资源退出/跨平台的运行证据。
本切片补齐 slice85 的调用者丢弃缺口，不代表整个 Engine 或 CAR 已完成。
