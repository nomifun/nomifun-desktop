# Theme Color Migration Guide

> 门禁 / Gate: `bun run check:dead-css`（`scripts/check-dead-css-utilities.mjs`）
> 拦住本文「⚠️ 三个死写法」一节里的写法新增。存量是棘轮基线，只许变少。

## ⚠️ 三个死写法 / Three dead forms (measured, not guessed)

下面三种写法在本仓库**编译不出 CSS，或编译成错的属性**。三者都用真实 UnoCSS
生成器（`ui/uno.config.ts`）实测过。写出来不会报错、tsc 抓不到、页面也不会崩——
元素只是静默保留继承来的颜色，所以特别容易蔓延。

### 1. `{text,bg,border}-[rgb(var(--RAMP-N))]` —— 整条声明被浏览器丢弃

`RAMP` ∈ `primary` | `danger` | `success` | `warning` | `link`。

UnoCSS 把中括号里的值识别成颜色，于是注入 slash-alpha：

```css
/* text-[rgb(var(--danger-6))] 实际产出 */
.text-\[rgb\(var\(--danger-6\)\)\] {
  --un-text-opacity: 1;
  color: rgb(var(--danger-6) / var(--un-text-opacity));
}
```

但这些 ramp 变量是**逗号分隔的三元组**（Arco 是 `--red-6: 245,63,63`，预设主题写
`--primary-6: 232, 23, 74;`），展开后是 `rgb(245,63,63 / 1)` —— 语法非法，浏览器
把**整条 `color` 声明**丢掉。

```tsx
// ❌ 编译出来的声明被浏览器丢弃，元素继承父级颜色
<span className='text-[rgb(var(--danger-6))]'>删除</span>
<div className='bg-[rgb(var(--primary-6))]' />
<div className='focus-visible:border-[rgb(var(--primary-6))]' />

// ✅ 用项目自带规则（uno.config.ts 的 (bg|text|border)-(primary|success|warning|danger)-[1-9]）
<span className='text-danger-6'>删除</span>   // → color: rgb(var(--danger-6))
<div className='bg-primary-6' />
<div className='focus-visible:border-primary-6' />
```

**例外（合法，不要改）**：显式 `rgba(...)` 自带 alpha，UnoCSS 不再注入，产出原样保留：

```tsx
// ✅ 合法：background-color: rgba(var(--primary-6),0.12)
<div className='bg-[rgba(var(--primary-6),0.12)] text-danger-6' />
// ✅ 合法：非颜色属性（阴影）里的 rgb()/rgba() 不走颜色管道
<div className='shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.22)]' />
```

`link` 色阶**没有**对应的项目规则（规则只覆盖 primary/success/warning/danger），且
Arco 只提供三元组 `--link-6`（没有 `--color-link-6`），所以要 link 色就写
`text-[rgba(var(--link-6),1)]` —— 自带 alpha，不会被注入。

### 2. `border-border-N` —— 产出 0 条 CSS

theme 里没有名为 `border` 的颜色，`border-border-1/2/3` 完全不生成规则。

```tsx
// ❌ 一条 CSS 都没有
<div className='border border-border-2' />

// ✅ 三种可选替代
<div className='border border-arco-2' />              // → var(--color-border-2)（Arco 边框 token，1-4）
<div className='border border-3' />                   // → var(--bg-3)（项目背景/分隔色阶）
<div className='border border-[var(--border-base)]' /> // → var(--border-base)（基础边框变量）
```

### 3. `-b-` 方向陷阱：`border-b-base` / `border-b-light` 打不到基础边框

UnoCSS 解析 `border-b-*` 时**先吃掉 `-b-` 当成 bottom 方向**，再拿剩下的键去
`theme.colors` 里查，所以：

| 写法             | 实际产出                                | 本意                              |
| ---------------- | --------------------------------------- | --------------------------------- |
| `border-b-base`  | `border-bottom-color: var(--bg-base)`   | 四边基础边框 `var(--border-base)` |
| `border-b-light` | `border-bottom-color: rgb(246 246 246)` | 浅色边框 `var(--border-light)`    |

