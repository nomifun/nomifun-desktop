# 渲染器样式规范

[English](styling.md)

使用语义 token 和现有 Arco/UnoCSS 模式。共享渲染器面向 880x600 及以上的
桌面窗口和桌面级 WebUI 浏览器。渲染器或 UI 规则修改后运行
`bun run check:desktop-ui-boundary`。

## UnoCSS 工具类

可执行定义在 [ui/uno.config.ts](../../ui/uno.config.ts)。
[死工具类检查](../../scripts/check-dead-css-utilities.mjs) 拦截已知错误写法，
并通过真实生成器编译具有工具类形态的 token，检测不产出 CSS 的类。

| 意图 | 正确写法 | 避免 |
| --- | --- | --- |
| 背景 | `bg-base`、`bg-1` 至 `bg-6`、`bg-8` 至 `bg-10` | `bg-bg-N`、`bg-0`、`bg-7` |
| 文字 | `text-t-primary`、`text-t-secondary` | `text-text-primary` |
| 语义色阶 | `text-danger-6`、`bg-primary-6` | `text-[rgb(var(--danger-6))]` |
| 显式透明度 | `bg-[rgba(var(--primary-6),0.12)]` | `bg-primary-6/12` |
| 边框颜色 | `border-arco-2`、`border-[var(--border-base)]` | `border-border-2`、`border-b-base`、`border-b-light` |
| 边框宽度 | `border-2px`、`border-b-2px` | 把 `border-2` 或 `border-b-2` 当宽度 |

Arco 色阶是逗号分隔的 RGB 三元组。UnoCSS 注入的 slash-alpha 会让
`rgb(var(--danger-6) / ...)` 展开成非法颜色。使用项目语义规则或自带 alpha
的 `rgba(...)`。非语义色板、ring/outline 和带方向的颜色工具类需要显式 alpha。

`-b-` 方向陷阱会先消费方向再查颜色：`border-b-base` 指向
`--bg-base`，不是 `--border-base`。数字 theme key 是颜色；宽度显式加 `px`。

仓库没有全局 border reset，需明确写宽度、样式和颜色：

```tsx
<div className='border border-solid border-arco-2' />
<div className='border-b border-b-solid border-b-[var(--border-base)]' />
```

带方向的宽度配同方向样式。只有下边框宽度却写 `border-solid`，
其他边会回落到 `border-width: medium`。
不要添加全局 `* { border-width: 0; border-style: solid; }` reset，
它会改变原生控件默认边框和渲染器各处现有 CSS。

## 预设主题

主题明暗模式与色系分开。优先使用
[styles/themes](../../ui/src/renderer/styles/themes) 中的语义 token；
`--aou-1..10` 是当前品牌色阶的兼容名称。

[AppThemeRuntime](../../ui/src/renderer/components/layout/AppThemeRuntime.tsx)
通过 [applyCustomCss](../../ui/src/renderer/utils/theme/applyCustomCss.ts)
注入自定义 CSS，处理器给 keyframes 以外的声明追加 `!important`。预设必须：

- 亮色定义在 `:root, body`，后接
  `[data-theme='dark'], [data-theme='dark'] body` 暗色块。
  Arco 在 `body` 定义 token，只覆盖 `:root` 不足。
- 覆盖完整的 App/Arco token，亮暗 key 集合一致。准确必需列表以
  [check-theme-contract.mjs](../../scripts/check-theme-contract.mjs) 为准。
- `--primary-rgb` 和 `--primary-1..7` 使用 RGB 三元组。
  提供字面量强调色供侧栏主题选择器取色。
- 装饰只作用于具体目标和外观。禁止布局属性、宽选择器、外联资源及
  `@media` 内的变量。
- 正文对比度至少 4.5:1。Popover/Dropdown/Select 内容弹层背景足够实
  （alpha 至少 0.86）；反馈与 Tooltip 背景完全不透明。
- 不给消息排版外层 `.message-item` 添加预设背景、玻璃边框或模糊；
  使用语义气泡 token 或具体消息组件。

## 验证

样式改动运行 `bun run check:dead-css` 和 `bun run check:theme`，
并在亮暗模式检查受影响控件。行为改动运行定向组件/契约测试；
纯文档改动运行 `bun run check:docs` 和 `git diff --check`。
