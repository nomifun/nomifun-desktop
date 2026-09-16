# Slice75：删除外部 Runtime 打包链与发布锁字段（未验证）

分支 rf/agent-capability-platform-v2。继续 CAR-08 的物理清理，不改变 Agent/Engine
职责、Conversation owner 或源码注册约束；不是 CAR-08 或三平台验收通过。

## 删除项

- scripts/desktop-build-mac.sh 删除外部 Runtime 路径/hello 环境变量读取、协议
  hello 校验、二进制架构校验/复制/暂存、打包后 sidecar 比较及 release-lock
  --sidecar 注入整条链路。旧 --with-codex-runtime（含 = 形式、透传位置）明确拒绝。
- 不再创建、清空或打包 target/nomifun-runtime；不删除开发者已有文件。
- apps/desktop/tauri.macos.conf.json 删除旧 runtime 资源映射，保留空平台 overlay。
  共享配置仍提供 UI、LICENSE、NOTICE。macOS 主程序架构检查、签名、公证、
  DMG 收集与发布锁生成保留。脚本显式 cd 到仓库根，减少相对配置路径歧义。
- 打包结果对已退役 executable/hello 文件名的拒绝不再受开关控制；包含同名
  symlink，Resources 根缺失/为 symlink 或遍历失败均失败。它只覆盖这些已知
  资源名称，不是任意重命名二进制或嵌入内容的鉴别工具。

## 发布锁 v2

scripts/release/release-lock.mjs 的 schema_version 改为 2.0.0，删除 sidecars
顶层字段、CLI 参数、枚举和摘要路径。构造器拒绝未知输入，包括旧 sidecars: {}，
避免默默忽略旧调用者希望声明的外部执行器。新格式只包含 source_commit、platform、
host、helpers、package、legal 与 schema_version。

helpers 保留为一般辅助制品（并非 Engine 装载接口），不能将旧 Runtime 改名
塞入 helpers 当作 CAR 合规。Host 摘要记录实际主程序字节；这次没有添加或伪造
Engine catalog、真实 suite、日志或端到端质量证据。

v1 锁显式拒绝，提示基于真实当前制品重新生成 v2；不自动剥离字段、不修改已存在
的锁文件、不把历史发布证据转换成当前 CAR 通过证据。新的 Windows 签名 RC 生成器、
C8 Host-only candidate 生成调用和 macOS fixture 调用已经去掉旧输入。
旧 C7/C8 sidecar 验证分支仍属后续删除项，不能拿来验证新的 Engine。

## 源码测试同步与未执行范围

现有 macOS 打包源约束测试改为检查旧入口已删除、资源映射为空及已退役资源拒绝。
发布锁测试的真实文件摘要对象改为普通 helper，保留缺失/变更检查，补上 v1 和
sidecars 字段拒绝断言。它们只是已修改的测试源码，没有运行。

未运行 shell 脚本、JS 检查、Bun/Rust 测试、构建、打包、服务、模型调用或迁移；
没有读取签名凭据、签名、公证、生成制品/发布锁、commit 或 push。

## 当前余项

旧 Wrapper crate、历史平台桥接/宿主和协议 schema 仍在 workspace，slice74 的
非默认特性只是前一步隔离，不替代 CAR-08 物理删除。共有 Wave1 测试需迁出旧宿主；
旧 gate/validation 的 sidecar 分支与证据字段仍需删除或迁移。平台能力边界和实际
执行验证缺口仍按 ENGINE-READINESS 清单处理。

本次仅修改打包和发布合同，没有改变官方 Engine 策略代码；当前标识保持 Coding
host2-coding-loop74 / Nomi host47。整体目标未完成。