`uno.config.ts` 里曾有一个 `borderColors = { 'b-base', 'b-light', 'b-1', 'b-2', 'b-3' }`
配置块想支撑这套写法，**它永远不可达**（方向解析先发生，且 `b-1/2/3` 会被
`backgroundColors` 的数字键抢先命中），已删除；删除前后整站产出 CSS 逐字节相同。

```tsx
// ❌ 落到 --bg-* 上，不是基础边框色
<div className='border border-b-base' />

// ✅ 四边基础边框
<div className='border border-[var(--border-base)]' />
// ✅ 只要下边框（宽度/样式/颜色分开写）
<div className='border-b border-b-solid border-b-[var(--border-base)]' />
```

同一个陷阱还有一个变体：**数字键**也会被 theme 颜色劫持。`border-b-2` 不是
「下边框 2px」，而是 `border-bottom-color: var(--bg-2)`（`backgroundColors` 的 `2` 键
命中）。要下边框宽度就写 `border-b-2px`。

## 🧹 存量清理配方 / Sweep recipe

存量故意**没有**在引入门禁的那次改动里一起改：一次性替换 95 个文件会同时改变 276 处
渲染颜色（现在继承父级颜色的元素会开始显示本意颜色），那是独立的、需要逐一目测的
改动。基线记录在 `scripts/check-dead-css-utilities.mjs` 的 `BASELINE` 里。

HEAD 实测基线（`ui/src` 下的 `.ts`/`.tsx`/`.css`，不含 `*.test.ts(x)`）：

| 写法                                    | 文件数 | 出现次数 |
| --------------------------------------- | ------ | -------- |
| `{text,bg,border}-[rgb(var(--RAMP-N))]` | 79     | 228      |
| `border-border-N`                       | 17     | 40       |
| `border-b-base` / `border-b-light`      | 4      | 8        |
| **去重合计**                            | **95** | **276**  |

（连 `*.test.ts(x)` 一起算，ramp 是 85 个文件、`border-border-N` 是 19 个；测试里的
那些是「断言源码不含某写法」的字符串字面量，不会渲染成 CSS，所以门禁不扫测试文件。
其中 `knowledgeCreateCtaContrast.test.ts` 与 `scheduledTaskLayout.test.ts` 反过来断言了
破写法**存在**，清理时必须同步改这两处断言。）

机械替换步骤（建议一次只做一种写法、一个目录，便于目测）：

```bash
# 1) ramp：text-[rgb(var(--danger-6))] → text-danger-6（primary/success/warning/danger）
#    注意：不要动 rgba(...)，也不要动 shadow-[...] 里的 rgb()
rg -l --glob '!*.test.ts*' -e '(text|bg|border)-\[rgb\(var\(--(primary|danger|success|warning)-[0-9]\)\)\]' ui/src \
  | xargs sed -i -E 's/(text|bg|border)-\[rgb\(var\(--(primary|danger|success|warning)-([0-9])\)\)\]/\1-\2-\3/g'
#    link 没有项目规则，单独手改成 rgba(var(--link-6),1)（见上文第 1 节）
rg -n --glob '!*.test.ts*' -e '(text|bg|border)-\[rgb\(var\(--link-[0-9]\)\)\]' ui/src

# 2) border-border-N → border-arco-N（保留同级视觉层次）
rg -l -e 'border-border-[0-9]' ui/src | xargs sed -i -E 's/border-border-([0-9])/border-arco-\1/g'

# 3) border-b-base / border-b-light（4 个文件、8 处，手改：先判断是四边还是仅下边框）
rg -n -e 'border-b-(base|light)' ui/src
```

每清干净一个文件，就把它从 `scripts/check-dead-css-utilities.mjs` 的 `BASELINE` 里
删掉——门禁会在文件清零时主动要求你删（这张表只能变短）。跑
`bun run check:dead-css` 确认，再 `bun run build:ui` + 目测明暗两套主题。

## 🎨 使用方式

### 1. UnoCSS 原子类（推荐）✨

