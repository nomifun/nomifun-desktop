#!/usr/bin/env node
/**
 * Creative Studio legacy-surface retirement gate.
 *
 * Canvas and asset resources keep their domain/storage contracts. The former
 * product shell and independent generation pages must not return. The only
 * legacy browser values retained are one-way draft/resume import formats.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const DIST = join(ROOT, 'ui', 'dist');
const CHECK_DIST = process.argv.includes('--dist');

const LEGACY_TRACKED_PATHS = [
  /^ui\/src\/renderer\/pages\/creativeStudio\/workbenches\//,
  /^ui\/src\/renderer\/pages\/creativeStudio\/app\/(?:CreativeStudioFocusShell|CreativeStudioSider|resumeLocation|routes)(?:\.|\/)/,
  /^ui\/src\/renderer\/pages\/creativeStudio\/tasks\/historyClient(?:\.|\/)/,
  /^ui\/src\/renderer\/utils\/workspace\/workbenchSiderEvents\.ts$/,
  /^ui\/src\/renderer\/components\/layout\/Sider\/SiderNav\/SiderCreativeStudioEntry\.tsx$/,
  /^ui\/src\/renderer\/pages\/(?:workshop|assets)\//,
  /^ui\/src\/renderer\/components\/layout\/Sider\/SiderNav\/SiderWorkshopEntry\.tsx$/,
  /^ui\/src\/renderer\/services\/i18n\/locales\/(?:en-US|zh-CN)\/workshop(?:Canvas|Assets|Editor|Generation|Agent)?\.json$/,
];

const RUNTIME_MARKERS = [
  { label: 'retired independent generation import', pattern: /workbenches\/(?:image|video|audio|runtime|history|drafts|product)(?:\/|['"])/g },
  { label: 'retired product shell', pattern: /\b(?:CreativeStudioFocusShell|CreativeStudioSider|SiderCreativeStudioEntry|ImageWorkbenchProductRoute|VideoWorkbenchProductRoute)\b/g },
  { label: 'retired independent history API', pattern: /\b(?:listStandalone|retireStandalone|creativeTaskHistoryClient)\b/g },
  { label: 'retired product route', pattern: /<Route\s+path=['"]\/workshop(?:\/[^'"]*)?['"]/g },
  { label: 'retired page import', pattern: /pages\/(?:workshop|assets)(?:\/|['"])/g },
  { label: 'retired sidebar component', pattern: /SiderWorkshopEntry/g },
  { label: 'retired top-level asset route', pattern: /(['"])\/assets\1/g },
  { label: 'retired canvas route pattern', pattern: /(['"])\/workshop\/:id\1/g },
  { label: 'retired canvas template route', pattern: /`\/workshop\/\$\{/g },
  {
    label: 'retired translation namespace',
    pattern: /\b(?:t|i18n\.t)\(\s*['"]workshop(?:Canvas|Assets|Editor|Generation|Agent)?\./g,
  },
  { label: 'retired HTTP namespace', pattern: /\/api\/workshop(?:\/|['"`])/g },
  {
    label: 'retired unowned creation task API',
    pattern: /\/api\/creation\/tasks(?:[/?]|['"`])/g,
  },
  { label: 'retired Gateway tool name', pattern: /nomi_workshop_[A-Za-z0-9_]+/g },
];

const DIST_MARKERS = [
  { label: 'retired sidebar component', pattern: /SiderWorkshopEntry/g },
  {
    label: 'retired translation namespace',
    pattern: /workshop(?:Canvas|Assets|Editor|Generation|Agent)\.|workshop\.beta/g,
  },
  { label: 'retired canvas route pattern', pattern: /\/workshop\/:id/g },
  { label: 'retired HTTP namespace', pattern: /\/api\/workshop\//g },
  { label: 'retired unowned creation task API', pattern: /\/api\/creation\/tasks(?:[/?'"`])/g },
  { label: 'retired Gateway tool name', pattern: /nomi_workshop_[A-Za-z0-9_]+/g },
];

const BACKEND_MARKERS = [
  {
    label: 'retired standalone task owner or API',
    pattern: /\b(?:StandaloneWorkbenchKind|StandaloneWorkbench|CreationWorkbenchKind|ListStandaloneWorkbenchTasksParams|RetireStandaloneWorkbenchTasksParams|list_standalone_workbench_tasks(?:_page)?|retire_standalone_workbench_tasks)\b/g,
  },
];

const trackedFiles = () =>
  execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], {
    cwd: ROOT,
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  })
    .split('\0')
    .filter(Boolean);

const isRuntimeSource = (path) =>
  path.startsWith('ui/src/renderer/') &&
  /\.(?:[cm]?[jt]sx?)$/.test(path) &&
  !/\.(?:test|spec)\.[cm]?[jt]sx?$/.test(path) &&
  !path.includes('/__tests__/');

const lineOf = (source, index) => source.slice(0, index).split('\n').length;

const scanSource = (path, source, markers) => {
  const violations = [];
  for (const { label, pattern } of markers) {
    pattern.lastIndex = 0;
    for (const match of source.matchAll(pattern)) {
      violations.push({ path, line: lineOf(source, match.index), label, token: match[0] });
    }
  }
  return violations;
};

function* walkFiles(directory) {
  for (const name of readdirSync(directory)) {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) yield* walkFiles(path);
    else yield path;
  }
}

// `git ls-files` keeps unstaged deletions in the index. Ignore paths that no
// longer exist in the worktree so this gate can validate legitimate removals
// before the contributor stages or commits them.
const files = trackedFiles().filter((path) => existsSync(join(ROOT, path)));
const retiredTracked = files.filter((path) =>
  LEGACY_TRACKED_PATHS.some((pattern) => pattern.test(path))
);
const runtimeViolations = files
  .filter(isRuntimeSource)
  .flatMap((path) => scanSource(path, readFileSync(join(ROOT, path), 'utf8'), RUNTIME_MARKERS));
for (const path of files.filter((path) => /^crates\/backend\/[^/]+\/src\/.+\.rs$/.test(path))) {
  runtimeViolations.push(...scanSource(path, readFileSync(join(ROOT, path), 'utf8'), BACKEND_MARKERS));
}

const distViolations = [];
if (CHECK_DIST) {
  if (!existsSync(DIST)) {
    console.error('❌ ui/dist is missing; run `bun run build:ui` before the dist retirement gate.');
    process.exit(1);
  }
  for (const path of walkFiles(DIST)) {
    if (!/\.(?:css|html|js|json)$/.test(path)) continue;
    distViolations.push(
      ...scanSource(relative(ROOT, path).replaceAll('\\', '/'), readFileSync(path, 'utf8'), DIST_MARKERS)
    );
  }
}

if (retiredTracked.length > 0) {
  console.error('❌ Retired Creative Workshop files returned to the tracked tree:');
  retiredTracked.forEach((path) => console.error(`  ${path}`));
}
if (runtimeViolations.length > 0 || distViolations.length > 0) {
  console.error('❌ Retired Creative Workshop markers returned:');
  [...runtimeViolations, ...distViolations].forEach(({ path, line, label, token }) =>
    console.error(`  ${path}:${line} [${label}] ${token}`)
  );
}
if (retiredTracked.length > 0 || runtimeViolations.length > 0 || distViolations.length > 0) {
  process.exit(1);
}

const distSummary = CHECK_DIST ? ` and ${DIST_MARKERS.length} dist marker families` : '';
console.log(
  `✅ Creative Studio retirement clean (${files.length} tracked files, ${RUNTIME_MARKERS.length} runtime marker families${distSummary})`
);
