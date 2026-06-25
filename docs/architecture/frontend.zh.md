# 前端

前端是位于 [`ui/`](../../ui/) 的一个 React 19 SPA。两个宿主 —— Tauri 桌面外壳与 `nomifun-web` —— 都加载同一份 Vite 构建产物（`ui/dist`）。渲染进程从不使用 Electron IPC；在两个宿主中它都通过普通的 HTTP 与 WebSocket 与后端通信。

## 技术栈

| 关注点 | 选择 |
| --- | --- |
| 框架 | React 19 + TypeScript |
| 打包工具 | Vite 6 |
| UI 库 | Arco Design（`@arco-design/web-react`）—— 主色 `#4E5969` |
| 样式 | UnoCSS（utility 类）+ `ui/src/renderer/styles/themes/` 下按主题划分的 CSS |
| 路由 | `react-router-dom` v7 + **`HashRouter`**（对 `file://` 风格宿主与刷新安全至关重要） |
| 数据获取 / 缓存 | SWR |
| 状态 | React Context（auth、theme、feedback、preview、conversation history）—— 不使用 Redux |
| i18n | `i18next` + `react-i18next`，语言包 `zh-CN`、`en-US` |
| 编辑器 | Monaco（设置、代码预览）、CodeMirror（更轻量的输入） |
| Markdown | `react-markdown` + `remark-gfm` + KaTeX + mermaid |
| 终端 | `xterm.js`（含 `xterm-addon-fit`、`xterm-addon-web-links`） |
| Service worker | Web 宿主注册了 PWA service worker（参见 [`registerPwa.ts`](../../ui/src/renderer/services/registerPwa.ts)）；Tauri 外壳显式跳过它 |

## 三层结构：`common/`、`platform/`、`renderer/`

`ui/src/` 内的目录划分是承担约定职责的关键。

```
ui/src/
├── common/      shared library code (no React)
│   ├── adapter/   the bridge factory: HTTP + WS + Tauri shim
│   ├── api/       typed API surfaces built on the bridge
│   ├── chat/      chat library helpers (rendering hooks, types)
│   ├── config/    constants, configService (settings cache)
│   ├── platform/  platform-detection helpers
│   ├── types/     TypeScript mirrors of nomifun-api-types DTOs
│   ├── update/    self-update flow helpers
│   ├── utils/     shared utilities (date, hash, ...)
│   └── index.ts
├── platform/    runtime substrate
│   ├── bridge event hub (the legacy "buildProvider/buildEmitter" API)
│   ├── logger
│   ├── storage
│   └── theme
├── renderer/    the React app
│   ├── pages/         feature pages (conversation, terminal, settings, ...)
│   ├── components/    reusable UI components and layout
│   ├── hooks/         hooks and React Contexts (Auth, Theme, Feedback, ...)
│   ├── services/      i18n, FileService, PasteService, SpeechToTextService, registerPwa
│   ├── styles/        Arco overrides and theme variables
│   ├── utils/         renderer-specific utilities
│   ├── main.tsx       entry point (createRoot)
│   └── index.html
└── shims/       small interop shims pulled in by Vite
```

这种划分是有意设计的：`common/` 不知道 DOM 或 React 的存在；`platform/` 是接好桥事件中心与 logger 的小型基板；`renderer/` 才是真正的应用。这让桥接逻辑可以脱离 React 进行测试，并且如果将来出现第二个客户端目标，可以共享 `common/`。

## 适配层（桥接层）

文件位置：[`ui/src/common/adapter/`](../../ui/src/common/adapter/)。

适配层是前端可移植性故事的核心。它对外暴露一个稳定的形状 —— `provider/invoke` 用于请求—响应，`on/emit` 用于事件 —— 渲染进程的其余部分都消费这个形状。该形状之下，它根据宿主把调用路由到三种传输之一：

| 适配文件 | 传输 | 用途 |
| --- | --- | --- |
| [`httpBridge.ts`](../../ui/src/common/adapter/httpBridge.ts) | HTTP `fetch` + 单例 WebSocket | 默认 —— 所有 `/api/*` 与 `/ws` 流量。 |
| [`tauriShell.ts`](../../ui/src/common/adapter/tauriShell.ts) | Tauri JS API 与插件（`@tauri-apps/api`、`tauri-plugin-*`） | 仅用于操作系统外壳：窗口控制、对话框、OS 路径、开机启动、通知、深链接、自更新。由 `isTauri()` 守护。 |
| [`browser.ts`](../../ui/src/common/adapter/browser.ts) | 进入 platform 事件中心的旧版 WebSocket 桥接 | 把 `platform/` 的 `bridge.emit` 调用接到同一个 `/ws` 端点，并处理 auth 过期重定向。 |

复合体 —— 由 [`ipcBridge.ts`](../../ui/src/common/adapter/ipcBridge.ts) 导出 —— 才是应用其余部分引入的对象。在渲染进程看来，每次操作都长得一样，无论它最终走的是 HTTP、WS 还是 Tauri-IPC。

### 解析后端 URL

渲染进程需要知道与之对话的 URL，而答案因宿主而异：

