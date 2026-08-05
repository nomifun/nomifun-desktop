#!/usr/bin/env node
/**
 * 死 CSS 工具类棘轮 / Dead CSS utility-class ratchet
 *
 * 三种写法在本仓库编译不出（或编译错）CSS。全部用真实 UnoCSS 生成器实测过，
 * 不是推测。All three forms below were measured with the real UnoCSS generator
 * (ui/uno.config.ts), not guessed.
 *
 * 1) ramp —— `{text,bg,border}-[rgb(var(--RAMP-N))]`，RAMP ∈ primary|danger|
 *    success|warning|link。UnoCSS 把中括号里的值当颜色处理并注入 slash-alpha：
 *      color: rgb(var(--danger-6) / var(--un-text-opacity))
 *    而这些 ramp 变量是「逗号分隔三元组」（Arco `--red-6: 245,63,63`，预设写
 *    `--primary-6: 232, 23, 74;`），于是 `rgb(245,63,63 / 1)` 无法解析，浏览器
 *    整条声明作废——元素静默保留继承来的颜色。
 *    ✅ 改用项目自带规则：`text-danger-6` / `bg-primary-6` / `border-success-5`
 *       （uno.config.ts 的 ^(bg|text|border)-(primary|success|warning|danger)-([1-9])$
 *       规则输出合法的 `color: rgb(var(--danger-6))`）。
 *    ⚠️ 显式 `rgba(var(--x), 0.12)` 自带 alpha，不会被注入，是合法写法，本检查不拦。
 *
 * 2) deadBorder —— `border-border-N`。theme 里没有名为 `border` 的颜色，
 *    完全不产出 CSS。✅ 改用 `border-arco-N`（Arco --color-border-N）或
 *    `border-N`（--bg-N 色阶）或 `border-[var(--border-base)]`。
 *
 * 3) bottomBorder —— `border-b-base` / `border-b-light`。UnoCSS 先把 `-b-` 解析成
 *    bottom 方向，再拿剩下的键查 theme，所以产出的是基于 --bg-* 的
 *    `border-bottom-color`，而不是本意的基础边框色。
 *    ✅ 改用 `border-[var(--border-base)]` / `border-b-[var(--border-base)]`。
 *
 * 棘轮语义 / Ratchet semantics（不是一刀切 / not a big bang）:
 *   - 存量 95 个文件记在下面的 BASELINE 里，保持原样，不算失败。
 *   - 失败条件：① 不在 BASELINE 的文件出现任一写法；② BASELINE 文件的条数变多；
 *     ③ BASELINE 文件已清零（或文件已不存在）——必须把它从 BASELINE 删掉，
 *     这样这张表只会变短。
 *   - 条数变少（但没清零）只提示不失败，方便分批清理。
 *
 * 扫描范围：ui/src 下的 .ts/.tsx/.css，排除 *.test.ts(x) 与 *.d.ts —— 测试文件里
 * 这些字符串是「断言源码里不含某写法」的字面量，不会渲染成 CSS。.md 同理不扫，
 * 迁移指南 ui/src/renderer/styles/MIGRATION.md 需要把错误写法作为反例展示。
 *
 * 用法 / Usage:
 *   bun scripts/check-dead-css-utilities.mjs             # 校验，发现违规 exit 1
 *   bun scripts/check-dead-css-utilities.mjs --self-test # 校验器自测
 */
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SCAN_DIR = join(ROOT, 'ui', 'src');
const SELF = 'scripts/check-dead-css-utilities.mjs';