```tsx
// ✅ 背景色 - 简洁直观
<div className="bg-base">     // 主背景 (白色/黑色)
<div className="bg-1">        // 次级背景 (#F7F8FA)
<div className="bg-2">        // 三级背景 (#F2F3F5)
<div className="bg-brand">    // 品牌色背景 (#7583B2)

// ✅ 文本色 - 语义化
<div className="text-t-primary">    // 主要文字 (#1D2129)
<div className="text-t-secondary">  // 次要文字 (#86909C)
<div className="text-brand">        // 品牌色文字

// ✅ 边框色（⚠️ 不要写 border-b-base，见上文「-b- 方向陷阱」）
<div className="border-[var(--border-base)]">  // 基础边框 (#E5E6EB)
<div className="border-arco-2">                // Arco 边框 token
<div className="border-3">                     // --bg-3 分隔色

// ✅ 状态色阶（走项目规则，1-9）
<div className="text-danger-6 bg-primary-1 border-success-5">

// ✅ 品牌色系列
<div className="bg-aou-1">           // AOU 色板 1-10
<div className="hover:bg-brand-hover"> // 品牌色悬停
```

### 2. 内联样式（CSS 变量）

```tsx
<div style={{ backgroundColor: 'var(--bg-base)' }}>
<div style={{ color: 'var(--text-primary)' }}>
<div style={{ borderColor: 'var(--border-base)' }}>
<div style={{ backgroundColor: 'var(--brand)' }}>
```

## 📋 常见颜色映射表

| 旧值 (Hex) | UnoCSS 类                               | CSS 变量                | 说明            |
| ---------- | --------------------------------------- | ----------------------- | --------------- |
| `#FFFFFF`  | `bg-base`                               | `var(--bg-base)`        | 主背景          |
| `#F7F8FA`  | `bg-1`                                  | `var(--bg-1)`           | 次级背景/填充色 |
| `#F2F3F5`  | `bg-2`                                  | `var(--bg-2)`           | 三级背景        |
| `#E5E6EB`  | `bg-3` 或 `border-[var(--border-base)]` | `var(--border-base)`    | 边框/分隔线     |
| `#7583B2`  | `bg-brand` / `text-brand`               | `var(--brand)`          | 品牌色          |
| `#EFF0F6`  | `bg-aou-1` / `bg-brand-light`           | `var(--aou-1)`          | 品牌浅色背景    |
| `#E5E7F0`  | `bg-aou-2`                              | `var(--aou-2)`          | AOU 色板 2      |
| `#1D2129`  | `text-t-primary`                        | `var(--text-primary)`   | 主要文字        |
| `#86909C`  | `text-t-secondary` / `bg-6`             | `var(--text-secondary)` | 次要文字        |
| `#165DFF`  | `bg-primary` / `text-primary`           | `var(--primary)`        | 主色调          |

## 🔄 迁移步骤

1. **搜索**硬编码颜色：`bg-#`, `text-#`, `color-#`, `border-#`
2. **查表**对应的主题变量
3. **替换**为 UnoCSS 类
4. **跑门禁** `bun run check:dead-css`（别把新写法写成上文三个死写法）
5. **测试**明暗主题切换

## 💡 迁移示例

### Before (硬编码):

```tsx
<div className='bg-#EFF0F6 hover:bg-#E5E7F0'>
  <span className='text-#1D2129'>文本</span>
  <div className='border border-#E5E6EB'></div>
</div>
```

### After (主题变量):

```tsx
<div className='bg-aou-1 hover:bg-aou-2'>
  <span className='text-t-primary'>文本</span>
  <div className='border border-[var(--border-base)]'></div>
</div>
```

### 常见模式:

```tsx
// ❌ 不推荐
<div className="bg-#F7F8FA text-#86909C border-#E5E6EB">

// ✅ 推荐
<div className="bg-1 text-t-secondary border-[var(--border-base)]">
```

## 🎯 快速参考

- **背景**: `bg-base`, `bg-1`, `bg-2`, `bg-3`
- **文字**: `text-t-primary`, `text-t-secondary`, `text-t-disabled`
- **边框**: `border-[var(--border-base)]`, `border-arco-1` ~ `border-arco-4`, `border-3`
  （❌ 不是 `border-b-base` / `border-b-light` / `border-border-N`）
- **品牌**: `bg-brand`, `bg-brand-light`, `bg-brand-hover`
- **状态**: `bg-primary`, `bg-success`, `bg-warning`, `bg-danger`
- **状态色阶**: `text-danger-6`, `bg-primary-1` ~ `bg-primary-9`
  （❌ 不是 `text-[rgb(var(--danger-6))]`）
- **AOU色板**: `bg-aou-1` ~ `bg-aou-10`
