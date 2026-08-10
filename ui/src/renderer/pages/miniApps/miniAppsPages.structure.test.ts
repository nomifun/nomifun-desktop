/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const listSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const runnerSource = readFileSync(new URL('./RunnerPage.tsx', import.meta.url), 'utf8');
const mutationsSource = readFileSync(new URL('./useMiniAppMutations.tsx', import.meta.url), 'utf8');
const frameSource = readFileSync(new URL('./MiniAppFrame.tsx', import.meta.url), 'utf8');
const relativeTimeSource = readFileSync(new URL('./relativeTime.ts', import.meta.url), 'utf8');

describe('mini-app pages structure', () => {
  test('the library reads the list through the mini-app bridge section', () => {
    expect(listSource.includes('ipcBridge.miniapps.list.invoke()')).toBe(true);
    // The HTML body never rides the list response, so the library must not try
    // to render it locally.
    expect(listSource.includes('srcDoc')).toBe(false);
  });

  test('rename + delete live in one shared module used by both pages', () => {
    expect(mutationsSource.includes('ipcBridge.miniapps.update.invoke(')).toBe(true);
    expect(mutationsSource.includes('ipcBridge.miniapps.delete.invoke(')).toBe(true);
    expect(mutationsSource.includes("t('miniApps.rename.title')")).toBe(true);
    expect(mutationsSource.includes("t('miniApps.delete.confirmTitle')")).toBe(true);

    for (const source of [listSource, runnerSource]) {
      expect(source.includes('useMiniAppMutations')).toBe(true);
      // No duplicated dialog or mutation call left behind on either page.
      expect(source.includes('ipcBridge.miniapps.update.invoke(')).toBe(false);
      expect(source.includes('ipcBridge.miniapps.delete.invoke(')).toBe(false);
      expect(source.includes('Modal.confirm(')).toBe(false);
    }
  });

  test('the library opens a card by miniapp_id and hands creation to the start page', () => {
    expect(listSource.includes('navigate(`/mini-apps/${app.miniapp_id}`)')).toBe(true);
    expect(listSource.includes("'/guid?miniapp=1'")).toBe(true);
    expect(listSource.includes("t('miniApps.actions.create')")).toBe(true);
    expect(listSource.includes("t('miniApps.empty.cta')")).toBe(true);
    expect(listSource.includes("t('miniApps.list.searchPlaceholder')")).toBe(true);
    expect(listSource.includes("t('miniApps.list.updatedAt'")).toBe(true);
    // Its own filtered-empty copy and its own time keys — the workshop namespace
    // must stay free to evolve.
    expect(listSource.includes("t('miniApps.list.filterEmpty')")).toBe(true);
    // The relative-time phrasing is shared with the right-rail quick panel, so it
    // lives in one module — but still on THIS namespace's keys, so the workshop
    // gallery's wording stays free to evolve.
    expect(listSource.includes('formatMiniAppRelativeTime(')).toBe(true);
    expect(relativeTimeSource.includes("t('miniApps.time.justNow')")).toBe(true);
    expect(listSource.includes('workshop.time.')).toBe(false);
    expect(relativeTimeSource.includes('workshop.time.')).toBe(false);
  });

  test('a library card shows an explicit "use it" control, without double-firing', () => {
    // The library is the official place a mini-app is used, so "open" must be
    // labelled on the card rather than hidden behind guessing that the card body
    // is clickable.
    expect(listSource.includes("t('miniApps.actions.open')")).toBe(true);
    expect(listSource.includes('CARD_OPEN_BUTTON_CLASS')).toBe(true);
    // The card body already navigates; the inner control has to stop the bubble
    // or one click would open twice.
    expect(/stopPropagation\(\);\s*onOpen\(app\);/.test(listSource)).toBe(true);
    // Whole-card click and the hover mutations all survive.
    expect(listSource.includes('onClick={() => onOpen(app)}')).toBe(true);
    expect(listSource.includes("t('miniApps.actions.rename')")).toBe(true);
    expect(listSource.includes("t('miniApps.actions.delete')")).toBe(true);
  });

  test('the sandboxed serve-route frame lives in ONE shared component', () => {
    expect(frameSource.includes('resolveMiniAppServeUrl(miniAppId)')).toBe(true);
    expect(frameSource.includes('sandbox={MINI_APP_IFRAME_SANDBOX}')).toBe(true);
    // A titled iframe is the only accessible name the frame gets.
    expect(frameSource.includes('title={name}')).toBe(true);
    // The sandbox constant is imported, never re-spelled — re-adding
    // `allow-same-origin` next to `allow-scripts` would void the sandbox.
    expect(frameSource.includes('allow-same-origin')).toBe(false);
    // Reload is a remount keyed on the parent's token plus the private retry
    // counter; there is no ref handle to drift out of sync.
    expect(frameSource.includes('key={mountKey}')).toBe(true);
    expect(frameSource.includes('reloadToken}:${retryToken}')).toBe(true);
  });

  test('the frame watchdog fires once per mount and is always cleared', () => {
    expect(frameSource.includes('MINI_APP_LOAD_WATCHDOG_MS = 6000')).toBe(true);
    expect(frameSource.includes('window.setTimeout(')).toBe(true);
    expect(frameSource.includes('window.clearTimeout(watchdogRef.current)')).toBe(true);
    // Cleared on unmount/remount (effect teardown) and the moment a load lands.
    expect(frameSource.includes('return clearWatchdog;')).toBe(true);
    expect(frameSource.includes('onLoad={handleLoad}')).toBe(true);
    // A slim dismissible hint, not a full-screen error: a slow app stays usable.
    expect(frameSource.includes("t('miniApps.frame.stalledHint')")).toBe(true);
    expect(frameSource.includes("t('miniApps.actions.retry')")).toBe(true);
    expect(frameSource.includes("t('miniApps.actions.openInBrowser')")).toBe(true);
    expect(frameSource.includes('ipcBridge.shell.openExternal.invoke(resolveMiniAppServeUrl(miniAppId))')).toBe(true);
  });

  test('the runner loads a branded id and delegates the runtime to MiniAppFrame', () => {
    expect(runnerSource.includes('parseMiniAppId(rawId)')).toBe(true);
    expect(runnerSource.includes('ipcBridge.miniapps.get.invoke({ miniapp_id: miniAppId })')).toBe(true);
    // No second iframe: the runner and the quick panel must not be able to drift.
    expect(runnerSource.includes('<iframe')).toBe(false);
    expect(runnerSource.includes('MINI_APP_IFRAME_SANDBOX')).toBe(false);
    expect(
      /<MiniAppFrame\s+miniAppId=\{miniAppId\}\s+name=\{app\.name\}\s+reloadToken=\{reloadToken\}\s*\/>/.test(
        runnerSource
      )
    ).toBe(true);
  });

  test('the runner tells a deleted mini-app apart from a failed load', () => {
    // `httpBridge` throws on 404 — it never resolves to null — so the detail
    // call's catch has to recognise the status itself.
    expect(runnerSource.includes('isBackendHttpError(e) && e.status === 404')).toBe(true);
  });

  test('the runner remounts the frame to refresh and offers the full action set', () => {
    expect(runnerSource.includes('setReloadToken((token) => token + 1)')).toBe(true);
    expect(runnerSource.includes("navigate('/mini-apps')")).toBe(true);
    expect(runnerSource.includes("t('miniApps.runner.notFound')")).toBe(true);
  });

  test('the runner never navigates to a source conversation', () => {
    // A mini-app outlives the conversation that produced it, so a jump into that
    // conversation is a link that rots — and navigating to `/conversation/:id`
    // from outside the session shell errored outright. Iteration happens in the
    // mini-app's own session instead, which this page hosts.
    expect(runnerSource.includes('/conversation/')).toBe(false);
    expect(runnerSource.includes('openSource')).toBe(false);
    expect(runnerSource.includes('LinkOne')).toBe(false);
  });

  test('neither page reaches for retired collaboration vocabulary', () => {
    for (const source of [listSource, runnerSource, mutationsSource, frameSource]) {
      expect(/orchestr/i.test(source)).toBe(false);
      expect(/sub[-_ ]?agent/i.test(source)).toBe(false);
      expect(/\bfleet\b/i.test(source)).toBe(false);
      // Patterns rather than literals: the repo-wide vocabulary gate scans this
      // file's own lines too.
      expect(/task[_-]?board/i.test(source)).toBe(false);
      expect(/shared[_-]tasks/i.test(source)).toBe(false);
    }
  });
});