/** 三种死写法及其有效替代 / The three dead forms and their valid replacements */
const FORMS = {
  ramp: {
    // 只匹配整个中括号值恰好是 rgb(var(--ramp-N)) 的情形；rgba(...) 与
    // shadow-[...rgb(var(--x))...] 都不匹配。
    re: /(?:text|bg|border)-\[rgb\(var\(--(?:primary|danger|success|warning|link)-\d\)\)\]/g,
    label: '{text,bg,border}-[rgb(var(--RAMP-N))] 注入 slash-alpha，整条声明被浏览器丢弃',
    fix: '改用 text-danger-6 / bg-primary-6 / border-success-5 这类项目规则类；需要透明度时写 rgba(var(--primary-6), 0.12)',
  },
  deadBorder: {
    re: /\bborder-border-\d\b/g,
    label: 'border-border-N：theme 无 border 颜色，产出 0 条 CSS',
    fix: '改用 border-arco-N（--color-border-N）或 border-N（--bg-N）或 border-[var(--border-base)]',
  },
  bottomBorder: {
    re: /\bborder-b-(?:base|light)\b/g,
    label: 'border-b-base / border-b-light：-b- 先被解析成 bottom 方向，落到 --bg-* 上',
    fix: '改用 border-[var(--border-base)]（四边）或 border-b-[var(--border-base)]（仅下边框）',
  },
};

/**
 * 存量基线 / Pre-existing baseline（HEAD 实测：ramp 79 文件 / 228 处，
 * deadBorder 17 文件 / 40 处，bottomBorder 4 文件 / 8 处，去重后共 95 个文件）。
 * 这张表只允许变短：清理干净一个文件就删掉它对应的一行。
 * This table may only shrink: delete a row once its file is clean.
 */
