# UARC-032 Creation、Workshop、Office、Attachments 与 Model Role 实施记录

> Wave 2 barrier：`67417aa08`
> Wave 3 实现提交：`8454133b124a3d38636f882b098de80ce17028c0`
> 平台：Windows 实现与验证；macOS shared compile/UI 仍待 Mac 主机

## 交付

- 将 19 个单动作 Capability 收敛为 `creation.media`、`creative.workshop`、`office`、
  `plugin.development` 四个产品 Module 与 19 个 exact Actions。
- Creation Agent 输入物理移除 provider/model/model-selection；产品 Action 解析宿主 model route，credential、
  protocol 与 provider operation code 不进入 Agent contract。稳定任务 identity 包含 exact Action ID。
- Workshop 的 Canvas、asset、template run 继续由真实产品 owner 执行；Office 新增 asset-library owner，提供
  bounded preview 与 document/sheet/slides revision 创建，并验证来源资产、DTO、内容边界与持久化结果。
- Plugin development read/edit/publish/serve 绑定 exact Plugin resource 与 action-specific operations；
  current-turn attachments 仍是 Session input，不再提供持久 `session.attachments.read` grant。
- Creative Studio selector 只展示用户可理解的服务/模型名称，隐藏 raw model ID、协议和 provider UUID；
  loading、empty、unavailable、error、accepted/progress/result 状态完整，并支持 880×600 窗口及窄 pane。
- Conversation 与 canonical AgentSession 的 slash discovery 使用两个显式入口：现有 Conversation ID 保留
  owner/restricted/Skill-extra 校验；canonical ID 从新 Store 的 immutable Binding 编译并验证 typed resources。

## 物理删除

- 删除媒体 provider operations、`llm.*` 媒体能力与 `session.attachments.read` 的 Agent grant。
- 删除 Creation 请求中 provider/model 选择字段，以及 19 个旧单动作 Capability ID。
- 删除 Office “只登记、无生产 owner”路径；没有恢复旧 Runtime/Capability projection。

## 验证

```text
cargo test -p nomifun-agent-domain-wave3 --lib -- --test-threads=1
  13 passed
cargo test -p nomifun-office --lib -- --test-threads=1
  89 passed
cargo test -p nomifun-app --test official_preset_catalog_integrity \
  cold_skill_commands_use_saved_binding_without_starting_runtime_or_context \
  -- --test-threads=1
  1 passed
cargo test -p nomifun-app --test auxiliary_e2e slash_commands_no_active_task \
  -- --test-threads=1
  1 passed
cargo check -p nomifun-app --features browser-use
cargo test --locked -p nomifun-app --test nomi_core_live_provider_smoke --no-run
  passed

bun test <Creative Studio + slash focused files>
  23 passed
bun run typecheck
bun run check:desktop-ui-boundary
  passed；1931 renderer sources，minimum 880×600
```

`target_inventory check`、`agent-v2-contract check`、live-provider runner self-test、UARC boundary self-test 与
`git diff --check` 均通过。880×600 与 390px 内部 pane 已完成视觉检查。

商业模型验证仅使用 StepFun Coding Plan 的 `step-3.7-flash`：直接 provider probe 返回 HTTP 200 且包含有效
choice；密钥仅通过瞬时环境/stdin 注入，未写入文件、日志或提交。集成 `--model-smoke` 已编译，但 canonical
`start_turn` 当前只持久化 `turn/started`、尚未由 UARC-051/052 接入统一 Runtime dispatch，因此该链在等待
模型回复时超时，未记作通过，也未用免费模型替代证据。

## 平台状态

- Windows：verified（实现、UI、编译与 scoped gates）。
- macOS：pending；Creative UI、Office 文件交互与 shared compile 仍须在 Mac 真机验证。
