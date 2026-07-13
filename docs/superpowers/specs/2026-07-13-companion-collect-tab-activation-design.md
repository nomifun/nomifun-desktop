# 桌面伙伴「整理」标签激活态设计

## 目标

让桌面伙伴管理页共享域内的「整理」标签具备与“需求”页一致、可一眼识别的激活态。

## 范围与方案

- 仅修改 `ui/src/renderer/pages/nomi/index.tsx` 的共享域标签控件。
- 用项目已有的 `SegmentedTabs` 替换当前 Arco `Tabs` 的导航外观，使用与需求页相同的浅色选中底、主题色文字和轻阴影。
- `collect`、`learn`、`suggestions`、`migrate` 四个标签及其内容、懒加载和 `?tab=` 深链继续保持原有行为。
- 不修改伙伴域子标签、页面数据请求、文案或路由契约。

## 交互与可访问性

- 当前标签由既有 `activeTab` 决定；点击后仍调用现有 `setTab`，保留浏览器历史替换与 URL 同步。
- `SegmentedTabs` 提供 `tablist` / `tab` 语义及 `aria-selected`，选中状态也将通过视觉样式清晰呈现。

## 验证

- 添加或更新渲染页的静态回归测试，确认共享标签使用 `SegmentedTabs`，且四个标签键和内容渲染分支仍完整。
- 运行对应 UI 测试与 TypeScript/lint 可用的最小验证命令。