const BASELINE = new Map([
  ['ui/src/renderer/components/base/NomiSelect.tsx', { deadBorder: 1 }],
  ['ui/src/renderer/components/layout/Sider/CompanionAccessTokenPanel.tsx', { ramp: 4 }],
  ['ui/src/renderer/components/layout/Sider/SiderNav/SiderWorkshopEntry.tsx', { ramp: 1 }],
  ['ui/src/renderer/components/layout/Sider/WebuiControlPanel.tsx', { ramp: 1, deadBorder: 1 }],
  ['ui/src/renderer/components/media/WebviewHost.tsx', { deadBorder: 1 }],
  ['ui/src/renderer/components/settings/SettingsModal/contents/FeedbackReportModal.tsx', { deadBorder: 2 }],
  ['ui/src/renderer/components/settings/SettingsModal/contents/ModelModalContent.tsx', { ramp: 9 }],
  ['ui/src/renderer/components/settings/SettingsModal/contents/SystemModalContent/index.tsx', { ramp: 1 }],
  ['ui/src/renderer/components/settings/SettingsModal/contents/ToolsModalContent.tsx', { deadBorder: 2 }],
  ['ui/src/renderer/components/settings/UpdateModal.tsx', { ramp: 15, deadBorder: 1 }],
  ['ui/src/renderer/components/workspace/WorkspaceFolderSelect.tsx', { deadBorder: 3 }],
  ['ui/src/renderer/pages/assets/index.tsx', { ramp: 6 }],
  ['ui/src/renderer/pages/conversation/Preview/components/PreviewPanel/PreviewToolbar.tsx', { deadBorder: 1 }],
  ['ui/src/renderer/pages/conversation/Preview/components/viewers/MarkdownViewer.tsx', { deadBorder: 1 }],
  ['ui/src/renderer/pages/conversation/SessionList/ConversationRow.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/conversation/SessionList/TerminalRow.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/conversation/SessionList/WorkpathDrawer.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/conversation/Workspace/components/FileChangeList.tsx', { bottomBorder: 3 }],
  ['ui/src/renderer/pages/conversation/Workspace/components/WorkspaceToolbar.tsx', { bottomBorder: 1 }],
  ['ui/src/renderer/pages/conversation/components/AutoWorkControl.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/conversation/components/ChatTitleEditor.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/conversation/components/ConversationTerminalPanel.tsx', { ramp: 5 }],
  ['ui/src/renderer/pages/conversation/components/ConversationTitleMinimap/index.tsx', { ramp: 5 }],
  ['ui/src/renderer/pages/conversation/components/IdmmControl.tsx', { ramp: 4 }],
  ['ui/src/renderer/pages/conversation/components/KnowledgeControl.tsx', { ramp: 6 }],
  ['ui/src/renderer/pages/conversation/components/SummonPanel/index.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/conversation/execution/ExecutionControls.tsx', { bottomBorder: 3 }],
  ['ui/src/renderer/pages/conversation/execution/StepModelPill.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/conversation/platforms/nomi/NomiSessionMetricsPanel.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/conversation/platforms/openclaw/StarOfficeMonitorCard.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/cron/ScheduledTasksPage/ScheduledTaskActions.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/cron/ScheduledTasksPage/TaskDetailPage.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/cron/ScheduledTasksPage/index.tsx', { ramp: 1, deadBorder: 1 }],
  ['ui/src/renderer/pages/cron/components/CronJobManager.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/customerService/CsAgentDetailPage.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/customerService/index.tsx', { ramp: 4 }],
  ['ui/src/renderer/pages/guid/components/QuickActionButtons.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/knowledge/CreateStudio/SourceConfig.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/knowledge/CreateStudio/TeachingCard.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/knowledge/CreateStudio/index.tsx', { ramp: 5 }],
  ['ui/src/renderer/pages/knowledge/KnowledgeCard.tsx', { ramp: 5 }],
  ['ui/src/renderer/pages/knowledge/KnowledgeConsumersSection.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/knowledge/KnowledgeDetailPage/index.tsx', { ramp: 8 }],
  ['ui/src/renderer/pages/knowledge/KnowledgeEmptyState.tsx', { ramp: 4 }],
  ['ui/src/renderer/pages/knowledge/KnowledgeListPage/index.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/knowledge/KnowledgeTagFilterBar.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/knowledge/KnowledgeTagManagementModal.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/knowledge/knowledgeKind.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/mcp/PluginSettingsPanel.tsx', { ramp: 1, deadBorder: 1 }],
  ['ui/src/renderer/pages/modelHub/FreeModelsContent.tsx', { ramp: 6 }],
  ['ui/src/renderer/pages/openCapabilities/index.tsx', { deadBorder: 11 }],
  ['ui/src/renderer/pages/requirements/RequirementDrawer/AttachmentsField.tsx', { deadBorder: 3 }],
  ['ui/src/renderer/pages/requirements/RequirementDrawer/index.tsx', { deadBorder: 2 }],
  ['ui/src/renderer/pages/requirements/SourcesPage/SourceCard.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/requirements/WorkspacePage/RequirementBoardCard.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/requirements/WorkspacePage/RequirementFilters.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/requirements/WorkspacePage/RequirementListRow.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/requirements/components/RequirementDisplayNumber.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/settings/AgentSettings/RemoteAgentManagement.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/settings/DisplaySettings/CssThemeModal.tsx', { deadBorder: 2 }],
  ['ui/src/renderer/pages/settings/PresetSettings/PresetEditDrawer.tsx', { ramp: 1, deadBorder: 4 }],
  ['ui/src/renderer/pages/settings/PresetSettings/PresetTagFilterBar.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/settings/PresetSettings/PresetTagPicker.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/settings/PresetSettings/TagManagementModal.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/settings/SkillsHubSettings.tsx', { bottomBorder: 1 }],
  ['ui/src/renderer/pages/settings/components/AddModelModal.tsx', { deadBorder: 3 }],
  ['ui/src/renderer/pages/settings/components/AddPlatformModal.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/settings/components/ModelAdvancedEditor.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/settings/components/ProviderConnectionsSection.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/settings/skill/AgentSkillImportDrawer.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/settings/skill/SkillCard.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/settings/skill/SkillDetailDrawer.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/settings/skill/SkillMarketCard.tsx', { ramp: 4 }],
  ['ui/src/renderer/pages/workshop/CanvasPage.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/workshop/assets/AssetCard.tsx', { ramp: 4 }],
  ['ui/src/renderer/pages/workshop/assets/AssetDetailModal.tsx', { ramp: 4 }],
  ['ui/src/renderer/pages/workshop/assets/AssetLibraryControls.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/workshop/assets/AssetsPanel.tsx', { ramp: 4 }],
  ['ui/src/renderer/pages/workshop/canvas/nodes/CompareNode.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/canvas/nodes/GroupNode.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/canvas/nodes/ImageNode.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/canvas/nodes/LoopNode.tsx', { ramp: 8 }],
  ['ui/src/renderer/pages/workshop/canvas/nodes/OutputNode.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/canvas/nodes/VideoNode.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/canvas/nodes/nodeShared.tsx', { ramp: 3 }],
  ['ui/src/renderer/pages/workshop/canvas/overlays/CanvasToolbar.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/canvas/overlays/FloatingMenu.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/canvas/overlays/ShortcutsHelp.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/generation/GeneratorCard.tsx', { ramp: 6 }],
  ['ui/src/renderer/pages/workshop/generation/InputSummary.tsx', { ramp: 1 }],
  ['ui/src/renderer/pages/workshop/generation/ModelPicker.tsx', { ramp: 5 }],
  ['ui/src/renderer/pages/workshop/generation/ParamControls.tsx', { ramp: 5 }],
  ['ui/src/renderer/pages/workshop/generation/PromptField.tsx', { ramp: 2 }],
  ['ui/src/renderer/pages/workshop/generation/ResultView.tsx', { ramp: 7 }],
  ['ui/src/renderer/pages/workshop/index.tsx', { ramp: 5 }],
]);

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

