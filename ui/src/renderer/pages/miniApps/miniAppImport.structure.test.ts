/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Structure + behaviour gates for 「导入小程序」 (spec D14).
 *
 * Four invariants are pinned here because each one silently degrades rather than
 * crashing when it breaks:
 *  1. validate runs BEFORE import (an import-first flow turns every rejection
 *     into a 400 the UI has to reverse-engineer);
 *  2. the report is rendered per severity, so a fatal finding can never be shown
 *     with the same weight as a warning;
 *  3. all thirteen rule ids the validator can emit have copy in BOTH locales and
 *     a `t(` lookup — a missing sentence is a blank line, not an error;
 *  4. a blocked report offers 「用会话改造」, which is the only way forward when the
 *     app needs rewriting.
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import {
  MINI_APP_IMPORT_RULE_IDS,
  MINI_APP_IMPORT_SEVERITY_ORDER,
  formatMiniAppImportBytes,
  formatMiniAppImportDetail,
  groupMiniAppImportFindings,
  isMiniAppImportRuleId,
  resolveMiniAppImportRuleKeys,
} from './importReport';
import {
  MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT,
  MINI_APP_IMPORT_INLINE_SOURCE_LIMIT,
  buildMiniAppImportConversionPrompt,
  miniAppImportSourceBaseName,
} from './importConversion';
import { MINI_APP_FILE_NAME } from './contract';

const dialogSource = readFileSync(new URL('./MiniAppImportDialog.tsx', import.meta.url), 'utf8');
const reportSource = readFileSync(new URL('./importReport.ts', import.meta.url), 'utf8');
const conversionSource = readFileSync(new URL('./importConversion.ts', import.meta.url), 'utf8');
const listSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const zh = JSON.parse(
  readFileSync(new URL('../../services/i18n/locales/zh-CN/miniApps.json', import.meta.url), 'utf8')
) as Record<string, any>;
const en = JSON.parse(
  readFileSync(new URL('../../services/i18n/locales/en-US/miniApps.json', import.meta.url), 'utf8')
) as Record<string, any>;

/**
 * The catalogue the backend can emit, mirrored from `IMPORT_RULE_IDS` in
 * crates/backend/nomifun-miniapp/src/validation.rs. Spelled out rather than
 * imported so that dropping an id from the frontend list fails here instead of
 * quietly shrinking the expectation.
 */
const BACKEND_RULE_IDS = [
  'empty_payload',
  'size_over_limit',
  'not_html',
  'no_root_document',
  'fragment_not_document',
  'local_ref_unsupported',
  'dev_server_ref',
  'framework_source_entry',
  'server_template_markers',
  'esm_bare_specifier',
  'external_cdn_ref',
  'web_storage_use',
  'nested_iframe_embed',
] as const;

/** Severities exactly as the backend emits them today. */
const BACKEND_SEVERITY: Record<(typeof BACKEND_RULE_IDS)[number], 'fatal' | 'autofix' | 'warning'> = {
  empty_payload: 'fatal',
  size_over_limit: 'fatal',
  not_html: 'fatal',
  no_root_document: 'fatal',
  fragment_not_document: 'autofix',
  local_ref_unsupported: 'fatal',
  dev_server_ref: 'fatal',
  framework_source_entry: 'fatal',
  server_template_markers: 'fatal',
  esm_bare_specifier: 'fatal',
  external_cdn_ref: 'warning',
  web_storage_use: 'warning',
  nested_iframe_embed: 'warning',
};

