#!/usr/bin/env bun
/**
 * help — 脚本目录的单一真相源呈现层。
 *
 *   bun run help            按分组彩色打印脚本目录
 *   bun run help --check    校验 package.json 的 scripts 与 scripts.json 双向对齐，
 *                           并校验两个 README 的生成表锚点存在
 *                           （有脚本没说明 / 有说明没脚本 / group 未定义 / 缺锚点 → 退出 1）
 *   bun run help --readme   用 scripts.json 重新生成 README.md 与 README.zh-CN.md 的
 *                           脚本目录表（各自在一对 HTML 注释锚点之间幂等替换）
 *
 * 描述来自 scripts/scripts.json（唯一真相源）；实际 shell 命令只存在于
 * package.json（不在此重复，避免双写漂移）。新增脚本的契约：package.json 加键
 * + scripts.json 加一行说明，二者必须对齐（--check 守门），两个 README 表 --readme 再生。
 *
 * 纯 node:fs，无第三方依赖；非 TTY 或设置 NO_COLOR 时自动降级为无色。
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const PKG = join(ROOT, 'package.json');
const MANIFEST = join(ROOT, 'scripts', 'scripts.json');
const READMES = [join(ROOT, 'README.md'), join(ROOT, 'README.zh-CN.md')];

const BEGIN = '<!-- BEGIN GENERATED SCRIPTS (bun run help --readme) -->';
const END = '<!-- END GENERATED SCRIPTS -->';

const useColor = process.stdout.isTTY && !process.env.NO_COLOR;
const paint = (code, s) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s);
const bold = (s) => paint('1', s);
const cyan = (s) => paint('36', s);
const dim = (s) => paint('2', s);
const red = (s) => paint('31', s);
const green = (s) => paint('32', s);

const pkg = JSON.parse(readFileSync(PKG, 'utf8'));
const manifest = JSON.parse(readFileSync(MANIFEST, 'utf8'));
const scripts = pkg.scripts ?? {};
const groups = manifest.groups ?? [];
const entries = manifest.scripts ?? {};

/** 返回对齐问题列表（空 = 对齐）。 */
function alignmentProblems() {
  const inPkg = Object.keys(scripts);
  const inManifest = Object.keys(entries);
  const manifestSet = new Set(inManifest);
  const pkgSet = new Set(inPkg);
  const groupIds = new Set(groups.map((g) => g.id));

  const missingDesc = inPkg.filter((k) => !manifestSet.has(k));
  const orphanDesc = inManifest.filter((k) => !pkgSet.has(k));
  const badGroup = inManifest.filter((k) => !groupIds.has(entries[k].group));

  const problems = [];
  if (missingDesc.length)
    problems.push(`package.json 脚本缺 scripts.json 说明: ${missingDesc.join(', ')}`);
  if (orphanDesc.length)
    problems.push(`scripts.json 有说明但 package.json 无脚本: ${orphanDesc.join(', ')}`);
  if (badGroup.length)
    problems.push(`scripts.json 脚本 group 未在 groups 定义: ${badGroup.join(', ')}`);
  return problems;
}

/** 按 groups 顺序分组；组内按 scripts.json 键序。 */
function groupedRows() {
  const rows = [];
  for (const g of groups) {
    const keys = Object.keys(entries).filter((k) => entries[k].group === g.id);
    if (keys.length) rows.push({ group: g, keys });
  }
  return rows;
}

function printList() {
  const width = Object.keys(entries).reduce((m, k) => Math.max(m, k.length), 0);
  console.log('\n' + bold('NomiFun 脚本目录') + dim('   bun run <script>') + '\n');
  for (const { group, keys } of groupedRows()) {
    console.log(bold(group.title));
    for (const k of keys) {
      console.log('  ' + cyan(k.padEnd(width)) + '  ' + dim('— ' + entries[k].desc));
    }
    console.log('');
  }
  console.log(
    dim(`共 ${Object.keys(entries).length} 个脚本。bun run help --check 校验登记完整性。`) + '\n'
  );
}

/** 返回 README 锚点问题列表（空 = 两个 README 的生成表锚点均就绪）。 */
function anchorProblems() {
  const problems = [];
  for (const file of READMES) {
    const name = file.slice(ROOT.length + 1);
    let md;
    try {
      md = readFileSync(file, 'utf8');
    } catch {
      problems.push(`${name} 不存在或不可读`);
      continue;
    }
    const b = md.indexOf(BEGIN);
    const e = md.indexOf(END);
    if (b === -1 || e === -1) {
      problems.push(`${name} 缺少生成表锚点：\n      ${BEGIN}\n      ${END}`);
    } else if (e < b) {
      problems.push(`${name} 锚点顺序颠倒（END 在 BEGIN 之前）`);
    }
  }
  return problems;
}

function readmeTable() {
  const lines = ['| 脚本 | 说明 |', '| --- | --- |'];
  for (const { group, keys } of groupedRows()) {
    lines.push(`| **${group.title}** | |`);
    for (const k of keys) lines.push(`| \`bun run ${k}\` | ${entries[k].desc} |`);
  }
  return lines.join('\n');
}

function writeReadme() {
  const anchors = anchorProblems();
  if (anchors.length) {
    console.error(red('README 锚点问题：'));
    for (const p of anchors) console.error('  - ' + p);
    process.exit(1);
  }
  const table = readmeTable();
  for (const file of READMES) {
    const name = file.slice(ROOT.length + 1);
    const md = readFileSync(file, 'utf8');
    const b = md.indexOf(BEGIN);
    const e = md.indexOf(END);
    const next = md.slice(0, b + BEGIN.length) + '\n\n' + table + '\n\n' + md.slice(e);
    if (next === md) {
      console.log(green(`${name} 脚本目录表已是最新（无变化）。`));
      continue;
    }
    writeFileSync(file, next);
    console.log(green(`${name} 脚本目录表已更新。`));
  }
}

const arg = process.argv[2];

if (arg === '--check') {
  const problems = [...alignmentProblems(), ...anchorProblems()];
  if (problems.length) {
    console.error(red('✗ 脚本登记未对齐：'));
    for (const p of problems) console.error('  - ' + p);
    process.exit(1);
  }
  console.log(green('✓ package.json 与 scripts.json 对齐，README 锚点就绪。'));
} else if (arg === '--readme') {
  const problems = alignmentProblems();
  if (problems.length) {
    console.error(red('请先修复对齐再生成 README：'));
    for (const p of problems) console.error('  - ' + p);
    process.exit(1);
  }
  writeReadme();
} else {
  printList();
}