/** 把命中折叠成 { form: count } / Fold hits into per-form counts */
function countsOf(hits) {
  const counts = {};
  for (const h of hits) counts[h.form] = (counts[h.form] ?? 0) + 1;
  return counts;
}

function selfTest() {
  const cases = [
    // 干净写法 / clean forms — 一条都不许命中
    { src: "<div className='text-danger-6 bg-primary-6 border-success-5' />", bad: 0 },
    { src: "<div className='border-arco-2 border-3 border-[var(--border-base)]' />", bad: 0 },
    { src: "<div className='border-b-[var(--border-base)] border-b border-b-solid' />", bad: 0 },
    // 显式 alpha 自带透明度，合法 / explicit alpha carries its own opacity
    { src: "<div className='bg-[rgba(var(--primary-6),0.12)]' />", bad: 0 },
    { src: "<div className='shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.22)]' />", bad: 0 },
    // 违规写法 / violations
    { src: "<div className='text-[rgb(var(--danger-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='bg-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='focus-visible:border-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='text-[rgb(var(--link-6))] bg-[rgb(var(--warning-5))]' />", bad: 2, form: 'ramp' },
    { src: "<div className='border-b border-border-1' />", bad: 1, form: 'deadBorder' },
    { src: "<div className='border border-dashed border-border-2' />", bad: 1, form: 'deadBorder' },
    { src: "<div className='border-b-base' />", bad: 1, form: 'bottomBorder' },
    { src: "<div className='border-t border-b-light' />", bad: 1, form: 'bottomBorder' },
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

  // 棘轮判定自测：用合成基线，不依赖真实 BASELINE 数据（真实基线会随清理变化）
  // Ratchet verdicts against a synthetic baseline so cleanup sweeps can shrink
  // the real BASELINE without breaking this self-test.
  const fakeBaseline = new Map([['ui/src/fake/Baselined.tsx', { ramp: 2, deadBorder: 1 }]]);
  const verdicts = [
    { file: 'ui/src/fake/New.tsx', counts: { ramp: 1 }, want: 'new' },
    { file: 'ui/src/fake/New.tsx', counts: {}, want: 'ok' },
    { file: 'ui/src/fake/Baselined.tsx', counts: { ramp: 3, deadBorder: 1 }, want: 'regression' },
    { file: 'ui/src/fake/Baselined.tsx', counts: {}, want: 'stale' },
    { file: 'ui/src/fake/Baselined.tsx', counts: { ramp: 1, deadBorder: 1 }, want: 'shrunk' },
    { file: 'ui/src/fake/Baselined.tsx', counts: { ramp: 2, deadBorder: 1 }, want: 'ok' },
    // 换一种写法但总数不变，也算变多（按写法逐一比较，不是比总数）
    { file: 'ui/src/fake/Baselined.tsx', counts: { ramp: 2, bottomBorder: 1 }, want: 'regression' },
  ];
  for (const { file, counts, want } of verdicts) {
    const got = verdictFor(file, counts, fakeBaseline).kind;
    if (got !== want) {
      failed += 1;
      console.error(`self-test ratchet failed for ${file}: expected "${want}", got "${got}"`);
    }
  }

  const total = cases.length + verdicts.length;
  if (failed > 0) {
    console.error(`❌ check-dead-css-utilities self-test: ${failed}/${total} case(s) failed`);
    process.exit(1);
  }
  console.log(`✅ check-dead-css-utilities self-test: ${total}/${total} cases pass`);
}

/** 单文件棘轮判定 / Ratchet verdict for one file */
function verdictFor(file, counts, baseline = BASELINE) {
  const base = baseline.get(file);
  const total = Object.values(counts).reduce((a, b) => a + b, 0);
  if (!base) return total > 0 ? { kind: 'new' } : { kind: 'ok' };
  if (total === 0) return { kind: 'stale' };
  const worse = [];
  const better = [];
  for (const form of Object.keys(FORMS)) {
    const now = counts[form] ?? 0;
    const was = base[form] ?? 0;
    if (now > was) worse.push({ form, was, now });
    else if (now < was) better.push({ form, was, now });
  }
  if (worse.length) return { kind: 'regression', worse };
  if (better.length) return { kind: 'shrunk', better };
  return { kind: 'ok' };
}

if (process.argv.includes('--self-test')) {
  selfTest();
  process.exit(0);
}

const found = new Map();
let scanned = 0;
for (const abs of walk(SCAN_DIR)) {
  const file = relative(ROOT, abs).split('\\').join('/');
  const source = readFileSync(abs, 'utf8');
  scanned += 1;
  const hits = scanSource(source);
  if (hits.length || BASELINE.has(file)) found.set(file, { hits, counts: countsOf(hits) });
}

const errors = [];
const notes = [];

for (const [file, { hits, counts }] of found) {
  const v = verdictFor(file, counts);
  if (v.kind === 'new') {
    errors.push(
      [`${file} 引入了死 CSS 工具类（该文件不在基线里）:`, ...hits.map((h) => `    ${file}:${h.line}  ${h.snippet}`)].join('\n')
    );
  } else if (v.kind === 'regression') {
    errors.push(
      [
        `${file} 的死 CSS 工具类变多了（基线只允许变少）:`,
        ...v.worse.map((w) => `    ${w.form}: 基线 ${w.was} → 现在 ${w.now}`),
      ].join('\n')
    );
  } else if (v.kind === 'shrunk') {
    notes.push(
      `${file} 已部分清理，可把基线收紧: ${v.better.map((b) => `${b.form} ${b.was} → ${b.now}`).join(', ')}`
    );
  }
}

// 基线里已清零 / 已消失的条目必须删除，保证这张表只会变短。
for (const file of BASELINE.keys()) {
  if (!existsSync(join(ROOT, file))) {
    errors.push(`${file} 已不存在，请把它从 ${SELF} 的 BASELINE 里删掉（这张表只能变短）。`);
  } else if (!found.has(file) || verdictFor(file, found.get(file).counts).kind === 'stale') {
    errors.push(`${file} 已清理干净 🎉 请把它从 ${SELF} 的 BASELINE 里删掉（这张表只能变短）。`);
  }
}

for (const n of notes) console.log(`ℹ️  ${n}`);

if (errors.length) {
  console.error(`\n❌ 死 CSS 工具类检查未通过（详见 ${SELF} 头注 / ui/src/renderer/styles/MIGRATION.md）:\n`);
  for (const e of errors) console.error(`  ${e}`);
  console.error('\n  有效替代 / Valid replacements:');
  for (const [form, { label, fix }] of Object.entries(FORMS)) {
    console.error(`    [${form}] ${label}\n        → ${fix}`);
  }
  process.exit(1);
}

const baselineTotal = [...BASELINE.values()].reduce((a, c) => a + Object.values(c).reduce((x, y) => x + y, 0), 0);
console.log(
  `✅ dead CSS utilities clean (${scanned} file(s) scanned; baseline holds ${BASELINE.size} file(s) / ${baselineTotal} occurrence(s), no new or worsened use)`
);
