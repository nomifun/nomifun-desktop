/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Source contracts for the right-rail mini-app quick panel.
 *
 * These pin the decisions that are cheap to regress and expensive to notice: the
 * entry sits directly below the terminal icon and exists exactly once, the panel
 * runs the ONE shared frame instead of a second iframe, and it stays read-only —
 * editing and deleting belong to the left "Mini-Apps" tab.
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

/**
 * Comments are documentation, not behaviour. The "does not use X" assertions
 * below must read the code only — the panel's own doc comment legitimately names
 * the mutations it refuses to import, and matching that prose would be a false
 * positive.
 */
const stripComments = (source: string): string =>
  source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^[ \t]*\/\/.*$/gm, '');

const read = (relative: string): string =>
  stripComments(readFileSync(new URL(relative, import.meta.url), 'utf8'));

const panel = read('./index.tsx');
const extraTabs = read('../../hooks/useWorkspaceExtraTabs.tsx');
const library = read('../../../miniApps/index.tsx');

describe('the rail entry sits below the terminal icon, exactly once', () => {
  test('the descriptor is declared in one place only', () => {
    const occurrences = extraTabs.match(/key: 'conversation-miniapps'/g) ?? [];
    expect(occurrences.length).toBe(1);
  });

  test('render order puts it after terminals and before knowledge', () => {
    // The rail renders files → changes → extraTabs in array order, so "below the
    // terminal icon" is purely this array position.
    const terminalAt = extraTabs.indexOf("key: 'conversation-terminals'");
    const miniAppAt = extraTabs.indexOf("key: 'conversation-miniapps'");
    const knowledgeAt = extraTabs.indexOf('...knowledgeTabs');
    expect(terminalAt).toBeGreaterThan(-1);
    expect(miniAppAt).toBeGreaterThan(terminalAt);
    expect(knowledgeAt).toBeGreaterThan(miniAppAt);
  });

  test('it reuses the mini-app glyph and its existing nav label', () => {
    // `title` is both the rail tooltip and the panel header text, so it has to be
    // a plain string rather than a node.
    expect(extraTabs.includes("title: t('miniApps.nav.entry')")).toBe(true);
    expect(extraTabs.includes('<ApplicationOne size={18} />')).toBe(true);
    expect(extraTabs.includes('content: <MiniAppPanel />')).toBe(true);
  });

  test('the entry is unconditional, so a stored selection is never overwritten', () => {
    // WorkspaceRailBody validates the persisted active tab and PERSISTS a `files`
    // fallback: a tab that appears asynchronously would silently lose the user's
    // choice. Only the existing whole-rail workspace gate may return [].
    const gates = extraTabs.match(/return \[\];/g) ?? [];
    expect(gates.length).toBe(1);
    expect(extraTabs.includes('if (!conversationId || !hasWorkspace) return [];')).toBe(true);
  });
});

describe('the panel runs the one shared mini-app frame', () => {
  test('no second iframe, sandbox or serve URL is spelled out here', () => {
    expect(panel.includes('<MiniAppFrame')).toBe(true);
    expect(
      /<MiniAppFrame\s+miniAppId=\{runningApp\.miniapp_id\}\s+name=\{runningApp\.name\}\s+reloadToken=\{reloadToken\}\s*\/>/.test(
        panel
      )
    ).toBe(true);
    expect(panel.includes('<iframe')).toBe(false);
    expect(panel.includes('MINI_APP_IFRAME_SANDBOX')).toBe(false);
    expect(panel.includes('resolveMiniAppServeUrl')).toBe(false);
    expect(panel.includes('srcDoc')).toBe(false);
  });

  test('reload goes through the frame token, not a ref handle', () => {
    expect(panel.includes('setReloadToken((token) => token + 1)')).toBe(true);
    expect(panel.includes('useRef')).toBe(false);
  });

  test('the frame is mounted in an already-sized box', () => {
    // MiniAppFrame is size-full; a parent without a resolved height collapses it
    // to zero, which reads as the blank render this feature just got fixed for.
    expect(panel.includes("<div className='relative flex-1 min-h-0 w-full bg-[var(--color-bg-1)]'>")).toBe(true);
  });
});

