# 阶段 1：本地销售浏览器运行时

本阶段用 Docker Compose 在本机跑通一条不依赖 VPS、也不依赖模型供应商的真实浏览器链路：

1. `nomifun-web` 以 `sales-runtime` feature 编译，启用 `nomifun-app/browser-use`；
2. 容器安装系统 Chromium，并以非 root 用户（UID/GID 10001）运行；
3. 独立的模拟客户站点提供 Contact Form，并把提交记录写入本地命名卷；
4. smoke 脚本通过 `/mcp-agent` 调用 NomiFun 自己的 `nomi_browser_*` 工具，完成打开、观察、填写、确认提交和结果校验。

普通 `Dockerfile` 最终目标与 `docker-compose.yml` 不会自动安装或启用 Chromium；只有本文件描述的本地销售栈会启用它。

## 启动

需要 Docker Desktop / Docker Engine 与 Compose：

```bash
docker compose -f docker-compose.sales-local.yml up --build -d
docker compose -f docker-compose.sales-local.yml ps
```

打开：

- NomiFun WebUI：<http://127.0.0.1:8787>
- 模拟 Contact Form：<http://127.0.0.1:8088>

Compose 内置的管理员密码与安装令牌只用于绑定在 loopback 的阶段 1 环境。可通过 `NOMIFUN_ADMIN_PASSWORD` 与 `NOMIFUN_ACCESS_TOKEN` 环境变量覆盖；不要把默认值用于共享环境、局域网或云端。

## 运行端到端验收

```bash
node tools/sales-local/smoke.mjs
```

成功时最后一行类似：

```text
PASS: browser submitted contact record <uuid>
```

这个检查不是直接调用模拟站点 API 来伪装浏览器结果。它会建立有安装令牌认证的 MCP session，确认 browser tools 已注册，启动容器里的 Chromium，根据 accessibility refs 填表，并在提交按钮触发的不可逆操作安全门上执行一次 `proceed_once`。提交后的 transport 回执即使不明确，也只会观察当前页面并核对可见的 `Inquiry received`，绝不会再次点击提交。最后才从模拟站点读取记录进行断言。

可以直接查看当前记录：

```bash
curl -s http://127.0.0.1:8088/api/submissions
```

## 阶段 2：启用销售 Agent

Compose 会把仓库维护的 `sales-contact-operator` 技能只读挂载到 NomiFun 的
Custom Skills 目录；它不会覆盖 `/data/skills` 下的其他用户技能。创建或刷新配套设定：

```bash
node tools/sales-local/setup-phase2.mjs
```

脚本使用本地管理员账号登录，但不会读取或写入模型密钥。修改过本地管理员密码时，显式传入：

```bash
NOMIFUN_ADMIN_PASSWORD='你的本地密码' node tools/sales-local/setup-phase2.mjs
```

脚本是幂等的：重复执行会更新同名的用户设定，不会创建重复记录。成功时会确认：

- `sales-contact-operator` 已被 NomiFun 发现；
- “本地销售联络助手”设定已解析到默认 Agent 并绑定这个技能；
- 设定原本没有模型偏好时，已自动选中一个当前可用的 Chat 模型；
- 至少一个启用的 Chat 模型可用。

当前本地实例需要至少一个已启用的 Chat 模型。模型请求和回复会发送到你在 Model Hub 配置的供应商；不要在演练提示词里放入密码、密钥、私人客户数据或其他敏感信息。

销售工作台的“立即执行”会直接创建一个使用“本地销售联络助手”设定的 NomiFun Agent 会话，并把该会话关联到销售任务。执行看板每 8 秒从会话回复中同步结构化进度。

在 WebUI 中开始演练：

1. 打开 <http://127.0.0.1:8787/#/guid>，点击“新建会话”；
2. 在输入框上方点击“使用 Skills · 已启用”（数字可能不同）；
3. 在右侧抽屉顶部从“Skills”切换到“设定”；
4. 点击“本地销售联络助手”卡片中的“使用”；
5. 发送示例提示：`请在本地模拟 Contact Form 上演练一次自动销售联络流程，核实后直接提交并输出 Dashboard 事件。`；
6. 检查 Agent 的官网证据、提交结果和 `SALES_DASHBOARD_EVENT` 记录。

左侧导航里的“设定”页面用于创建和编辑设定，所以详情页只有“保存”和“取消”，不能从那里启动会话。

技能要求逐家公司处理，不绕过登录、CAPTCHA 或访问限制，不编造个性化信息。任务明确授权自动提交时，只向官网证据充分、符合筛选要求且没有销售联络禁令的公司提交一次；不符合、跳过和失败记录都会输出为 Dashboard 事件。

技能先用只读批量抓取发现候选公司，再核实官网域名和实际 Contact Form；只有得到精确表单 URL 后才打开交互浏览器。它不会在 `about:blank` 上等待或反复重开浏览器，也不会把行业目录自带的买家询价入口误当作供应商官网表单。

## 多账号工作台与数据隔离

安装管理员可打开 <http://127.0.0.1:8787/#/sales/users> 创建销售账号或重设密码。普通账号进入后只看到自己的销售工作台；运营管理、模型、技能和账号管理入口不会展示。

隔离边界如下：

- 每个登录账号拥有独立的公司资料、计划、候选公司、结果、Dashboard 数据和关联 Agent 会话；
- 保存工作台时必须同时提交当前登录账号 ID，切换账号期间排队的旧保存请求会被拒绝，避免串写；
- NomiFun Agent 会话按用户归属校验；模型供应商、系统级技能和 Agent 设定由管理员统一管理；
- 管理员能创建账号和重设密码，但销售工作台 API 不允许通过伪造用户 ID 读取或写入其他账号的数据。

可运行真实登录验收。脚本会为管理员和一个本地测试账号分别写入不同的标记数据，验证互相不可见、普通账号不能访问账号管理接口，然后恢复两个账号原有的工作台数据：

```bash
node tools/sales-local/tenant-smoke.mjs
```

成功时最后一行类似：

```text
PASS: admin and sales-isolation-check have isolated sales workspaces.
```

脚本会保留 `sales-isolation-check` 测试账号，便于重复验收；管理员可在“账号与隔离”页面重设其密码。

## 停止与清理

保留 NomiFun 数据与模拟提交：

```bash
docker compose -f docker-compose.sales-local.yml down
```

同时删除本地销售栈的两个命名卷：

```bash
docker compose -f docker-compose.sales-local.yml down -v
```

`down -v` 会永久删除本阶段生成的账号、配置与 Contact Form 提交记录。

## 当前安全边界

- 两个宿主端口都只绑定 `127.0.0.1`。
- NomiFun 浏览器容器以非 root 用户运行，模拟站点与 NomiFun 数据使用不同命名卷。
- 当前浏览器引擎在 Linux 上仍会给 Chromium 加 `--no-sandbox`；非 root 与容器隔离降低影响面，但不能替代 Chromium sandbox。云端部署前应单独完成 sandbox、出口网络、密钥与域名 allowlist 的加固。
- Phase 1 smoke 测试不需要 LLM API key。销售工作台的自然语言 Agent 需要一个已启用的 Chat 模型；不要求开启免费模型，可以使用你自己配置的模型供应商。
