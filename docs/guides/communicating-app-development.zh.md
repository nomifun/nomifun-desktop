# NomiFun 应用开发指南

应用页的创建、编辑和导入工作区固定使用官方「内置应用」Agent（`app.developer`），
用户照常选择模型。它有普通编程能力及应用工作区所需的 `app.hosting` 管理动作，执行普通 Conversation，
保留完整开发过程和历史，不创建专属创作会话、验收流水线或第二套日志。

其他 Agent 可在工作台明确选入「NomiFun 应用开发」能力。只有已授权的 Agent/会话
可以调用 `read_app_guide` 按需读取本指南和使用托管工具。详细连接说明不加入所有 Agent 的上下文。
工作区文件和进程权限继续由普通编程能力授予；已有个人 Agent 和冻结会话不自动扩大动作范围。

## 编写、登记与运行

在应用工作区中，“创建一个应用”“修改这个应用”“导入这个项目”包含把当前程序接到右侧
应用详情与运行配置。不能只写文件、让用户猜命令，再报告开发完成。用户无需重复要求命名、
填写命令或提供托管步骤；这些是应用开发的一部分。用户明确只讨论方案、只改文件或不要启动时，
遵守其范围。后台程序没有网页时保持地址为空并实际检查其功能，不伪造网页。

1. 先按需调用 `read_app_guide` 读取本指南，再调用 `current_app`。它只从本次调用的 canonical Session 工作区寻找精确对应的托管应用；
   返回 `application` 时，工作区已是该应用自己的目录。使用返回的 ID，直接修改当前文件，
   **不要再次 `import_app` 复制这个目录、创建重复应用**。应用详情、状态、操作与预览由同一 owner 提供。
2. 根据用户需求编写真实项目、识别外部输入并测试。导入助手先检查用户带来的目录/ZIP和文件结构，
   区分可编译源码与可直接运行产物；不因用户仅说“导入”就武断执行构建。说明判断依据，
   帮用户填写配置和完成需要的工作，保留原项目。必要的缺项可询问，已确定的常规选择直接执行。
   在应用工作区导入外部代码时，`import_app`必须带 `current_app`返回的 `app_id`和外部源绝对
   `path`，将文件导入当前应用，保留相同应用和开发会话。不能把当前托管目录当作外部来源再导入自身。
3. 使用 `update_app` 更新同一应用的名称、描述、标签、`import_kind` 及普通 `settings`。
   填写与真实成果相符的名称和简短说明，不能把已生成的游戏仍留作“新应用”。
   可发送 `detect_settings: true` 按当前实际文件补充未配置的普通运行设置；例如唯一的静态 HTML
   或 package.json 项目。它保存同一应用的配置，不启动进程、不再次复制目录。
   识别不出自定义服务时直接填写其实际命令和网页地址；不让用户代替你完成已知的常规配置。
   配置可覆盖 `project_directory`、`install_command`、`preview_command`、`build_command`、
   `start_command`、`output_directory`、`web_url`、`node_path`、`environment` 或 `mcp_url`。
   明确 `null` 跳过命令，省略字段保留当前值。工具不能修改运行时 Agent 连接或开发会话引用。
   对直接运行的产物通常不安装、不构建；源码按真实项目的依赖、预览和生产命令配置。
4. `preview_app` 按预览命令启动真实进程；`start_app` 首次执行安装、构建、启动，后续日常启动
   只运行启动命令；`rebuild_app` 在改文件后重新构建并启动同一应用。`app_status`、`list_apps`
   查询真实状态，`stop_app` 停止程序。完全未配置预览或启动命令时，显式预览/使用会检查
   当前文件，为唯一可识别项目补充普通命令。直接生成的静态 HTML 可使用平台 HTTP 预设，
   不必再次导入；同目录存在 README 或业务文件不妨碍识别唯一项目。多个候选项目或无法识别
   时需要明确配置，查询状态不自动修改配置或启动。已有命令和网页地址保持原值；已有前台命令
   的项目中，明确跳过的安装/构建也保持原值。
   标准 `python -m http.server` 的确定端口和本机绑定可补充网页地址；自定义服务仍需提供其真实地址。
   网页地址和进程都可空配置，但启动需要真实前台命令，
   不能把目录当进程或叠加 PM2 daemon。用户可在右侧手动配置和执行相同操作，不必借助 Agent。
5. 配置保存后，通过 `preview_app` 预览当前成果；修改已运行的程序时先 `stop_app`，
   修改当前文件及配置后再预览或 `rebuild_app`。不得用普通进程工具另起一份服务绕过托管。
   从真实操作结果确认网页地址，实际打开网页并验证应用功能后再报告可预览；返回 `running`
   但 `web_url` 为空不能说明预览已可用。用户点击程序预览会打开可缩放的内嵌视图。
   网页关闭不停止服务。运行中修改命令、依赖或配置前先停止应用。
   普通开发过程完整保存在会话中，不能把模型回答“完成”、文件存在或 HTTP 200当成运行验证。

右侧详情只读取同一托管 owner 的结果。真实预览运行时可在右侧直接加载网页；工具调用失败时
保留错误并继续解决，不把口头“已部署”“已启动”当作完成。报告实际应用名称、可用操作与验证
结果，未完成的配置或启动问题必须明确指出。