describe('the quick panel is search-and-use only', () => {
  test('it touches neither mutation bridge call nor the shared dialogs', () => {
    expect(panel.includes('useMiniAppMutations')).toBe(false);
    expect(panel.includes('ipcBridge.miniapps.update')).toBe(false);
    expect(panel.includes('ipcBridge.miniapps.delete')).toBe(false);
    expect(panel.includes("t('miniApps.actions.rename')")).toBe(false);
    expect(panel.includes("t('miniApps.actions.delete')")).toBe(false);
    expect(panel.includes('Modal.confirm(')).toBe(false);
  });

  test('reads the library through the list call it shares with the left tab', () => {
    expect(panel.includes('ipcBridge.miniapps.list.invoke()')).toBe(true);
    expect(library.includes('ipcBridge.miniapps.list.invoke()')).toBe(true);
  });

  test('refetches on mount, so re-showing the panel refreshes it', () => {
    // The rail unmounts an inactive tab's body, so the mount fetch doubles as
    // "refresh when shown"; the explicit control covers a same-tab solidify.
    expect(panel.includes('void refresh();')).toBe(true);
    expect(panel.includes("t('miniApps.actions.refresh')")).toBe(true);
  });
});

describe('the list matches the library it mirrors', () => {
  test('same name/description filter as /mini-apps', () => {
    const filter = 'app.name.toLowerCase().includes(q) || app.description.toLowerCase().includes(q)';
    expect(panel.includes(filter)).toBe(true);
    expect(library.includes(filter)).toBe(true);
  });

  test('the card carries a visible "use it" control without double-firing', () => {
    expect(panel.includes("t('miniApps.actions.open')")).toBe(true);
    expect(panel.includes('CARD_OPEN_BUTTON_CLASS')).toBe(true);
    expect(/stopPropagation\(\);\s*onOpen\(app\);/.test(panel)).toBe(true);
    expect(panel.includes('onClick={() => onOpen(app)}')).toBe(true);
  });

  test('one relative-time formatter serves both surfaces', () => {
    expect(panel.includes('formatMiniAppRelativeTime(app.updated_at, t)')).toBe(true);
    expect(library.includes('formatMiniAppRelativeTime(app.updated_at, t)')).toBe(true);
  });

  test('loading, error, empty and filtered-empty all have their own copy', () => {
    expect(panel.includes('<Spin />')).toBe(true);
    expect(panel.includes("t('miniApps.errors.loadListFailed')")).toBe(true);
    expect(panel.includes("t('miniApps.empty.title')")).toBe(true);
    expect(panel.includes("t('miniApps.list.filterEmpty')")).toBe(true);
    expect(panel.includes("t('miniApps.list.searchPlaceholder')")).toBe(true);
  });

  test('a failed fetch never renders as an empty library', () => {
    // `apps` starts empty, so a first-load failure would otherwise show the error
    // strip AND "no mini-apps yet" + "go create one" — telling a user with a full
    // library that it is empty. The empty state must be gated on knowing.
    expect(panel.includes('error ? null : (')).toBe(true);
    const guardAt = panel.indexOf('error ? null : (');
    const emptyCopyAt = panel.indexOf("t('miniApps.empty.title')");
    expect(guardAt).toBeGreaterThan(-1);
    expect(emptyCopyAt).toBeGreaterThan(guardAt);
  });
});

describe('the rail is a use-here loop with an escape hatch', () => {
  test('picking a card runs it in the rail instead of navigating away', () => {
    expect(panel.includes('setRunningId(app.miniapp_id)')).toBe(true);
  });

  test('a full-screen escape hatch reaches the runner route', () => {
    expect(panel.includes('navigate(`/mini-apps/${runningApp.miniapp_id}`)')).toBe(true);
    expect(panel.includes("t('miniApps.panel.openFullPage')")).toBe(true);
  });

  test('back returns to the list with the query preserved', () => {
    expect(panel.includes('const backToList = useCallback(() => setRunningId(null), []);')).toBe(true);
    expect(panel.includes("setSearchQuery('')")).toBe(false);
    expect(panel.includes("t('miniApps.panel.backToList')")).toBe(true);
  });

  test('the running app is derived from the live list, not stored', () => {
    // A mini-app deleted from the left tab must drop the rail back to the list
    // rather than keep a frame open for something that no longer exists.
    expect(panel.includes('apps.find((app) => app.miniapp_id === runningId)')).toBe(true);
    expect(panel.includes('useState<IApiMiniApp | null>')).toBe(false);
  });
});

describe('the panel avoids retired collaboration vocabulary', () => {
  test('no retired terms in the panel or the registration site', () => {
    for (const source of [panel, extraTabs]) {
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
