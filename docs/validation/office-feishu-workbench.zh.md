# 飞书工作台：文档基础管理验收记录

对应[现役实施计划](../handoffs/office-feishu-workbench.zh.md)。
验证日期：2026-10-26。按用户本次要求移除 NomiFun 内部文档阅读、预览和编辑。

## 本次范围

- 文档页保留云空间根目录列表、关键词搜索分页与类型筛选；根目录不代表全部云空间。
- 文档名、浏览器打开按钮和粘贴链接都交给浏览器，不再读取 Markdown 正文或原生 blocks。
- 新建只填写标题；创建空白文档后回到列表，在浏览器编写正文。
- 删除显示具体标题和链接并等待确认；只允许确切云文档类型和 token，不处理知识库、文件夹、快捷方式和 Base 删除。
- 删除按需申请精确权限，沿用冻结身份、请求幂等、提交栅栏和原回执；未知结果不重发。
- 成功创建／删除的最近入口变化与原始回执原子提交；历史未知文档操作仍阻止同资源新删除。
- 删除详情／编辑组件、原生文档适配器、正文 DTO、样式和旧验收用例，替换为基础管理测试和离线验收页。
- UI/API 从 HEAD 47 升为 48；不改数据库结构或 Agent Session。
- 日程、任务、独立多维表格页、本地 Office 文件预览和渠道功能保留原合同。

## 自动验证

| 检查 | 本次结果 |
| --- | --- |
| `cargo test --locked -p nomifun-office-hub --lib -- --test-threads=2` | 58 passed；3 个子进程夹具入口按原标记 ignored；20.21 秒 |
| `cargo test --locked -p nomifun-office-hub --lib adapter::tests::` | 最终权限与删除回执调整后 15 passed |
| `bun test --cwd ui src/renderer/pages/officeHub` | 44 passed，265 assertions |
| `bun run typecheck` | 通过 |
| `bun run check:desktop-ui-boundary` | 通过，最小支持视口 880×600 |
| `bun run check:i18n` | 通过，双语与生成类型一致 |
| `bun run check:docs` | 通过 |
| `bun run build:ui` | 通过，25.85 秒；manifest 为 UI/API 48 |
| `git diff --check` | 通过 |

覆盖：真实 CLI 列表参数及权限备选、标题专用创建、删除精确终态／异步未完成回执、链接校验、
未知结果不重发、既有跨身份目标栅栏、回执与最近入口原子提交，以及已有快照和 Agent 清理保留测试。
前端覆盖浏览器打开、知识库链接、列表与搜索结果分页、取消删除、授权返回、待核验禁用与创建后清空旧筛选。
数据库结构、备份格式和 Agent Session 未修改。

## 浏览器验收与外部证据

[离线验收页](../../ui/test/office-hub-browser.README.md)已更新为列表、搜索、创建、删除与链接打开。
它导入实际 renderer，业务边界和浏览器跳转均显式模拟，不访问真实飞书账号或云端文档。

已在 Codex 内置桌面浏览器的 1280×720 视口执行：

- 点击文档名只记录原浏览器链接，写入数保持 0，列表仍显示。
- 删除确认显示标题和 URL；取消后写入数仍为 0。
- 确认删除后该行消失，写入数为 1。
- 新建仅有标题输入；创建后回到列表并显示新文档，写入数为 2。
- 关键词搜索只显示匹配标题，清空后恢复列表。

以上为真实浏览器渲染配合模拟业务；本次未补 880×600 浏览器截图，桌面静态边界检查已通过。

本次没有真实租户 OAuth、管理员审批、资源 ACL、云端列表或删除的执行证据。
当前既有桌面进程未被终止或替换；源码修改和前端构建不等于安装了新桌面发行包。

## 官方合同

- [CLI 1.0.98 Drive 列表](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-drive/references/lark-drive-files-list.md)
- [CLI 1.0.98 删除与终态回执](https://github.com/larksuite/cli/blob/v1.0.98/shortcuts/drive/drive_delete.go)
