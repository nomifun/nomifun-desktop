# 办公工作台：紧凑版验收记录

对应[现役实施计划](../handoffs/office-feishu-workbench.zh.md)。
验证日期：2026-10-26；分支 codex/office-feishu-workbench。
本记录仅描述本次 CLI 原生能力收敛后的代码，不沿用旧预览／模型生成版的通过数字。

## 自动验证

| 检查 | 本次结果 |
| --- | --- |
| cargo test --locked -p nomifun-agent-contracts --lib schema::tests | 14 passed |
| cargo test --locked -p nomifun-db --test office_receipts_compact --test office_hub_schema --test id_schema_contract --test agent_store_reset | 16 passed（5＋2＋6＋3） |
| cargo test --locked -p nomifun-db --lib agent_store_clean_cut::tests | 6 passed |
| node --test scripts/check-agent-session-boundary.test.mjs | 3 passed |
| cargo test --locked -p nomifun-office-hub --lib | 54 passed、3 内部子进程 helper ignored（收敛全套，110.02 秒） |
| cargo test --locked -p nomifun-office-hub --lib service::tests:: | profiles 拆分后 19 passed，68.44 秒 |
| cargo test --locked -p nomifun-office-hub --lib store::tests:: | 导航时间显示收口后 11 passed，18.66 秒 |
| cargo test --locked -p nomifun-skill-library --test lark_cli_builtin | 5 passed |
| cargo test --locked -p nomifun-runtime --lib existing_ | 3 passed |
| bun test --cwd ui src/renderer/pages/officeHub | 40 passed、0 failed、222 assertions，5.39 秒 |
| bun run check | 完整检查通过；最终局部显示调整后 typecheck、i18n 另行通过 |
| cargo build --locked -p nomifun-app --bin nomicore --no-default-features | 最新源构建通过；隔离实例启动成功 |
| git diff --check | 通过 |

三项 ignored 是父测试显式调用的进程 helper，不是跳过的业务用例。
最后的业务服务、Store 和 UI 修改已分别运行相关目标检查。
现有 KaTeX 测试 harness 的 quirks-mode 警告及 ts-rs 对 deny_unknown_fields 的解析警告不影响这些测试结果。

## 数据与架构证据

- 017 是 016 之后的前向迁移；不改已存在 migration 的 checksum。
- 只收敛手工办公域，四张现役表为连接、操作、项目和回执。
  原尝试／观察合并为回执；最近打开资源成为连接中的有限导航列表。
- 从含未知、部分结果及历史输入的数据升级，逐项输入、原回执、UUID、
  自增主键高水位和待核验目标围栏保留；无执行事实的旧 Draft 才会删除。
- 有执行事实的 Draft、坏导航、坏回执或未知 manifest 会整体回滚。
  SQLite NULL 不能绕过回执类别与证据检查。
- schema manifest 只允许精确已知的非 Agent 转换；Agent generation/head 不变。
  Agent clean cut 保留办公数据，不能把办公记录作为 Agent Session 或模型上下文。
- 独立脚本检查同时限制迁移读取和写入的数据表；不允许从 Agent 表复制数据、
  修改其他表的 sqlite_sequence 或扩大 schema_metadata 权限。

## Windows 实际页面与离线 CLI

使用本次构建的 nomicore 副本、独立 data/work 目录和仅监听 127.0.0.1 的实例。
最终二进制 SHA-256：`b839605c4b7a7a3ea614eff679e3fdd91c557890565defa538770c8d2b62cff3`。
页面为实际 Vite renderer，不是 HTML 原型；测试 CLI 明确为 QA SIMULATED，不访问飞书。

已检查最小桌面 880×600 的入口、环境步骤、已有配置发现、精确 profile 选择、
显式 `offline_access` 身份登录和可跳过的业务权限步骤。输入和页脚均在支持的桌面布局内。
最新构建还实际完成：

- 选择真实返回的段落 ID `blk_qa`，以旧 revision 保存一次，回到原文读回相同中文内容；
  单项只有一次 attempt，界面无 Markdown 模式、分屏、修改对比或追加正文面板。
- 正常保存只显示中性进度；错误和结果未知才提供原请求回执恢复入口。
- 1200×800 的 Base 原生选项控件和同表两条记录单次保存；接受后先显示待读回确认。
  只读核验后一条已确认、另一条仍待确认，汇总为部分完成；两项 attempt 均仍为 1。
  重新查询表格时未解决项仍携带原 pending operation，已确认项没有 pending 标记。
- 最近打开列表显示连接中的实际打开时间，搜索结果使用远端更新时间，两者不混用。
- 实际主库恰有四张办公表，未产生持久 Draft；每个业务项目只有一次原始 attempt。

该实例曾因测试 PowerShell 的字符串附加属性、字符编码和并发日志文件锁产生不完整 JSON，
宿主按未确认响应拒绝读取；修正隔离 fixture 后继续验收，不将其计作真实 CLI 成功。

## 外部与平台证据边界

本次自动化使用隔离 SQLite、CLI 契约执行器和测试专属目录，未读取真实用户飞书凭据。
真实企业应用创建、管理员审批、真实资源 ACL 与写入仍需在授权的飞书租户验收。
离线成功不代表外部服务通过。其他操作系统发行与原生交互没有本次运行证据。
