# 飞书工作台：已有文档列表根因与验收

对应[现役实施计划](../handoffs/office-feishu-workbench.zh.md)。验证日期：2026-10-26。

## 根因

1. 默认列表使用 `drive files list` 的空 `folder_token`，只能列根目录直接子项；这与用户的可访问文档集合不同，尤其遗漏知识库。
2. 列表额外要求 `space:document:retrieve`，而已连接账号原本具备的是 `search:docs:read`；用户有文档却无法进入旧列表。
3. 列表与搜索有两个 DTO、权限、CLI 命令和解析分支；默认列表没有分页，类型筛选仅作用于当前页，可能把全局存在的类型显示为空。
4. 分页令牌复用了最多 512 字符的资源 ID 校验。真实账号返回的合法游标为 537 字节，原搜索翻页也会被拒绝。
5. 读取失败与空状态同时渲染；测试主要依赖“根目录已有文档”的理想夹具，未覆盖真实知识库结构与长令牌。

## 最终架构

`Documents(query, filter, cursor)` → 精确权限检查 → 官方 `drive +search` → Search v2 → 严格元数据投影 → 列表。

- 空关键词浏览、关键词搜索、类型筛选和分页共用一条查询；权限统一为 `search:docs:read`。
- 类型通过 `--doc-types` 在服务端筛选，按 `edit_time` 排序；不增加 owner、时间、文件夹或知识空间限制。
- 删除旧根目录命令／解析器、独立搜索合同、重复权限、当前页类型筛选和 URL 充当资源 ID 的路径。
- token 来自 `result_meta.token`；知识库 DOCX 保持 `wiki` 资源身份，不开放误删。
- 原样传递不透明游标，校验与资源 ID 分离；拒绝坏分页，不用不可靠 total 宣称全部资源数。
- UI 明确区分加载、权限、错误和无匹配；查询条件或连接变化后不复用旧分页。
- 标题创建、删除回执、浏览器打开与正文不进入 NomiFun 的边界保持原合同。
- UI/API 48 → 49；没有数据库迁移、正文缓存、凭据副本或 Agent Session 变化。

## 真实账号只读证据

读取当前开发数据集的办公连接定位，不读取凭据文件、不登录或申请新权限。
现有 CLI 版本为 1.0.98，用户身份为 ready，已具备 `search:docs:read`。

| 路径 | 实际结果 |
| --- | --- |
| 旧根目录命令 | exit 3；`authorization / missing_scope`；缺少 `space:document:retrieve` |
| 官方 CLI 空关键词浏览第一页 | 20 项，`has_more=true`，下一页令牌 537 字节 |
| 同查询第二页 | 5 项，`has_more=false`，两页 token 无重复 |
| NomiFun 现有受监督运行器＋新适配器的显式只读探针 | 20＋5 项，投影为 23 个 wiki 和 2 个 Base |
| 服务端云文档类型筛选 | 第一页 20 项，全部 DOCX，仍有下一页 |
| 服务端 Base 类型筛选 | 2 项，全部 BITABLE，无下一页 |

探针仅读取状态和最多两页元数据；使用独立临时工作目录，不写真实云文档、不修改连接配置。
原始标题、链接、用户 ID、凭据和游标未写入验收记录。

可重复执行的探针为 `adapter::tests::live_document_discovery_read_only`，默认 ignored。
显式设置 `NOMIFUN_OFFICE_PROBE_CLI`、`NOMIFUN_OFFICE_PROBE_PROFILE` 后，
运行 `cargo test --locked -p nomifun-office-hub --lib adapter::tests::live_document_discovery_read_only -- --ignored --nocapture`。
该探针要求账号中已有文档；不会替用户连接账号或开通权限。

## 自动验证

| 检查 | 最终结果 |
| --- | --- |
| `cargo test --locked -p nomifun-office-hub --lib -- --test-threads=2` | 62 passed，4 ignored；20.54 秒 |
| 显式 `live_document_discovery_read_only -- --ignored --nocapture` | 1 passed；真实账号 25 项；2.20 秒 |
| `bun test --cwd ui src/renderer/pages/officeHub` | 移除独立 Base 页面后 46 passed，270 assertions |
| `bun run typecheck` | 通过 |
| `bun run check:desktop-ui-boundary` | 通过，最小支持视口 880×600 |
| `bun run check:i18n` | 通过 |
| `bun run check:docs` | 通过 |
| `bun run build:ui` | 移除独立 Base 页面后通过，25.15 秒，manifest 为 UI/API 49 |
| `git diff --check` | 通过 |

默认 ignored 包含三个原有子进程夹具入口和一个需显式账号的只读探针；后者已单独执行，不能把默认跳过计作真实账号证据。
自动用例覆盖真实 Search v2 元数据结构、Wiki 资源身份、原样长游标、坏分页拒绝、服务端筛选，
以及默认浏览翻页、查询／筛选／账号变更后的分页重置、游标循环与错误不冒充空结果。
既有创建／删除单次回执、pending 围栏、快照和 Agent 清理保留用例继续通过。

## 页面入口收敛

按用户确认，顶部仅保留工作台、文档、日程与任务、操作记录四个页签。
多维表格继续在文档中通过类型筛选、搜索和浏览器打开；删除独立 `BaseView`、编辑表单、
页面状态、字段转换辅助函数、专用样式和文案，以及已退役页面的交互测试。
连接向导不再提供独立的表内数据读取权限选项。历史操作记录与后端回执核验保持原合同。
现有 Base 列表交互测试确认顶部没有重复页签，且筛选仍提交 `filter=base` 并打开原浏览器链接。

## 浏览器范围与限制

[离线验收页](../../ui/test/office-hub-browser.README.md)导入实际 renderer，
业务与浏览器打开均为模拟；补充 25 项无关键词翻页、全局类型筛选、查询错误和空查询场景。
已在 Codex 内置桌面浏览器的 1280×720 视口执行：

- 不输入关键词时显示第一页 20 项；点击下一页后显示 21～25，上一页可用、下一页不可用。
- 查询错误场景显示错误和“重新检查”，不同时显示空文档断言。

真实 CLI 读取与浏览器模拟交互分别计为证据，不互相替代。
本次未补 880×600 的真实浏览器截图，桌面静态边界检查已通过。

搜索结果仍由飞书 ACL、可见性与索引决定；新建文档进入索引可能有延迟。
真实云端创建／删除本次未执行。已有桌面进程未被终止或替换，源码与 UI 构建不等于安装新发行包。

## 官方基线

- [CLI 1.0.98 统一检索实现](https://github.com/larksuite/cli/blob/v1.0.98/shortcuts/drive/drive_search.go)
- [空关键词、过滤与分页合同](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-drive/references/lark-drive-search.md)
- [根目录与文件夹直接子项语义](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-drive/references/lark-drive-files-list.md)
