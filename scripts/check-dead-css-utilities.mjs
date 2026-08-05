#!/usr/bin/env node
/**
 * 死 CSS 工具类禁令 / Dead CSS utility-class prohibition
 *
 * 四种写法在本仓库编译不出（或编译错）CSS。全部用真实 UnoCSS 生成器实测过，
 * 不是推测。All four forms below were measured with the real UnoCSS generator
 * (ui/uno.config.ts), not guessed.
 *
 * 这个检查曾是「棘轮」：存量 95 个文件 / 276 处记在 BASELINE 里被放行。存量已
 * 全部清理完毕，BASELINE 随之删除——现在是一刀切的禁令，任何一处都会失败。
 * This used to be a ratchet with a 95-file / 276-occurrence BASELINE. The sweep
 * is done and the baseline is gone: any occurrence anywhere now fails.
 *
 * 1) ramp —— `{text,bg,border}-[rgb(var(--RAMP-N))]`。UnoCSS 把中括号里的值当颜色
 *    处理并注入 slash-alpha：
 *      color: rgb(var(--danger-6) / var(--un-text-opacity))
 *    而这些 ramp 变量是「逗号分隔三元组」（Arco `--red-6: 245,63,63`，预设写
 *    `--primary-6: 232, 23, 74;`），于是 `rgb(245,63,63 / 1)` 无法解析，浏览器
 *    整条声明作废——元素静默保留继承来的颜色。
 *    ✅ primary/success/warning/danger 改用项目自带规则：`text-danger-6` /
 *       `bg-primary-6` / `border-success-5`（uno.config.ts 的
 *       ^(bg|text|border)-(primary|success|warning|danger)-([1-9])$ 规则输出合法的
 *       `color: rgb(var(--danger-6))`）。
 *    ✅ 其它色板（link / arcoblue / purple / orange / cyan / gray …）没有项目规则，
 *       写成自带 alpha 的 `text-[rgba(var(--arcoblue-6),1)]`，UnoCSS 就不再注入。
 *    ⚠️ 显式 `rgba(var(--x), 0.12)` 自带 alpha，是合法写法，本检查不拦。
 *    ⚠️ 注意本条也覆盖 `bg-[rgb(var(--success-6))]/12` 这种「中括号 + 斜杠透明度」
 *       的变体——它同样产出非法的 `rgb(... / 0.12)`。
 *
 * 2) deadBorder —— `border-border-N`。theme 里没有名为 `border` 的颜色，
 *    完全不产出 CSS。✅ 改用 `border-arco-N`（Arco --color-border-N）或
 *    `border-N`（--bg-N 色阶）或 `border-[var(--border-base)]`。
 *
 * 3) bottomBorder —— `border-b-base` / `border-b-light`。UnoCSS 先把 `-b-` 解析成
 *    bottom 方向，再拿剩下的键查 theme，所以产出的是基于 --bg-* 的
 *    `border-bottom-color`，而不是本意的基础边框色。
 *    ✅ 四边：`border border-solid border-[var(--border-base)]`；
 *       仅下边框：`border-b border-b-solid border-b-[var(--border-base)]`。
 *
 * 4) rampSlash —— `{bg,text,border}-(primary|success|warning|danger)-N/NN`。项目规则
 *    以 `$` 结尾锚定，末尾的 `/NN` 让整个类名一条规则都匹配不上，产出 0 条 CSS。
 *    这正是「把写法 1 机械 sed 成项目规则类」时最容易踩的坑（`bg-success-6/12`）。
 *    ✅ 需要透明度就写 `bg-[rgba(var(--success-6),0.12)]`。
 *
 * 扫描范围：ui/src 下的 .ts/.tsx/.css，排除 *.test.ts(x) 与 *.d.ts —— 测试文件里
 * 这些字符串是「断言源码里不含/曾含某写法」的字面量，不会渲染成 CSS。.md 同理不扫，
 * 迁移指南 ui/src/renderer/styles/MIGRATION.md 需要把错误写法作为反例展示。
 *
 * 用法 / Usage:
 *   bun scripts/check-dead-css-utilities.mjs             # 校验，发现违规 exit 1
 *   bun scripts/check-dead-css-utilities.mjs --self-test # 校验器自测
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SCAN_DIR = join(ROOT, 'ui', 'src');
const SELF = 'scripts/check-dead-css-utilities.mjs';

/** 四种死写法及其有效替代 / The four dead forms and their valid replacements */
const FORMS = {
  ramp: {
    // 只匹配整个中括号值恰好是 rgb(var(--name-N)) 的情形；rgba(...) 与
    // shadow-[...rgb(var(--x))...] 都不匹配。
    //   * 色板名不限于四个语义 ramp：arcoblue / purple / orange / cyan / gray 等
    //     Arco 色板同样是逗号三元组，犯的是同一个错。
    //   * 前缀也不限于 text/bg/border：ring- / outline- / border-t- / border-b- /
    //     divide- … 任何走 UnoCSS 颜色管道的工具类都会被注入 slash-alpha。
    //     实测证据：`ring-[rgb(var(--primary-6))]` 产出
    //     `--un-ring-color: rgb(var(--primary-6) / var(--un-ring-opacity))`。
    re: /(?<![\w-])[a-z]+(?:-[a-z])?-\[rgb\(var\(--[a-z]+-\d\)\)\]/g,
    label: '<任意颜色工具类>-[rgb(var(--RAMP-N))] 注入 slash-alpha，整条声明被浏览器丢弃',
    fix: 'primary/success/warning/danger 的 text/bg/border 改用 text-danger-6 / bg-primary-6 这类项目规则类；其它色板、或 ring-/outline-/border-t- 这些规则覆盖不到的前缀，写自带 alpha 的 ring-[rgba(var(--primary-6),1)]；需要透明度写 rgba(var(--primary-6), 0.12)',
  },
  deadBorder: {
    // 后缀不限于数字：theme 里根本没有名为 `border` 的颜色，所以 `border-border-base`
    // 与 `border-border-2` 一样产出 0 条 CSS。这条规则最初只写了 `\d`，于是
    // HTMLViewer.tsx 里 3 处 `border-border-base` 从棘轮时代一路漏到禁令时代。
    // The suffix is NOT limited to digits: there is no colour named `border` in the
    // theme at all, so `border-border-base` emits zero CSS exactly like
    // `border-border-2`. This regex used to be `\d`-anchored, which is how 3 live
    // `border-border-base` sites survived both the ratchet and the sweep.
    re: /\bborder-border-[a-z0-9]+\b/g,
    label: 'border-border-*：theme 无 border 颜色，产出 0 条 CSS',
    fix: '改用 border-arco-N（--color-border-N）或 border-N（--bg-N）或 border-[var(--border-base)]；注意本仓库没有全局 border-style 重置，宽度类要配 border-solid 才画得出来',
  },
  bottomBorder: {
    re: /\bborder-b-(?:base|light)\b/g,
    label: 'border-b-base / border-b-light：-b- 先被解析成 bottom 方向，落到 --bg-* 上',
    fix: '四边写 border border-solid border-[var(--border-base)]；仅下边框写 border-b border-b-solid border-b-[var(--border-base)]',
  },
  rampSlash: {
    re: /\b(?:bg|text|border)-(?:primary|success|warning|danger)-[1-9]\/\d+\b/g,
    label: '{bg,text,border}-RAMP-N/NN：项目规则以 $ 锚定，斜杠透明度让它一条规则都匹配不上，产出 0 条 CSS',
    fix: '需要透明度改写成 bg-[rgba(var(--success-6),0.12)]',
  },
};

