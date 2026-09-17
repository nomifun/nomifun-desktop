# UARC-040 Browser 产品模型与共享 Resource/Provider 合同实施记录

> Wave 3 barrier：`5bec65d182d5798d5fa99398a39ae59a218bbba1`
> 平台：Windows shared/native composition 已验证；macOS CEF 生产闭环待 `UARC-060`

## 交付

- 一个 Browser Module 提供七个 exact Actions；Action grant、Provider availability 与 Resource operation
  三者独立，Provider/Resource 的存在不能生成 Agent authority。
- `BrowserResourceKey` 绑定 principal、AgentSession 与 resource binding；persistent profile 使用
  `browser-v3/agent-sessions`，managed 与 attached Chrome 通过同一 provider-neutral binder/Tool surface。
- 任意授权 AgentSession（含 delegated Session）可绑定 Browser；删除 Conversation-main-only admission 与
  dedicated session-browser identity。
- attached disconnect、inventory、run admission 与 invoke 共用 operation barrier；managed RunGuard 保持
  native input、dialog、download、permission 与 cleanup 的 exact generation 约束。
- canonical delete 从 frozen Browser bindings 重算 profile policy，先关闭 native runtime，再精确删除持久
  profile。重启后即使内存 map 为空也可删除；ephemeral 不触盘，foreign/symlink/junction/special entry
  均 fail closed 并可重试。Windows Desktop 在 composition root 注入同一 canonical `BrowserProfileStore`。
- 物理删除 `SystemBrowser`/`BrowserWorkspace` 旧实现、路由、测试、UI 控件与 `nomi_system_browser`
  capability surface；Windows 仍使用 WebView2，未恢复 WKWebView。

## 验证

```text
cargo test -p nomifun-browser-platform --lib -- --test-threads=1
  55 passed
cargo test -p nomi-browser-engine --lib -- --test-threads=1
  293 passed, 9 ignored explicit real-Chrome cases
cargo test -p nomifun-app --test browser_workspace --features browser-use -- --test-threads=1
  15 passed
cargo check -p nomifun-desktop --all-targets
cargo build -p nomifun-desktop
  passed
```

Windows junction profile-delete case 实际执行并通过；Unix symlink case 在 Windows 按 cfg 跳过。
`check-browser-platform-boundary`、desktop UI boundary、typecheck 与 UARC scanner 均通过。

## 平台状态

- Windows：verified；生产 Browser host 为 WebView2。
- macOS：pending；现有独立 CEF child NSView 尚须 `UARC-060` 接入该 Resource 模型并完成真机、CEF、
  lifecycle、DMG/签名结构证据，未声称跨平台完成。
