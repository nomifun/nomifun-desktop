# NomiFun 本地销售工作台：当前架构与执行逻辑

> 当前实现直接使用 NomiFun 自己的 Agent、模型、Skills、浏览器工具和会话系统，不接入外部 Agent Worker。

## 系统架构

```mermaid
flowchart LR
    User["销售用户"] --> UI["NomiFun 销售工作台"]

    subgraph App["nomifun-sales 容器"]
        UI --> Auth["登录与 user_id 归属校验"]
        Auth --> Sales["公司资料、计划、公司与结果"]
        UI --> Conversation["NomiFun Agent 会话"]
        Preset["本地销售联络助手设定"] --> Conversation
        Skill["sales-contact-operator Skill"] --> Preset
        Model["用户配置的 Chat 模型"] --> Conversation
        Conversation --> Browser["NomiFun 浏览器工具"]
        Conversation --> Events["SALES_DASHBOARD_EVENT"]
        Events --> Sales
        Sales --> DB[("nomifun-sales-data")]
    end

    Browser --> Sites["目标公司官网与 Contact Form"]
    Browser -. "本地演练" .-> Mock["mock-contact-site :8088"]
```

## 立即执行逻辑

```mermaid
sequenceDiagram
    autonumber
    actor User as 当前登录用户
    participant Tasks as 联络任务页面
    participant Nomi as NomiFun Agent 会话
    participant Skill as sales-contact-operator
    participant Browser as 浏览器工具
    participant Dashboard as 执行看板

    User->>Tasks: 填写国家、数量、筛选条件
    User->>Tasks: 点击立即执行
    Tasks->>Nomi: 使用“本地销售联络助手”创建会话
    Tasks->>Nomi: 发送公司资料、任务 ID 和执行要求
    Nomi->>Skill: 加载销售联络规则
    loop 逐家公司处理
        Nomi->>Browser: 搜索并核实官网
        Browser-->>Nomi: 官网证据与 Contact Form
        Nomi->>Nomi: 判断行业匹配和联络限制
        alt 合格且允许销售联络
            Nomi->>Browser: 填写并自动提交一次
        else 不合格、验证码、登录或禁止销售
            Nomi->>Nomi: 跳过或记录失败，不绕过限制
        end
        Nomi-->>Dashboard: SALES_DASHBOARD_EVENT
    end
    Dashboard->>Nomi: 每 8 秒读取会话进度
    Dashboard->>Dashboard: 合并公司、状态、内容和结果
```

## 多账号隔离

```mermaid
flowchart TB
    A["账号 A"] --> WA["销售数据 A"]
    A --> CA["Agent 会话 A"]
    B["账号 B"] --> WB["销售数据 B"]
    B --> CB["Agent 会话 B"]

    WA -. "不可跨账号读取" .- WB
    CA -. "不可跨账号读取" .- CB

    Shared["管理员维护的共享配置<br/>Agent、Skills、模型供应商"] --> CA
    Shared --> CB
```

- 公司资料、任务、公司列表、结果和 Agent 会话按登录账号隔离。
- 普通账号不能读取其他账号的数据，也不能进入账号管理和系统配置。
- Agent、Skills 与模型供应商属于当前 NomiFun 实例的共享配置，由管理员维护。
- 如果未来不同客户公司需要完全不同的 Agent 和知识库，应增加“客户组织”层，并为组织绑定不同设定和知识库。

## 本地组件

| 组件 | 职责 |
|---|---|
| `nomifun-sales` | WebUI、认证、销售数据、Agent、模型、Skills 和浏览器运行时 |
| `mock-contact-site` | 本地 Contact Form 演练和提交验证 |
| `sales-contact-operator` | 官网核实、资格判断、表单处理、自动提交与事件格式 |
| “本地销售联络助手” | 把默认 Agent、销售 Skill 与可用模型组合为任务设定 |

详细启动和验收步骤参见 [sales-runtime-local.zh.md](./sales-runtime-local.zh.md)。
