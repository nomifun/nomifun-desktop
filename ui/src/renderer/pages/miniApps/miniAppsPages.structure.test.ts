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
const iterateSource = readFileSync(new URL('./useMiniAppIterate.ts', import.meta.url), 'utf8');
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

  test('a library card carries iterate beside rename and delete', () => {
    expect(listSource.includes('onIterate')).toBe(true);
    expect(listSource.includes("key: 'iterate'")).toBe(true);
    expect(listSource.includes('const { iterate } = useMiniAppIterate()')).toBe(true);
    // The card body navigates on click, so every hover action stops the bubble.
    expect(listSource.includes('onClick={(e) => e.stopPropagation()}')).toBe(true);
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
    expect(runnerSource.includes('<MiniAppFrame')).toBe(true);
    expect(runnerSource.includes('miniAppId={miniAppId}')).toBe(true);
    expect(runnerSource.includes('name={app.name}')).toBe(true);
    expect(runnerSource.includes('reloadToken={reloadToken}')).toBe(true);
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
    // Back / refresh / rename / delete / open-in-browser, plus the two labelled
    // verbs asserted below.
    expect(runnerSource.includes("t('miniApps.actions.back')")).toBe(true);
    expect(runnerSource.includes("t('miniApps.actions.refresh')")).toBe(true);
    expect(runnerSource.includes("t('miniApps.actions.rename')")).toBe(true);
    expect(runnerSource.includes("t('miniApps.actions.delete')")).toBe(true);
    expect(runnerSource.includes("t('miniApps.actions.openInBrowser')")).toBe(true);
    expect(runnerSource.includes('ipcBridge.shell.openExternal.invoke(resolveMiniAppServeUrl(miniAppId))')).toBe(true);
    // Refresh has to re-read the record too, or a user who iterated elsewhere
    // reloads an iframe that still serves the old snapshot and reports 改了不生效.
    expect(/const refresh = useCallback\(\(\) => \{[\s\S]{0,200}void load\(\)/.test(runnerSource)).toBe(true);
  });

  test('the runner is ONE column: no aside, no split, no chat (spec D18)', () => {
    // The body is a single flex child that may shrink and has a resolved height:
    // a percentage-height iframe under an auto-height ancestor collapses to 0px,
    // which looks exactly like the blank render this layout exists to avoid.
    expect(runnerSource.includes('relative flex-1 min-h-0 w-full overflow-hidden')).toBe(true);
    // Every trace of the deleted split and of the panel that lived in it.
    expect(runnerSource.includes('ContentAside')).toBe(false);
    // Pattern rather than the literal name, so the repo-wide zero-leftover grep
    // for the deleted panel does not hit this assertion.
    expect(/MiniApp[Ii]teration/.test(runnerSource)).toBe(false);
    expect(runnerSource.includes('closeOnEscape')).toBe(false);
    expect(runnerSource.includes('bodyClassName')).toBe(false);
    expect(runnerSource.includes('useResizableSplit')).toBe(false);
    expect(runnerSource.includes('storageKey')).toBe(false);
    expect(runnerSource.includes('MINI_APP_FRAME_MIN_WIDTH_PX')).toBe(false);
    // No chat, so no conversation module graph and no per-thread subscription.
    expect(runnerSource.includes('turnCompleted')).toBe(false);
    expect(runnerSource.includes('pages/conversation')).toBe(false);
    // And no reason left to know about the viewport.
    expect(runnerSource.includes('isMobile')).toBe(false);
    // The runner is reached by a plain card click; nothing arms a panel any more.
    expect(runnerSource.includes('useSearchParams')).toBe(false);
    expect(runnerSource.includes("'iterate'")).toBe(false);
  });

  test('「继续迭代」 lives on both surfaces and goes through ONE shared hook', () => {
    for (const source of [runnerSource, listSource]) {
      expect(source.includes("t('miniApps.iterate.toggle')")).toBe(true);
      expect(source.includes('useMiniAppIterate')).toBe(true);
      // Neither page re-implements the two steps, so they cannot tell the model
      // different things.
      expect(source.includes('provisionWorkspace')).toBe(false);
      expect(source.includes('buildMiniAppIterateMessage')).toBe(false);
    }
  });

  test('the iterate hook provisions the working copy, then launches an ORDINARY conversation', () => {
    // Order matters: the absolute path has to exist before it can be written into
    // the first message (spec D19).
    expect(iterateSource.includes('ipcBridge.miniapps.provisionWorkspace.invoke(')).toBe(true);
    expect(/provisionWorkspace[\s\S]{0,600}useNomiQuickStart|useNomiQuickStart[\s\S]{0,600}provisionWorkspace/.test(iterateSource)).toBe(true);
    expect(iterateSource.includes('workspace?.source_path?.trim()')).toBe(true);
    expect(iterateSource.includes('buildMiniAppIterateMessage(')).toBe(true);
    expect(iterateSource.includes('buildMiniAppIterateConversationName(')).toBe(true);
    // The shared launcher owns create → history refresh → first-turn handoff →
    // navigate to `/conversation/:id`, so this hook adds none of it.
    expect(iterateSource.includes('useNomiQuickStart')).toBe(true);
    expect(iterateSource.includes('ipcBridge.conversation.create')).toBe(false);
    expect(iterateSource.includes('sessionStorage')).toBe(false);
    expect(iterateSource.includes('navigate(')).toBe(false);
    // Ordinary in every sense: no marker, no mini-app id, no workspace override.
    expect(iterateSource.includes('miniapp_id:')).toBe(true); // the provision call's only argument
    expect(iterateSource.includes('MINI_APP_EXTRA_FLAG')).toBe(false);
    expect(iterateSource.includes('extra:')).toBe(false);
    expect(iterateSource.includes('workspace:')).toBe(false);
    // A synchronous guard, not just the reported flag: two clicks in one tick
    // would otherwise open two conversations for one app.
    expect(iterateSource.includes('startingRef.current')).toBe(true);
  });

  test('publishing is what changes the running app, and says so', () => {
    expect(runnerSource.includes('app.has_unpublished_changes')).toBe(true);
    expect(runnerSource.includes('ipcBridge.miniapps.publish.invoke({ miniapp_id: miniAppId })')).toBe(true);
    expect(runnerSource.includes("t('miniApps.publish.action')")).toBe(true);
    // One short sentence about published-vs-working, or users report 改了不生效.
    expect(runnerSource.includes("t('miniApps.publish.explain')")).toBe(true);
    // A publish changed the served document under a live iframe: adopt the record
    // AND remount the frame.
    expect(runnerSource.includes('setApp(published)')).toBe(true);
    expect(/setApp\(published\);[\s\S]{0,400}setReloadToken/.test(runnerSource)).toBe(true);
  });

  test('nothing in the library graph knows about draft rows any more', () => {
    // Spec D17: 「创建小程序」 creates a conversation, not a row, so there is no
    // placeholder document to store and no byte count to recognise it by.
    const contractSource = readFileSync(new URL('./contract.ts', import.meta.url), 'utf8');
    const quickStartSource = readFileSync(
      new URL('../../hooks/agent/useMiniAppQuickStart.ts', import.meta.url),
      'utf8'
    );
    for (const source of [contractSource, runnerSource, listSource, quickStartSource]) {
      expect(source.includes('MINI_APP_DRAFT_PLACEHOLDER_HTML')).toBe(false);
      expect(source.includes('MINI_APP_DRAFT_PLACEHOLDER_BYTES')).toBe(false);
      expect(source.includes('miniApps.draft.')).toBe(false);
    }
    expect(quickStartSource.includes('ipcBridge.miniapps.create')).toBe(false);
  });

  test('the runner hosts no conversation of its own', () => {
    // A mini-app outlives every conversation that touched it, so this page owns
    // no thread and links to none. 「继续迭代」 launches one through the shared hook,
    // which is also where the whole `pages/conversation/**` module graph stays.
    expect(runnerSource.includes('/conversation/')).toBe(false);
    expect(runnerSource.includes('openSource')).toBe(false);
    expect(runnerSource.includes('LinkOne')).toBe(false);
    expect(runnerSource.includes('conversation_id')).toBe(false);
  });

  test('neither page reaches for retired collaboration vocabulary', () => {
    for (const source of [listSource, runnerSource, mutationsSource, frameSource, iterateSource]) {
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
