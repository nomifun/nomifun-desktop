# Skill 冻结发布与多 Engine 消费（2026-09-14，未验证）

所有源码修改位于本地 `rf/agent-capability-platform-v2`。没有 commit/push，也没有运行
构建、测试、评测、E2E 或外部服务；局部 rustfmt 只是格式整理，不是功能验收。
当前 Coding `host2-coding-loop27`，Nomi `host12`。整体 CAR 仍进行中。

## 发布与产品入口

Skill Library 详情增加“预览冻结文件 → 确认生成候选”。仅接受逻辑来源和 Skill 名称，
不接受客户端任意路径、字节包或引擎可执行文件。接口沿用 Plugin 本地产品信任、实例 owner
及 headless installation admission 边界，不直接暴露给模型工具。

预览返回源摘要、预期 artifact 摘要、完整源文件清单、每文件摘要、library revision 和
同包已有 Project。提交重新捕获并核对两个摘要；已有 Project 必须显式指定目标及 revision。
服务复用 PluginArtifactStore scanner、manifest 校验、content-addressed 发布和现有
Project/ready-candidate 流程。无自动 apply、enable、Agent 修改或 Session 迁移。

后续用户操作仍是：Plugin 工作坊查看／应用／启用 → Agent 工作台选择 Skill → 发布新 revision。
原 Session 保留其 exact binding，不会跟着可变 Skill Library 文件或新候选变化。
源内容相同但宿主打包规则改变时，artifact 摘要可不同，仍须用户重新确认。

候选是固定 no-op JavaScript 入口加 Skill 资源的 Plugin Package v1。它没有 capability、
credential、service、依赖声明，不执行 Skill 脚本／hooks／frontmatter 配置。
现有 Plugin v1 启用仍依赖受管 Node；UI 明示此限制。Skill 中写了工具要求不等于获得权限，
用户仍须在 Agent 里选择对应能力。这是资源包发布，不是打包后加载 Engine；社区 Engine
仍只能源码／依赖接入并重新打包应用。

## 源与资源预算

- 拒绝链接／junction／reparse 点、特殊文件、不安全路径、大小写冲突、超深目录。
- 最多 256 个目录项、8 层资源目录、SKILL.md 加 64 个文件；资源相对路径最多 128 UTF-8 字节。
- SKILL.md 最多 16 KiB；其他 UTF-8 文本单个 256 KiB、合计 512 KiB。
- PNG/JPEG/WebP 最多 4 张、每张源文件最多 4 MiB、所有捕获文件合计最多 8 MiB。
- 图片经既有有界解码／重新编码；准备后的单张 base64 最多 2 MiB、合计 8 MiB。
- 不支持的非 UTF-8 非图片资源明确拒绝，不静默丢弃；脚本只是文本。

两路捕获工作槽，发布接口另有单路串行限制，忙时明确拒绝。捕获任务取消后仍持有自己的槽
直到退出；后续 artifact import 的后台任务可能在 HTTP 取消后完成，不能把此限制宣传成
所有后台 IO 的全局硬并发上限。读前／读后 metadata、规范路径和摘要核对不等于原子文件系统
快照或完整 handle-relative TOCTOU 防护。并发修改可能导致拒绝，需重新预览。

复用的是现有仓库的 revision 检查与候选记录语义，没有新建跨 Project/library 的原子事务。
制品入库之后若后续 CAS／数据库操作失败，可能留下未被引用的 content-addressed 制品或
尚未完成的 Project；这不代表启用成功。断连后的提交结果可能不确定，UI 不自动重试，
应先到工作坊查看结果。现有导入 operation 与 candidate 落库顺序也未在本轮改造成事务。

## 共享数据层，各自上下文策略

原 coding_skills 的读取实现抽为 `engine_skills`，两个官方 Engine 使用同一 exact
CompiledSnapshot／registry generation／packaged contribution／artifact inventory／摘要校验。
最多 16 个已选 Skill，正文合计 24 KiB，资源合计 64 个；多 Skill 的聚合预算可能比单包预算先触顶。
缺制品、锁不一致、来源或依赖能力不匹配均失败，不回退到可变目录／最新版本。

`nomifun-engine-core` 提供纯数据 `EngineContextResource`／`EngineContextContent`。
Coding 旧公开类型保留为别名，其单调用图片工具与上下文压缩策略不变。
`EngineSessionHost.read_selected_skills(admitted_session)` 对社区 Engine 提供相同的只读结果，
公开类型为 `SelectedEngineSkills`；只能从该 host 认可的 Session 获取，不授予新能力。
引擎自行决定资源索引、规划循环、分页策略和多模态准入，平台不强制社区使用 Coding planner。

Nomi 新增：

- 冻结正文和资源索引进入初始系统上下文；与已有 capability context 合计最多 64 KiB。
- 仅资源非空时注册 `nomifun_skill_resource`，并纳入原子批次注册／allowlist／路由冲突检查。
- 文本支持 UTF-8 字节分页和 `next_offset/eof`，二次 JSON 编码不超过 24 KiB；无任意路径读。
- 图片使用共享 host-prepared pixels。只有 factory 明确认可的 exact model image 支持和
  当前 `llm.vision` activation 同时具备才返回；按需 vision 激活复用既有 AtomicBool。
- 图片通过 Nomi 原有 ToolImage／provider／artifact 链路处理，不自行复制 Coding 历史格式。
  初始能力检查不等于每次真实 provider 接受图片；动态降级、压缩和重放仍服从原 Nomi 路径，
  尚无本轮真实模型运行证据，不能由资源读取成功推断模型已看见图片或任务已完成。

同时修复 `NomiPluginToolSession.with_context_contributor` 重复定义，统一为有 64 个上限的
Result 接口，并让生命周期装配调用传播失败；不丢弃已装配的 Robot／MCP contributors。

## 仍未完成

全产品生态消费者继承、更多 MCP transports/resources/交互生命周期、MiniApp／Robot 的
完整效果结算、跨启动进程证明／人工安全恢复、任意 shell 路径指令覆盖，以及独立任务语义
验收仍不因本轮完成。其他二进制 Skill 资源与自动脚本／hooks 执行不在本次支持范围内。
最新改动没有编译或运行结果；历史测试结果不能覆盖此切片。
