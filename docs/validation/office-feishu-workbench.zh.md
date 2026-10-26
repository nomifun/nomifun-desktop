# 飞书工作台：文档编辑验收记录

对应[现役实施计划](../handoffs/office-feishu-workbench.zh.md)。
验证日期：2026-10-26。下述结果区分本地自动验证、真实浏览器与真实飞书租户。

## 根因与修复边界

截图显示进入编辑模式后没有输入框、保存不可用。代码审计确认三项可复现的设计缺陷：

1. **状态与能力脱节**：只要文档与身份存在就能进入编辑；即使所有段落均只读，仍显示“正在编辑”，没有原因或选择提示。
2. **读写模型不对称**：XML 纯文本先被转义为 Markdown，又直接作为输入值；保存允许生成行内格式，但下一次 XML 解析把任何行内节点标为只读。一次编辑可以使同段后续无法编辑。
3. **替换粒度错误**：旧保存通过 `block_replace` 重建段落，使旧 block ID 失效，并需要额外的评论 XML 保护逻辑。

浏览器验收还确认：共享 Markdown 组件会把标准 Markdown 的 `\[文字\]` 转义误当 LaTeX，导致保存后阅读失真。
文档阅读现明确关闭聊天专用的 LaTeX 分隔符归一化，保持标准 Markdown 转义语义。

没有读取截图对应真实文档，不能仅凭截图断言该段具体含有哪些原生属性。
此次修复针对已经复现的架构问题，不把离线验收计为真实租户成功。

## 当前架构

- 业务访问仍全部依赖官方 `@larksuite/cli@1.0.98`；未增加宿主 HTTP SDK、凭据副本或另一套安装器。
- 阅读沿用官方 Markdown 导出。编辑先由 CLI 解析文档链接／wiki 定位，再读取真实 metadata 与 blocks。
- 原生分页固定同一 revision；拒绝重复 cursor、缺失块、重复 ID、不一致父子关系、超过 100 页或 10,000 块。
- 编辑值是原始文本。普通顶层段落通过 `update_text_elements` 原地 PATCH，保留 block ID 和段落样式；确认空正文后才创建首段。
- 标题、行内富文本、评论、嵌入资源和嵌套结构显示只读原因，不通过扁平化或重建全文绕过限制。
- 移除 XML 编辑解析器、评论 XML sidecar、Markdown 字面编码与段落替换路径，并移除该 crate 的 quick-xml 依赖。
- 身份、原 revision、请求幂等、原始回执和整篇文档 pending 围栏保留。未知结果不重发，也不把历史 Markdown 回执解释为新的原生 PATCH 成功。
- UI/API 合同从实际 HEAD 的 46 升为 47，阻止旧界面继续发送旧编辑语义。Agent Session、数据库和操作账本均未改变。
- 能力检查遵循官方 CLI shortcut 的精确 scope 预检；不以 OpenAPI 可接受的较宽 scope 推断整个工作台链可用。

## 自动验证

| 检查 | 本次结果 |
| --- | --- |
| cargo test --locked -p nomifun-office-hub --lib service::tests:: | 24 passed，94.47 秒 |
| cargo test --locked -p nomifun-office-hub --lib adapter:: | 21 passed，含 8 项原生文档契约测试和精确权限合同 |
| bun test --cwd ui src/renderer/pages/officeHub src/renderer/components/Markdown/streamingMarkdown.test.tsx | 55 passed，339 assertions |
| bun run typecheck | 通过 |
| bun run check:desktop-ui-boundary | 通过，包含最终文档解析调整 |
| bun run check:i18n | 已通过 |
| bun run check:docs | 通过 |
| bun run build:ui | 通过，25.70 秒；manifest 为 UI/API 47 |
| git diff --check | 通过 |

原生契约测试覆盖默认样式、纯文本／UTF-16 长度、换行和特殊字符、空段落与空正文的区分、全树完整性、PATCH／POST 请求体和严格保存回执。
服务测试覆盖固定版本分页、坏基线拒绝、连续编辑保持同一块、首段插入后再编辑、版本冲突、超时后保留单次 attempt 与写保护。
前端测试覆盖点击正文后焦点、全只读说明、原始输入、授权返回、失败重读及冲突后保留输入并重新选择目标。

未修改 Store、DB/schema、备份或 Agent Session，因此未重复运行这些域的全套检查。

## Windows 真实浏览器与离线业务边界

使用[可重复运行的验收页](../../ui/test/office-hub-browser.README.md)，Vite 仅监听 `127.0.0.1:5191`。
该页面导入实际 renderer，业务调用全部显式标记 QA SIMULATED，未访问后端、真实飞书账号或云端文档。

已实际执行：

- 880×600：进入编辑显示选择提示，点击正文后输入框自动获得焦点；中文、Markdown 标记、Windows 路径与换行按原始文本保存。
- 保存后再次进入编辑，同一测试 block 与原始内容仍可编辑，footer 仅记录一次写入。
- 全只读场景明确显示“只读”及具体原因，保存不可用。
- 确认空正文后创建首段，保存后可再次编辑；1200×800 宽桌面布局与焦点正常。
- 标准 Markdown 方括号阅读已再次验证：`第一段 · 原始文本 # * [内容]` 原样显示，实际 Shadow DOM 中 `.katex` 节点数为 0。

## 外部证据与限制

原生 API revision 参数允许操作历史版本，并不是跨客户端的严格 compare-and-swap。
保存前检查最新版本可以拒绝已知冲突，但检查与请求之间仍有其他客户端并发修改的窗口；不承诺排他编辑。

本次没有真实飞书租户的 OAuth／管理员审批、资源 ACL、原生网络调用或云端写入证据。
复杂富文本和完整协同编辑仍由飞书客户端提供。其他操作系统没有本次原生交互证据。
当前运行的既有桌面进程未被终止或替换；前端构建与源码变更不等于已经安装新的桌面发行包。

## 官方契约

- [CLI v1.0.98 raw API](https://github.com/larksuite/cli/blob/v1.0.98/cmd/api/api.go)
- [CLI 输出响应封装](https://github.com/larksuite/cli/blob/v1.0.98/internal/client/response.go)
- [CLI 精确 scope 预检](https://github.com/larksuite/cli/blob/v1.0.98/internal/auth/scope.go)
- [文档元数据](https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document/get)
- [固定版本块分页](https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document/list)
- [原地更新块文本](https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document-block/patch)
- [原生首段创建](https://open.feishu.cn/document/server-docs/docs/docs/docx-v1/document-block/create)