describe('mini-app import: validate before import', () => {
  test('the dialog validates on pick and imports only from a clean report', () => {
    expect(dialogSource.includes('ipcBridge.miniapps.validateImport.invoke(')).toBe(true);
    expect(dialogSource.includes('ipcBridge.miniapps.importApp.invoke(')).toBe(true);
    // Every source pick funnels into the same validate step; neither picker can
    // reach the import call.
    expect(/await runValidate\(\{ kind: 'path'/.test(dialogSource)).toBe(true);
    expect(/await runValidate\(\{ kind: 'html'/.test(dialogSource)).toBe(true);
    expect(/pickPath[\s\S]{0,600}importApp\.invoke/.test(dialogSource)).toBe(false);
    // The primary button cannot fire before a report exists, and never on a
    // blocked one.
    expect(
      dialogSource.includes('disabled={source === null || report === null || blocked || validating || converting}')
    ).toBe(true);
    expect(dialogSource.includes('const blocked = report?.blocked === true;')).toBe(true);
  });

  test('a 400 on import is read as a report, not as a transport failure', () => {
    // The route answers a blocked candidate with 400 carrying the success
    // envelope. Recovering it keeps a late rejection (the source changed after
    // validation) from surfacing as a bare error string.
    expect(dialogSource.includes('miniAppImportReportFromError(e)')).toBe(true);
    expect(dialogSource.includes("t('miniApps.import.errors.blockedOnImport')")).toBe(true);
  });

  test('the two intakes are chosen by probing the runtime, not by assuming one', () => {
    // `dialog.showOpen` returns absolute PATHS and only exists in the desktop
    // shell — its web fallback goes to `bridge.invoke`, whose promise never
    // settles without a responder, so calling it in a browser would hang.
    expect(dialogSource.includes('isTauriRuntime()')).toBe(true);
    expect(dialogSource.includes('ipcBridge.dialog.showOpen.invoke(')).toBe(true);
    expect(dialogSource.includes("properties: ['openDirectory']")).toBe(true);
    expect(dialogSource.includes("properties: ['openFile']")).toBe(true);
    // The browser intake sends BYTES; the desktop intake sends the path.
    expect(dialogSource.includes("type='file'")).toBe(true);
    expect(dialogSource.includes('await file.text()')).toBe(true);
    expect(dialogSource.includes("picked.kind === 'path' ? { path: picked.path } : { html: picked.html }")).toBe(true);
    // Folder import is path-only, so the browser flow says so instead of
    // offering a picker that cannot work.
    expect(dialogSource.includes("t('miniApps.import.webUiOnlyFile')")).toBe(true);
  });

  test('the library offers import beside both create buttons, and owns nothing else', () => {
    expect(listSource.includes('<MiniAppImportDialog')).toBe(true);
    // Header CTA + empty-state CTA.
    expect(listSource.split("t('miniApps.import.entry')").length - 1).toBe(2);
    // The flow itself belongs to the dialog: no validate/import call on the page.
    expect(listSource.includes('validateImport')).toBe(false);
    expect(listSource.includes('importApp')).toBe(false);
  });
});

describe('mini-app import: the report renders per severity', () => {
  test('severity order is fatal → autofix → warning', () => {
    expect([...MINI_APP_IMPORT_SEVERITY_ORDER]).toEqual(['fatal', 'autofix', 'warning']);
  });

  test('each severity has its own header, hint and accent in the dialog', () => {
    for (const severity of MINI_APP_IMPORT_SEVERITY_ORDER) {
      expect(dialogSource.includes(`miniApps.import.severity.${severity}`)).toBe(true);
      expect(dialogSource.includes(`miniApps.import.severity.${severity}Hint`)).toBe(true);
    }
    expect(dialogSource.includes('groupMiniAppImportFindings(')).toBe(true);
    expect(dialogSource.includes('t(style.headerKey, { total: group.findings.length })')).toBe(true);
    expect(dialogSource.includes('t(style.hintKey)')).toBe(true);
    // fatal reads as a refusal, autofix as reassurance, warning as a note.
    expect(dialogSource.includes('text-danger-6')).toBe(true);
    expect(dialogSource.includes('text-primary-6')).toBe(true);
    expect(dialogSource.includes('text-warning-6')).toBe(true);
  });

  test('grouping keeps backend order inside a bucket and drops empty buckets', () => {
    const groups = groupMiniAppImportFindings([
      { rule_id: 'web_storage_use', severity: 'warning' },
      { rule_id: 'local_ref_unsupported', severity: 'fatal', detail: './a.css' },
      { rule_id: 'dev_server_ref', severity: 'fatal', detail: 'http://localhost:5173/x.js' },
      { rule_id: 'nested_iframe_embed', severity: 'warning' },
    ]);
    expect(groups.map((group) => group.severity)).toEqual(['fatal', 'warning']);
    expect(groups[0]?.findings.map((f) => f.rule_id)).toEqual(['local_ref_unsupported', 'dev_server_ref']);
    expect(groups[1]?.findings.map((f) => f.rule_id)).toEqual(['web_storage_use', 'nested_iframe_embed']);
    expect(groupMiniAppImportFindings([])).toEqual([]);
  });

  test('a severity this build does not know is still shown, in its own bucket', () => {
    const groups = groupMiniAppImportFindings([
      { rule_id: 'future_rule', severity: 'catastrophe' as 'fatal' },
      { rule_id: 'web_storage_use', severity: 'warning' },
    ]);
    expect(groups.map((group) => group.severity)).toEqual(['warning', 'catastrophe']);
  });

  test('detail is interpolated, byte counts read as sizes, long refs are clipped', () => {
    expect(formatMiniAppImportDetail({ rule_id: 'size_over_limit', severity: 'fatal', detail: '5242880' })).toBe(
      '5.0 MB'
    );
    expect(
      formatMiniAppImportDetail({ rule_id: 'local_ref_unsupported', severity: 'fatal', detail: './style.css' })
    ).toBe('./style.css');
    expect(formatMiniAppImportDetail({ rule_id: 'web_storage_use', severity: 'warning' })).toBe(null);
    const long = formatMiniAppImportDetail({
      rule_id: 'local_ref_unsupported',
      severity: 'fatal',
      detail: 'a'.repeat(400),
    });
    expect(long?.length).toBe(121);
    expect(formatMiniAppImportBytes(2048)).toBe('2 KB');
    expect(formatMiniAppImportBytes(700)).toBe('700 B');
  });
});

describe('mini-app import: every rule id ships copy', () => {
  test('the frontend catalogue mirrors the backend rule list exactly', () => {
    expect([...MINI_APP_IMPORT_RULE_IDS]).toEqual([...BACKEND_RULE_IDS]);
    expect(MINI_APP_IMPORT_RULE_IDS.length).toBe(13);
    for (const id of BACKEND_RULE_IDS) expect(isMiniAppImportRuleId(id)).toBe(true);
    expect(isMiniAppImportRuleId('not_a_rule')).toBe(false);
  });

  test('each id maps to LITERAL i18n keys, so a missing one is a typecheck failure', () => {
    // `Record<MiniAppImportRuleId, I18nKey>` is what makes the compiler demand an
    // entry per id; template-literal keys would let a missing sentence through.
    expect(reportSource.includes('MINI_APP_IMPORT_RULE_TITLE_KEY: Record<MiniAppImportRuleId, I18nKey>')).toBe(true);
    expect(reportSource.includes('MINI_APP_IMPORT_RULE_FIX_KEY: Record<MiniAppImportRuleId, I18nKey>')).toBe(true);
    for (const id of BACKEND_RULE_IDS) {
      expect(reportSource.includes(`${id}: 'miniApps.import.rules.${id}.title'`)).toBe(true);
      expect(reportSource.includes(`${id}: 'miniApps.import.rules.${id}.fix'`)).toBe(true);
      const keys = resolveMiniAppImportRuleKeys(id);
      expect(keys?.title).toBe(`miniApps.import.rules.${id}.title`);
      expect(keys?.fix).toBe(`miniApps.import.rules.${id}.fix`);
    }
    // An id from a newer backend degrades to an explicit "unknown" sentence.
    expect(resolveMiniAppImportRuleKeys('future_rule')).toBe(null);
    expect(dialogSource.includes("t('miniApps.import.rules.unknown.title', { ruleId: finding.rule_id })")).toBe(true);
  });

  test('the dialog looks up that copy per finding, with detail interpolated', () => {
    expect(dialogSource.includes('resolveMiniAppImportRuleKeys(finding.rule_id)')).toBe(true);
    expect(dialogSource.includes('t(keys.title)')).toBe(true);
    expect(dialogSource.includes("t(keys.fix, { detail: detail ?? t('miniApps.import.detailUnknown') })")).toBe(true);
  });

  test('both locales carry a title and a remediation sentence for all 13', () => {
    for (const locale of [zh, en]) {
      for (const id of BACKEND_RULE_IDS) {
        const entry = locale.import?.rules?.[id];
        expect(typeof entry?.title).toBe('string');
        expect(entry.title.trim().length).toBeGreaterThan(0);
        expect(typeof entry?.fix).toBe('string');
        expect(entry.fix.trim().length).toBeGreaterThan(0);
      }
      // The five rules that carry a `detail` must interpolate it, and only those
      // (an unused `{{detail}}` renders as raw braces when nothing is passed —
      // the dialog always passes a value, so this is about honest phrasing).
      for (const id of [
        'size_over_limit',
        'local_ref_unsupported',
        'dev_server_ref',
        'server_template_markers',
        'esm_bare_specifier',
      ]) {
        expect(locale.import.rules[id].fix.includes('{{detail}}')).toBe(true);
      }
      expect(typeof locale.import?.detailUnknown).toBe('string');
    }
  });

  test('the autofix bucket holds the ONE repair that exists', () => {
    // `fragment_not_document` is the only rule the backend can repair; promising
    // more would make "handled on import" a lie.
    const autofix = BACKEND_RULE_IDS.filter((id) => BACKEND_SEVERITY[id] === 'autofix');
    expect(autofix).toEqual(['fragment_not_document']);
    expect(zh.import.rules.fragment_not_document.fix.includes('自动')).toBe(true);
    expect(en.import.rules.fragment_not_document.fix.includes('NomiFun')).toBe(true);
  });

  test('the copy states the two semantics the rules would otherwise be read wrong', () => {
    // `local_ref_unsupported` is NOT "put the file next to it": /serve returns one
    // document, so a sibling that travelled with the import is still unserveable.
    expect(zh.import.rules.local_ref_unsupported.fix.includes('只对外提供一个文档')).toBe(true);
    expect(en.import.rules.local_ref_unsupported.fix.includes('exactly one document')).toBe(true);
    // `web_storage_use` is a warning because the sandbox gives an opaque origin.
    expect(zh.import.rules.web_storage_use.fix.includes('opaque origin')).toBe(true);
    expect(en.import.rules.web_storage_use.fix.includes('opaque origin')).toBe(true);
  });
});

describe('mini-app import: the conversion fallback', () => {
  test('it is offered only on a blocked report, and it reuses the shared launcher', () => {
    expect(/\{blocked && \([\s\S]{0,400}t\('miniApps\.import\.convert'\)/.test(dialogSource)).toBe(true);
    expect(dialogSource.includes("t('miniApps.import.convertHint')")).toBe(true);
    expect(dialogSource.includes('useNomiQuickStart()')).toBe(true);
    // The create → seed → navigate sequence is NOT re-implemented here, and the
    // start page's own launcher is left alone.
    expect(dialogSource.includes('ipcBridge.conversation.create')).toBe(false);
    expect(dialogSource.includes('sessionStorage')).toBe(false);
    expect(dialogSource.includes('initial-message')).toBe(false);
    expect(dialogSource.includes('useMiniAppQuickStart')).toBe(false);
    // The thread is marked as a mini-app build so its `miniapp.html` opens in the
    // preview with 「发布为小程序」 — that is how the rewrite gets into the library.
    expect(dialogSource.includes('[MINI_APP_EXTRA_FLAG]: true')).toBe(true);
    expect(dialogSource.includes('system_prompt: MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT')).toBe(true);
  });

  test('the instruction names the single-file contract and the sandbox', () => {
    expect(MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT.includes(MINI_APP_FILE_NAME)).toBe(true);
    expect(MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT.includes('单文件')).toBe(true);
    expect(MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT.includes('allow-same-origin')).toBe(true);
    // Rewrite, not redesign: the user keeps their app.
    expect(MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT.includes('不是另做一个应用')).toBe(true);
  });

  test('the first message hands the model the exact rule ids that failed', () => {
    const prompt = buildMiniAppImportConversionPrompt({
      source: { kind: 'path', path: '/home/u/app/index.html' },
      findings: [
        { rule_id: 'local_ref_unsupported', severity: 'fatal', detail: './style.css' },
        { rule_id: 'esm_bare_specifier', severity: 'fatal', detail: 'react' },
        { rule_id: 'web_storage_use', severity: 'warning' },
      ],
    });
    expect(prompt.includes('local_ref_unsupported')).toBe(true);
    expect(prompt.includes('./style.css')).toBe(true);
    expect(prompt.includes('esm_bare_specifier')).toBe(true);
    expect(prompt.includes('react')).toBe(true);
    // Warnings ride along too: storage still has to be made safe.
    expect(prompt.includes('web_storage_use')).toBe(true);
    // The product is ONE self-contained document, in the workspace root.
    expect(prompt.includes(MINI_APP_FILE_NAME)).toBe(true);
    expect(prompt.includes('自包含')).toBe(true);
    // A path source is read by the model rather than pasted in.
    expect(prompt.includes('/home/u/app/index.html')).toBe(true);
    expect(prompt.includes('```html')).toBe(false);
  });

  test('a single-file path source also carries the document text, best effort', () => {
    // The path lets the model read a folder's siblings; the inlined text lets it
    // start even if its file tools cannot reach outside the session workspace.
    const prompt = buildMiniAppImportConversionPrompt({
      source: { kind: 'path', path: '/home/u/app.html', document: '<div>hi</div>' },
      findings: [{ rule_id: 'local_ref_unsupported', severity: 'fatal', detail: './a.css' }],
    });
    expect(prompt.includes('/home/u/app.html')).toBe(true);
    expect(prompt.includes('```html')).toBe(true);
    expect(prompt.includes('<div>hi</div>')).toBe(true);
    // The pre-read is optional and never fatal: a folder pick skips it, and a
    // failed read falls back to naming the path.
    expect(dialogSource.includes('ipcBridge.fs.readFile.invoke({ path: source.path })')).toBe(true);
    expect(dialogSource.includes("if (source.kind === 'path' && !source.folder)")).toBe(true);
    expect(/could not pre-read the import source[\s\S]{0,80}\}/.test(dialogSource)).toBe(true);
  });

  test('a byte-flow source is inlined, and a clip says so instead of lying', () => {
    const short = buildMiniAppImportConversionPrompt({
      source: { kind: 'html', fileName: 'app.html', html: '<div>hi</div>' },
      findings: [{ rule_id: 'fragment_not_document', severity: 'autofix' }],
    });
    expect(short.includes('```html')).toBe(true);
    expect(short.includes('<div>hi</div>')).toBe(true);
    expect(short.includes('源码过长')).toBe(false);

    const clipped = buildMiniAppImportConversionPrompt({
      source: { kind: 'html', fileName: 'app.html', html: 'x'.repeat(MINI_APP_IMPORT_INLINE_SOURCE_LIMIT + 10) },
      findings: [],
    });
    expect(clipped.includes('源码过长')).toBe(true);
    // No findings is still a legible instruction, never an empty list.
    expect(clipped.includes('没有给出具体失败项')).toBe(true);
  });

  test('the conversation name quotes the source file, both separators', () => {
    expect(miniAppImportSourceBaseName({ kind: 'path', path: '/home/u/app/index.html' })).toBe('index.html');
    expect(miniAppImportSourceBaseName({ kind: 'path', path: 'C:\\work\\app' })).toBe('app');
    expect(miniAppImportSourceBaseName({ kind: 'path', path: '/home/u/app/' })).toBe('app');
    expect(miniAppImportSourceBaseName({ kind: 'html', fileName: 'app.html', html: '' })).toBe('app.html');
    expect(dialogSource.includes("t('miniApps.import.conversationName'")).toBe(true);
    expect(dialogSource.includes('MINI_APP_NAME_SNIPPET_LENGTH')).toBe(true);
  });
});

describe('mini-app import: retired vocabulary stays out', () => {
  test('none of the new modules reach for it', () => {
    for (const source of [dialogSource, reportSource, conversionSource]) {
      expect(/orchestr/i.test(source)).toBe(false);
      expect(/sub[-_ ]?agent/i.test(source)).toBe(false);
      expect(/\bfleet\b/i.test(source)).toBe(false);
      expect(/task[_-]?board/i.test(source)).toBe(false);
      expect(/shared[_-]tasks/i.test(source)).toBe(false);
    }
  });
});