```ts
// ui/src/common/adapter/httpBridge.ts (excerpted)
function getBackendPort(): number {
  if (typeof window !== 'undefined' && window.__backendPort) {
    return window.__backendPort;             // desktop (Tauri): injected by init script
  }
  return globalThis.__backendPort ?? 13400;  // last-resort fallback
}

function isWebUiBrowserMode(): boolean {
  return typeof window !== 'undefined' && !window.__backendPort;
}

export function getBaseUrl(): string {
  if (isWebUiBrowserMode()) return '';                              // same-origin (browser)
  return `http://127.0.0.1:${getBackendPort()}`;                    // desktop
}
```

在桌面外壳中，Tauri 主进程在任何页面脚本执行之前通过**初始化脚本**注入 `window.__backendPort`（参见 [`apps/desktop/src/main.rs`](../../apps/desktop/src/main.rs)）—— 因此渲染进程的第一次调用就能看到正确的端口，无竞争。在 Web 宿主中不会注入端口；`getBaseUrl` 返回 `''`，`fetch` 把 URL 解析到页面自身的来源。

### CSRF 双提交

当宿主以认证模式运行（即未带 `--insecure-no-auth` 的 Web 宿主），后端会签发非 HttpOnly 的 cookie `nomifun-csrf-token`。在状态变更请求（POST / PUT / PATCH / DELETE）上，桥读取该 cookie 并把它回显到 `x-csrf-token` 头里。桌面外壳使用 `TrustLocalToken`：WebView 会在请求中带上 `window.__nomiLocalTrust` 注入的本地信任 secret，而不是关闭所有鉴权。

## 路由 —— `HashRouter`

[`ui/src/renderer/components/layout/Router.tsx`](../../ui/src/renderer/components/layout/Router.tsx) 是唯一的路由组件。它使用 **`HashRouter`**（形如 `/#/conversation/abc123` 的 URL），原因有两个：

1. Tauri 外壳通过 `tauri://` / `file://` 协议加载 SPA；`BrowserRouter` 在该协议下经历的页面重新加载（如深链接或应用内导航）后无法保留状态。
2. Web 宿主通过 `tower_http::services::ServeDir` 提供 SPA，并启用 `append_index_html_on_directories(true)`。Hash 路由意味着浏览器访问的任何路径都返回 `index.html`，由 SPA 完成其余工作 —— 静态服务器无需自定义 catch-all。

路由表的顶层条目涵盖会话运行时（`/guid`、`/conversation/:id`）、模型（`/models`）、助手与技能（`/assistants`）、MCP（`/mcp`）、开放能力（`/open-capabilities`）、终端（`/terminal-new`、`/terminal/:id`）、需求/AutoWork（`/requirements/*`、`/autowork` redirect）、定时任务（`/scheduled`、`/scheduled/:job_id`）、桌面伙伴（`/nomi` 配置页、`/companion` 桌面窗口）、知识库（`/knowledge`、`/knowledge/:id`）以及认证（`/login`）。旧 settings 路径只作为重定向保留；当前没有 `/team/:id` 前端路由。

页面通过 `React.lazy` 加载，使用 `<AppLoader>` 作为 fallback，使初始包保持精简。

## 状态与数据

- **SWR** 是主要的数据层。约定是任何列表或详情视图都声明一个 SWR key 字符串及一个 fetcher；HTTP 响应到达后变更操作会调用 `mutate(key)`。`ipcBridge.*.invoke` 的返回值直接喂给 SWR。
- **React Context** 承载不属于 SWR 的应用形态状态：认证（`AuthProvider`）、主题（`ThemeProvider`）、反馈 toast（`FeedbackProvider`）、文件预览（`PreviewProvider`），以及对话历史列表（`ConversationHistoryProvider`）。
- **`configService`**（`ui/src/common/config/configService.ts`）缓存后端设置；[`main.tsx`](../../ui/src/renderer/main.tsx) 中的入口点会在 i18n / theme 代码加载前启动 `configService.initialize()`，因此这些子系统在首次渲染时读到的是权威设置。

## 主题

Arco 的 `ConfigProvider` 在根处包裹应用，主色为 `primaryColor: '#4E5969'`，并按语言提供 locale（`enUS`、`zhCN`、`zhTW`、`jaJP`、`koKR` —— 韩语包用英语日历 / datepicker 字段做了补丁，因为 Arco 的 `koKR` 缺这些）。主题（`light`、`dark`、品牌变体）以纯 CSS 文件叠在 `ui/src/renderer/styles/themes/index.css` 中，通过 `ThemeProvider` 切换。

UnoCSS 与 Arco 并行提供 utility 类 —— 其配置位于仓库根目录的 `uno.config.ts`。Arco 的自定义覆盖位于 `ui/src/renderer/styles/arco-override.css`。

## 国际化

[`ui/src/renderer/services/i18n`](../../ui/src/renderer/services/) 用上述五种语言初始化 `i18next`。字符串按功能组织，解析后的语言通过 `main.tsx` 中的 `arcoLocales` map 流入 Arco。切换语言无需重新加载 —— i18next 与 Arco 都会按新语言重新计算。

## 一点平台特定的 UX

桌面外壳在 Windows / Linux 上是**无边框**的（[`ui/src/renderer/components/layout/Titlebar/`](../../ui/src/renderer/components/layout/Titlebar/) 中的 React 标题栏通过 `@tauri-apps/api/window` 绘制最小化 / 最大化 / 关闭按钮）；macOS 通过 `TitleBarStyle::Overlay` 保留原生交通灯按钮。同一份 SPA 在浏览器中会隐藏标题栏，让浏览器外框处理它。区别在运行时通过 `isTauri()`（定义于 `tauriShell.ts`）来检测。