function* walk(dir) {
  for (const name of readdirSync(dir).sort()) {
    if (name === 'node_modules' || name === 'dist' || name.startsWith('.')) continue;
    const full = join(dir, name);
    if (statSync(full).isDirectory()) yield* walk(full);
    else if (/\.(tsx?|css)$/.test(name) && !/\.test\.tsx?$/.test(name) && !/\.d\.ts$/.test(name)) yield full;
  }
}

/** 逐行定位，便于报错时给出坐标 / Locate matches with 1-based line numbers */
function scanSource(source) {
  const hits = [];
  const lines = source.split('\n');
  for (const [form, { re }] of Object.entries(FORMS)) {
    lines.forEach((line, i) => {
      for (const m of line.matchAll(re)) hits.push({ form, line: i + 1, snippet: m[0] });
    });
  }
  return hits;
}

function selfTest() {
  const cases = [
    // 干净写法 / clean forms — 一条都不许命中
    { src: "<div className='text-danger-6 bg-primary-6 border-success-5' />", bad: 0 },
    { src: "<div className='border-arco-2 border-3 border-[var(--border-base)]' />", bad: 0 },
    { src: "<div className='border-b border-b-solid border-b-[var(--border-base)]' />", bad: 0 },
    // 显式 alpha 自带透明度，合法 / explicit alpha carries its own opacity
    { src: "<div className='bg-[rgba(var(--primary-6),0.12)]' />", bad: 0 },
    { src: "<div className='text-[rgba(var(--arcoblue-6),1)]' />", bad: 0 },
    { src: "<div className='shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.22)]' />", bad: 0 },
    // 非颜色属性里的 rgb() 不走颜色管道，合法 / rgb() in a shadow is fine
    { src: "<div className='shadow-[0_0_8px_rgb(var(--warning-6))]' />", bad: 0 },
    { src: "<div className='shadow-[0_4px_14px_rgba(var(--primary-6),0.32)]' />", bad: 0 },
    // 违规写法 / violations
    { src: "<div className='text-[rgb(var(--danger-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='bg-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='focus-visible:border-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='text-[rgb(var(--link-6))] bg-[rgb(var(--warning-5))]' />", bad: 2, form: 'ramp' },
    // 非语义色板同样是逗号三元组，同一个错 / Arco palettes break identically
    { src: "<div className='!text-[rgb(var(--arcoblue-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='bg-[rgb(var(--gray-2))]' />", bad: 1, form: 'ramp' },
    // text/bg/border 之外的颜色前缀也一样被注入 / other colour prefixes too
    { src: "<div className='ring-2 ring-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='focus-visible:outline-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='border-t-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='!border-b-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    // ✅ 这些前缀没有项目规则，自带 alpha 才是正解 / explicit alpha is the fix there
    { src: "<div className='ring-[rgba(var(--primary-6),1)] border-t-[rgba(var(--primary-6),1)]' />", bad: 0 },
    // 中括号 + 斜杠透明度，同样产出非法的 rgb(... / .12)
    { src: "<div className='bg-[rgb(var(--success-6))]/12' />", bad: 1, form: 'ramp' },
    { src: "<div className='border-b border-border-1' />", bad: 1, form: 'deadBorder' },
    { src: "<div className='border border-dashed border-border-2' />", bad: 1, form: 'deadBorder' },
    // 非数字后缀同样死：theme 里没有 `border` 这个颜色，后面跟什么都产出 0 条 CSS
    // Non-numeric suffixes are dead too — there is no `border` colour to index into.
    { src: "<div className='border-b border-border-base' />", bad: 1, form: 'deadBorder' },
    { src: "<div className='border border-border-light' />", bad: 1, form: 'deadBorder' },
    // ✅ 正解：颜色用 arbitrary value，并且补上 border-style
    { src: "<div className='border-b border-b-solid border-b-[var(--border-base)]' />", bad: 0 },
    { src: "<div className='border-r border-r-solid border-r-[var(--border-base)]' />", bad: 0 },
    { src: "<div className='border-b-base' />", bad: 1, form: 'bottomBorder' },
    { src: "<div className='border-t border-b-light' />", bad: 1, form: 'bottomBorder' },
    // 项目规则类 + 斜杠透明度 = 匹配不上任何规则，产出 0 条 CSS
    { src: "<div className='bg-success-6/12' />", bad: 1, form: 'rampSlash' },
    { src: "<div className='hover:!text-danger-6/50' />", bad: 1, form: 'rampSlash' },
    // 混合 / mixed
    { src: "<div className='border-border-3 text-[rgb(var(--success-6))] border-b-base' />", bad: 3 },
  ];
  let failed = 0;
  cases.forEach(({ src, bad, form }, i) => {
    const hits = scanSource(src);
    if (hits.length !== bad) {
      failed += 1;
      console.error(`self-test case ${i} failed: expected ${bad} violation(s), got ${hits.length}\n  ${src}`);
      return;
    }
    if (form && hits.some((h) => h.form !== form)) {
      failed += 1;
      console.error(`self-test case ${i} failed: expected form "${form}", got ${[...new Set(hits.map((h) => h.form))]}`);
    }
  });

  const total = cases.length;
  if (failed > 0) {
    console.error(`❌ check-dead-css-utilities self-test: ${failed}/${total} case(s) failed`);
    process.exit(1);
  }
  console.log(`✅ check-dead-css-utilities self-test: ${total}/${total} cases pass`);
}

if (process.argv.includes('--self-test')) {
  selfTest();
  process.exit(0);
}

const errors = [];
let scanned = 0;
for (const abs of walk(SCAN_DIR)) {
  const file = relative(ROOT, abs).split('\\').join('/');
  scanned += 1;
  const hits = scanSource(readFileSync(abs, 'utf8'));
  if (hits.length) {
    errors.push([`${file} 使用了死 CSS 工具类:`, ...hits.map((h) => `    ${file}:${h.line}  ${h.snippet}`)].join('\n'));
  }
}

if (errors.length) {
  console.error(`\n❌ 死 CSS 工具类检查未通过（详见 ${SELF} 头注 / ui/src/renderer/styles/MIGRATION.md）:\n`);
  for (const e of errors) console.error(`  ${e}`);
  console.error('\n  有效替代 / Valid replacements:');
  for (const [form, { label, fix }] of Object.entries(FORMS)) {
    console.error(`    [${form}] ${label}\n        → ${fix}`);
  }
  process.exit(1);
}

console.log(`✅ dead CSS utilities clean (${scanned} file(s) scanned, ${Object.keys(FORMS).length} banned form(s), no baseline)`);
