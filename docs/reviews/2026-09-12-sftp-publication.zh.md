# R24：SFTP 原子发布与通道归属

范围：nomi-ssh 的 fs.rs、新增 fs_session.rs、connection.rs 句柄共享，以及 nomifun-ssh/sink.rs 的文件调用与恢复路径。未把整个 SSH 模块标为完成；shell、池、路由/服务等另续。

## 发现与证据

- R24-01：旧实现任意 rename 错误后都 remove 原文件，永久重命名失败可丢失原内容；CREATE/TRUNCATE 临时路径可覆盖碰撞文件；忽略权限设置和关闭失败；普通 SFTP v3 覆盖写实际先删再改名。
- 已在修改生产逻辑前运行真实 russh-sftp 协议夹具：4 项全部失败。分别观察原文件消失、碰撞仍成功、权限失败仍发布、覆盖写删除目的文件。记录：临时验证目录 nomifun-review-r18-2EEsPY/sftp-publication-before.log。
- R24-02：依赖 read_dir 先累积服务器的所有批次，本项目之后的输出长度检查无法限制累积过程。已读取锁定版本 2.4.0 的真实实现，不把目录扫描当行为证据。
- R24-03：SFTP 初始化无总超时；读文件 metadata/read 分别计时；高层 API 无法发送 posix-rename，转 raw 后必须显式解决取消时未知/已知远端句柄的归属，调用方原来只转字符串，不会恢复文件通道。

## 实现

- 只保留一个 SFTP 通道和一个串行操作槽；每次取出会话，成功才放回。失败/取消时释放 raw session（背压中断后由 R27 补齐）；下一次操作从同一已认证 SSH handle 重建 SFTP。没有新增并存的第二通道、连接池重拨或自行脱管的清理任务。
- 一个 30 秒预算包含等待槽、必要的通道重建和所有协议请求；初始化也受限。目录逐批累计，包含字符串容器开销，超限即停止请求。文件仍有 8 MiB 内容上限，metadata 缺失时也逐块检查。
- 通过 EXCLUDE 独占创建临时文件，初始权限 0600；除 NoSuchFile 外不把 stat 失败视为新文件；保留既有模式位时必须成功执行句柄 FSETSTAT。
- 保留最多八个在途写请求，作用域内拥有所有 future；协商服务器 limits/fsync，所有写、权限、支持的 fsync、CLOSE 确认完成才发布。
- 宣告支持时发送 posix-rename@openssh.com；否则用 v3 rename 创建新目的路径。不能原子替换的服务器返回错误，绝不先删原文件。已确认独占创建的临时路径在普通失败时尽力 unlink；碰撞路径不清理。
- 删除 remove+rename 降级、吞掉权限/关闭错误的分支、分散的超时包装及高层目录全量积累。futures-util 已存在工作区和依赖图中，本批 Cargo.lock 仅增加 nomi-ssh 的直接依赖边，不升级外部包。

## 验证

- 最初 4 项回归：旧实现 0/4，修复后 4/0。
- 最终 cargo test -p nomi-ssh --lib：27/0；含 16 项新增协议测试，无 sshd 条件跳过。
- 新增测试覆盖：四个原始缺陷；v3 新建/禁止破坏性覆盖；写/CLOSE 失败；stat 拒绝；分块往返/单通道复用；空文件；失败和取消后服务器 EOF 与重建；总截止时间；目录早停；metadata 无 size 时内容上限；服务器 limits 和 fsync 成败。
- cargo test -p nomifun-ssh --lib sink::tests -- --test-threads=4：4/0，证明后端 trait/Send/调用接口兼容；不声称覆盖池并发。
- 新夹具初次扩展编译暴露 OpenFlags 没有 PartialEq，已改用 contains 后重跑。没有隐藏这次失败。
- 仅格式化新增 Rust 文件；git diff --check 通过。UI 未改，不重复前端全套测试。

## 边界与续接

- 后续 [R27](2026-09-12-sftp-stream-boundaries.zh.md) 已补齐卡住写入的取消及协议包分配上限，nomi-ssh 40/0。上述 R24 EOF 回归只覆盖无写背压场景；本地底层流 Drop 不证明真实对端确认关闭或远端进程死亡。

- 协议夹具通过内存 duplex 运行真实 SFTP 编解码，不接远端、不改真实远端文件；未运行真实 OpenSSH、Windows SFTP 服务端或 Linux/macOS SSH 集成环境。
- 取消/断链可以留下临时文件；完成权限恢复后的临时文件可能已有目标模式。若 RENAME 已发出但应答丢失，发布结果未知，可能是旧文件或完整新文件，不承诺取消即撤销。
- fsync 仅在服务器宣告支持时调用；并不承诺目录 fsync 或断电持久性。新文件 0600 是显式安全默认；仅保留 POSIX 模式位，不承诺 ACL/owner/xattr。
- 单个 RemoteFs 的独立文件操作串行化，文件内写入仍有八请求窗口；未做真实网络吞吐基准。
- 下一批继续 sink.rs 的 glob 命令生成（发现允许重定向字符，尚待生产入口回归）、shell.rs 以及后端 pool/service/routes 等。全局审计未完成。
