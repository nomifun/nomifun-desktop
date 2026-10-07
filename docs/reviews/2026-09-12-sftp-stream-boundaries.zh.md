# R27：SFTP 底层包长与背压取消

本批只补齐 R24 的底层传输边界；不代表整个 SSH 模块完成。

## 发现与红→绿证据

- R27-01：锁定的 russh-sftp 2.4.0 客户端使用 read_packet(stream, u32::MAX)，没有应用 Config 的默认 256 KiB 包长限制。读取包头后直接分配 payload，应用层文件/目录限额不能约束它。
- R27-02：依赖写任务在 select 分支内等待 write_all/shutdown；关闭帧排在写入之后，取消不能中断已阻塞的写操作。
- 修改前两项回归均在 200ms 观察期限失败：永久 Pending 写入时 Drop Session 不释放底层流；仅发送 512 KiB 长度头仍等待正文。后者没有尝试多 GiB 分配。

## 实现

- 新增私有 fs_stream，复用 LengthDelimitedCodec，在依赖分配前检查 256 KiB payload 上限；保留原始四字节头及正文，通过 StreamReader 提供原来的字节流接口。
- Session 持有取消 DropGuard；构造失败/取消同样触发。取消唤醒读端及已 Pending 的 write/flush/shutdown，不新增生产后台任务，不修改依赖源码。
- raw 字段先释放、随后 guard 取消：兼容正常关闭请求，同时不再依赖关闭帧越过背压。
- nomi-ssh 使用工作区已有 tokio-util 的 codec/io/rt feature，没有升级依赖版本。

## 验证

- 两项旧实现失败测试均修复通过；另测已阻塞 shutdown、精确 256 KiB payload/连续帧/分段包头保真，以及上限 +1 立即 InvalidData 后 EOF。
- cargo test -p nomi-ssh --lib：40/0。
- cargo test -p nomifun-ssh --lib sink:: -- --test-threads=4：10/0。
- git diff --check 通过。仅格式化审计新增文件；UI 未修改，不重复 UI 全量测试。
- 日志位于临时验证目录 nomifun-review-r18-2EEsPY：sftp-stream-before.log、sftp-stream-final.log、ssh-consumer-r27.log。

## 限制与续接

- 帧包装会缓存一个受限帧，依赖仍有其受限 payload 分配；不是零复制或总内存只占一份 payload 的承诺。
- 测试保持 peer 活着并观察底层流实际 Drop；这证明本地流释放，不证明远端进程死亡。真实 russh ChannelStream Drop 使用依赖异步 channel-close，不能据本地 Drop 推断对端已确认关闭。
- R24 的临时文件残留、已发送 rename 后结果未知、权限/持久性限制仍然适用；没有真实 OpenSSH/Linux/macOS 集成或网络吞吐基准。
- 下一检查点仍为 R26-02 目录参数/环境语义、R26-03 shell 生命周期及后端 R25-02/连接池/服务等。