用户提交的开发回合真实完成后，应用页还会通过显式配置更新读取当前文件，补齐可确定的缺项。
该操作核验确切 Session/Turn 和闲置准入围栏，不执行安装、构建或启动，不在 GET、轮询或重开
历史时触发。用户未保存的字段保持在界面中，未编辑字段跟随 Agent 保存的实际配置。
已加载的预览在同一进程下收到该成功回合的文件修改观察时刷新一次；只读回合和普通轮询保留
网页状态。重启后的新进程仍加载实际新网页。仅 shell 生成或删除文件不会冒充这类写入效果。

如果 `current_app` 返回 `null`，当前是普通项目工作区。可先开发实际文件，然后 `import_app`
传项目绝对 `path`、`name`、`import_kind: "source"` 或 `"artifact"`及所需 `settings`；
返回的 `application.directory` 才是托管副本。导入后修改原工作区不改变这个副本，
应继续编辑返回的目录，或用现有「覆盖」替换文件后重新构建。`import_app`不是更新配置动作。

不要求 SDK、专有 Manifest、activate、window.nomi、版本、部署、发布、预览资格或专属验收流程。
静态网页可以使用平台普通 HTTP 预设；SSR/后台程序执行真实启动命令。
`running` 只说明受管进程存在，必须另外实际验证网页或后台业务功能。

## 无感连接 NomiFun

预览/启动的后端进程自动继承三个环境字段：

| 环境字段 | 含义 |
| --- | --- |
| `NOMIFUN_HOST_URL` | 当前 Host 的实际本机 HTTP 地址 |
| `NOMIFUN_LOCAL_TRUST` | 放进请求头 `x-nomi-local-trust` 的本机凭据 |
| `NOMIFUN_AGENT_BINDING_ID` | 用户在运行设置选择 Agent 后准备的连接引用，可无此字段 |

用户不配置 token。安装和构建不获得连接字段，Host 重启后更换凭据，下一次启动获得新连接。
这些字段只在普通后端读取；不能写进网页、打包产物、响应、日志或源码常量。
浏览器通过应用自己的后端调用平台，不能建立前端 SDK Bridge。
本机连接按安装 owner 信任，与 NomiFun 同一用户；它不是应用隔离沙箱。外部客户端继续原鉴权。

下面是普通 Node 后端函数示例，可按技术栈采用同样的 HTTP 语义：

```js
async function nomifun(path, method = 'GET', body) {
  const response = await fetch(new URL(path, process.env.NOMIFUN_HOST_URL), {
    method,
    headers: {
      'x-nomi-local-trust': process.env.NOMIFUN_LOCAL_TRUST,
      ...(body === undefined ? {} : { 'content-type': 'application/json' }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const result = await response.json();
  if (!response.ok || result.success === false) {
    throw new Error(`NomiFun request failed (${response.status})`);
  }
  return result.data ?? result;
}
const apps = await nomifun('/api/apps');
```

可向前端返回 `connected`、应用数量或需要的业务结果，不返回请求头、环境变量或整个配置。
自身 PID、文件、子进程等信息直接用 OS/语言接口，例如 `process.pid`，无需平台请求。

## 请求 Agent：复用四操作

用户在应用的运行设置中选择目标 Agent。工具不能伪造 `agent_binding_id` 或开发会话引用。
更换 Agent 前先停止应用，使后端运行环境获得正确连接。

后端使用以上 `nomifun()` 进行：

```js
const opened = await nomifun('/api/remote/open', 'POST', {
  binding_id: process.env.NOMIFUN_AGENT_BINDING_ID,
  idempotency_key: openKey,
});
const session = opened.agent_session_id;
await nomifun('/api/remote/turn', 'POST', {
  agent_session_id: session,
  input: { content: '用户要 Agent 执行的内容' },
  idempotency_key: turnKey,
});
const observation = await nomifun(
  `/api/remote/observe?agent_session_id=${encodeURIComponent(session)}&after_seq=0&limit=100`,
);
await nomifun('/api/remote/cancel', 'POST', {
  agent_session_id: session,
  idempotency_key: cancelKey,
});
```

`openKey`、`turnKey`、`cancelKey` 为每个逻辑操作稳定的幂等标识；结果不确定时保留原 key 和 Session，
先 observe，不盲目新建或重复派发。依据实际事件判断完成和错误，不能把已提交任务或 HTTP 200 当成业务完成。
会话与历史属于现役 canonical owner，应用不复制第二份日志或直接删 Message 投影。

## 向 Agent 提供应用工具

应用可以提供普通 MCP HTTP 地址，通过 `settings.mcp_url` 登记唯一来源。
NomiFun 从正在运行的同一服务发现工具，不额外拉起程序；Agent/会话明确选择，默认不向所有 Agent 注入。
Schema、资源读取、调用、容量和失败语义复用现役 MCP，连接故障只影响该来源。
Provider 地址、Skill、主题继续原有设置和导入，不另建扩展生命周期。
自动上下文/before_model/before_tool、内嵌 Surface、事件总线、App Cron 和动态领域适配器不在本能力范围。

应用拥有自己的业务文件和数据库，迭代由应用及编程 Agent 处理。平台只运行当前文件，
不提供业务迁移、备份或回滚。删除仅处理托管副本和来源/运行连接引用，
原导入目录、外部数据、普通开发会话与共享会话保留。
