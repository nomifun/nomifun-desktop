/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError } from '@/common/types/ids';
import { miniAppImportReportFromError, miniapps, type IApiMiniApp } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const MINIAPP_ID = '0190f5fe-7c00-7a00-8000-0000000000b1';
const CONVERSATION_ID = '0190f5fe-7c00-7a00-8000-0000000000b2';
const realFetch = globalThis.fetch;

const rawMiniApp = (miniappId: unknown) => ({
  miniapp_id: miniappId,
  name: 'Pomodoro',
  description: 'A timer with reminders',
  icon: '⏱️',
  source_conversation_id: CONVERSATION_ID,
  html_size: 4_096,
  published_at: 1_754_800_000_123,
  has_unpublished_changes: false,
  created_at: 1,
  updated_at: 2,
});

function respondWith(data: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(JSON.stringify({ success: true, data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

/** Non-2xx answer with an arbitrary body — how a blocked import arrives. */
function respondWithStatus(status: number, body: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(JSON.stringify(body), {
        status,
        headers: { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

describe('mini-app wire contract', () => {
  test('every CRUD call addresses the routes the backend mounts', () => {
    expect(source.includes("httpGet<IApiMiniApp[], void>('/api/miniapps')")).toBe(true);
    expect(source.includes("httpPost<IApiMiniApp, IApiCreateMiniApp>('/api/miniapps')")).toBe(true);
    // Detail / update / delete share the `{miniapp_id}` capture on the same prefix.
    expect(source.split('`/api/miniapps/${p.miniapp_id}`').length - 1).toBe(3);
    expect(source.includes('delete: httpDelete<boolean, { miniapp_id: MiniAppId }>')).toBe(true);
  });

  test('the per-app verbs hang off the same capture, and there are only two', () => {
    // Five per-app routes exist on the backend — GET/PUT/DELETE on the bare id,
    // plus `/publish` and `/workspace` — and `/serve` is a sixth the bridge never
    // spells because it is unauthenticated and composed as an iframe `src` by
    // `pages/miniApps/contract.ts`. So exactly TWO suffixed paths live here.
    expect(source.split('`/api/miniapps/${p.miniapp_id}/').length - 1).toBe(2);
    expect(source.split('`/api/miniapps/${p.miniapp_id}/publish`').length - 1).toBe(1);
    expect(source.split('`/api/miniapps/${p.miniapp_id}/workspace`').length - 1).toBe(1);
    // Both are POSTs that send no body — the client never names a path, the
    // server derives everything from the id.
    expect(source.includes('publish: withResponseMap(')).toBe(true);
    expect(source.includes('provisionWorkspace: httpPost<IApiMiniAppWorkspace, { miniapp_id: MiniAppId }>')).toBe(true);
    // The retired per-app conversation pair is gone from the wire entirely: a
    // mini-app no longer owns a conversation, so nothing provisions or reads one.
    // Patterns rather than literals: the repo-wide zero-leftover grep scans this
    // file's own lines too.
    expect(/iteration[-]session/.test(source)).toBe(false);
    expect(/Iteration[S]ession/.test(source)).toBe(false);
  });

  test('the workspace route answers one absolute source path and nothing else', () => {
    expect(source.includes('export interface IApiMiniAppWorkspace')).toBe(true);
    expect(source.includes('source_path: string')).toBe(true);
    // No conversation id and no directory ride this response: the path IS the
    // whole contract, and 「继续迭代」 writes it into an ordinary conversation.
    expect(/export interface IApiMiniAppWorkspace \{\s*source_path: string;\s*\}/.test(source)).toBe(true);
  });

  test('the workspace path survives the wire and needs no branding', async () => {
    try {
      const sourcePath = `/home/u/.local/share/NomiFun/miniapps/${MINIAPP_ID}/miniapp.html`;
      respondWith({ source_path: sourcePath });
      const provisioned = await miniapps.provisionWorkspace.invoke({
        miniapp_id: MINIAPP_ID as IApiMiniApp['miniapp_id'],
      });
      expect(provisioned.source_path).toBe(sourcePath);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('the wire shape is snake_case and never carries the HTML body', () => {
    expect(source.includes('export interface IApiMiniApp')).toBe(true);
    for (const key of [
      'miniapp_id: MiniAppId',
      'name: string',
      'description: string',
      'icon: string | null',
      'source_conversation_id: string | null',
      'html_size: number',
      'published_at: number | null',
      'has_unpublished_changes: boolean',
      'created_at: number',
      'updated_at: number',
    ]) {
      expect(source.includes(key)).toBe(true);
    }
    // The record carries no conversation reference the client may navigate to:
    // `source_conversation_id` is provenance and stays an unbranded string.
    expect(/iteration_conversation/.test(source)).toBe(false);
    // The document itself only ever travels on create/update requests and back
    // out through the unauthenticated serve route.
    expect(source.includes('export interface IApiCreateMiniApp')).toBe(true);
    expect(source.includes('html: string')).toBe(true);
    expect(source.includes('html?: string')).toBe(true);
  });

  test('one mapper brands miniapp_id for every arrival path', () => {
    expect(source.includes('const fromApiMiniApp')).toBe(true);
    expect(source.includes('miniapp_id: parseMiniAppId(value.miniapp_id)')).toBe(true);
    // list + get + create + update + publish all route through the single mapper.
    expect(source.split('fromApiMiniApp').length - 1).toBeGreaterThanOrEqual(6);
  });

  test('list rows are branded at the boundary and a prefixed id is rejected', async () => {
    try {
      respondWith([rawMiniApp(MINIAPP_ID)]);
      const rows: IApiMiniApp[] = await miniapps.list.invoke();
      expect(rows[0]?.miniapp_id).toBe(MINIAPP_ID);
      expect(rows[0]?.html_size).toBe(4_096);
      expect(rows[0]?.source_conversation_id).toBe(CONVERSATION_ID);
      expect(rows[0]?.published_at).toBe(1_754_800_000_123);
      expect(rows[0]?.has_unpublished_changes).toBe(false);

      respondWith([rawMiniApp(`miniapp_${MINIAPP_ID}`)]);
      let error: unknown;
      try {
        await miniapps.list.invoke();
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof InvalidEntityIdError).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('a missing mini-app survives the detail mapper as null', async () => {
    try {
      respondWith(null);
      expect(await miniapps.get.invoke({ miniapp_id: MINIAPP_ID as IApiMiniApp['miniapp_id'] })).toBe(null);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('the two import routes are collection-level and take no id', () => {
    // Both hang off the collection prefix, NOT the `{miniapp_id}` capture — the
    // backend registers them before it so `validate` and `import` are never read
    // as ids. Counted so a copy-paste cannot quietly point one at the other.
    expect(source.split("'/api/miniapps/validate'").length - 1).toBe(1);
    expect(source.split("'/api/miniapps/import'").length - 1).toBe(1);
    expect(
      source.includes("httpPost<IApiMiniAppImportResponse, IApiMiniAppImportRequest>('/api/miniapps/validate')")
    ).toBe(true);
    expect(
      source.includes("httpPost<IApiMiniAppImportResponse, IApiMiniAppImportRequest>('/api/miniapps/import')")
    ).toBe(true);
    expect(source.includes('validateImport: withResponseMap(')).toBe(true);
    expect(source.includes('importApp: withResponseMap(')).toBe(true);
    // Adding them must not have grown the per-app capture counts.
    expect(source.split('`/api/miniapps/${p.miniapp_id}/').length - 1).toBe(2);
    expect(source.split('`/api/miniapps/${p.miniapp_id}`').length - 1).toBe(3);
  });

  test('the import request supplies exactly one of html / path, both optional on the wire', () => {
    expect(source.includes('export interface IApiMiniAppImportRequest')).toBe(true);
    for (const key of ['html?: string', 'path?: string', 'name?: string']) {
      expect(source.includes(key)).toBe(true);
    }
    // The report is rule ids plus structured detail — never prose from the server.
    expect(source.includes("export type IApiMiniAppImportSeverity = 'fatal' | 'autofix' | 'warning'")).toBe(true);
    expect(source.includes('rule_id: string')).toBe(true);
    expect(source.includes('severity: IApiMiniAppImportSeverity')).toBe(true);
    expect(source.includes('detail?: string')).toBe(true);
    expect(source.includes('blocked: boolean')).toBe(true);
    expect(source.includes('applied_fixes: string[]')).toBe(true);
  });

  test('validate answers 200 even for a blocked candidate, and brands nothing it has no app for', async () => {
    try {
      respondWith({
        report: {
          findings: [
            { rule_id: 'local_ref_unsupported', severity: 'fatal', detail: './style.css' },
            { rule_id: 'web_storage_use', severity: 'warning' },
          ],
          blocked: true,
        },
        applied_fixes: [],
      });
      const verdict = await miniapps.validateImport.invoke({ path: '/home/u/app/index.html' });
      expect(verdict.report.blocked).toBe(true);
      expect(verdict.report.findings.length).toBe(2);
      expect(verdict.report.findings[0]?.detail).toBe('./style.css');
      // Nothing was adopted, so there is no record to brand.
      expect(verdict.app).toBe(undefined);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('a successful import brands the adopted app through the shared mapper', async () => {
    try {
      respondWith({
        report: { findings: [{ rule_id: 'fragment_not_document', severity: 'autofix' }], blocked: false },
        applied_fixes: ['fragment_not_document'],
        app: rawMiniApp(MINIAPP_ID),
      });
      const adopted = await miniapps.importApp.invoke({ html: '<div>hi</div>' });
      expect(adopted.app?.miniapp_id).toBe(MINIAPP_ID);
      expect(adopted.app?.published_at).toBe(1_754_800_000_123);
      expect(adopted.applied_fixes).toEqual(['fragment_not_document']);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('the 400 that refuses a blocked import still yields its report', async () => {
    // The route answers `(400, ApiResponse::ok(outcome))`, so the SUCCESS envelope
    // rides a client-error status. `httpRequest` throws, and the report has to be
    // recoverable from the thrown error's body or the UI would have to ask twice.
    try {
      respondWithStatus(400, {
        success: true,
        data: {
          report: { findings: [{ rule_id: 'esm_bare_specifier', severity: 'fatal', detail: 'react' }], blocked: true },
          applied_fixes: [],
        },
      });
      let caught: unknown;
      try {
        await miniapps.importApp.invoke({ path: '/home/u/app.html' });
      } catch (e) {
        caught = e;
      }
      const recovered = miniAppImportReportFromError(caught);
      expect(recovered?.report.blocked).toBe(true);
      expect(recovered?.report.findings[0]?.rule_id).toBe('esm_bare_specifier');
      expect(recovered?.report.findings[0]?.detail).toBe('react');
      expect(recovered?.app).toBe(undefined);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('an ordinary rejection is NOT dressed up as a report', async () => {
    // A directory with no entry document, a path that does not exist, both inputs
    // at once: all plain `{ success: false, error }` bodies. Reading one as a
    // report would render an empty finding list as "nothing wrong".
    try {
      respondWithStatus(400, { success: false, error: 'no_root_document', code: 'BAD_REQUEST' });
      let caught: unknown;
      try {
        await miniapps.importApp.invoke({ path: '/home/u/app' });
      } catch (e) {
        caught = e;
      }
      expect(miniAppImportReportFromError(caught)).toBe(null);

      respondWithStatus(500, { success: false, error: 'internal', code: 'INTERNAL' });
      let serverError: unknown;
      try {
        await miniapps.validateImport.invoke({ html: '<div>hi</div>' });
      } catch (e) {
        serverError = e;
      }
      expect(miniAppImportReportFromError(serverError)).toBe(null);
    } finally {
      globalThis.fetch = realFetch;
    }
    // A non-error value is not a report either.
    expect(miniAppImportReportFromError(new Error('boom'))).toBe(null);
    expect(miniAppImportReportFromError(undefined)).toBe(null);
  });
});
