/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Source contracts for the session knowledge rail panel.
 *
 * These guard the decisions that are cheap to regress and expensive to notice:
 * read-only-ness, per-base tree keys, the non-recursive expand-all, and the
 * fact that BOTH conversation registration sites go through one helper.
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

/**
 * Comments are documentation, not behaviour. The "does not use X" assertions
 * below must read the code only — the panel's own doc comment legitimately names
 * the mutations it refuses to import and the recursive expander it refuses to
 * reuse, and matching that prose would be a false positive.
 */
const stripComments = (source: string): string =>
  source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^[ \t]*\/\/.*$/gm, '');

const read = (relative: string): string =>
  stripComments(readFileSync(new URL(relative, import.meta.url), 'utf8'));

const panel = read('./index.tsx');
const mounts = read('./useSessionKnowledgeMounts.ts');
const tabFactory = read('./useSessionKnowledgeTab.tsx');
const extraTabs = read('../../hooks/useWorkspaceExtraTabs.tsx');
const chatConversation = read('../../components/ChatConversation.tsx');
const terminalPage = read('../../../terminal/TerminalSessionPage.tsx');
const terminalRail = read('../../../terminal/TerminalWorkspaceRail.tsx');

describe('the async resolve window cannot desync the rail from the body', () => {
  test('the last known mount state is seeded synchronously from storage', () => {
    // WorkspaceRailBody validates the persisted active tab against the tab list
    // and PERSISTS its fallback. If the knowledge tab is missing on the first
    // render, the user's stored `session-knowledge` selection is rewritten to
    // `files` while the rail keeps its own copy — icon active, file tree shown.
    expect(mounts.includes('function readSeed(')).toBe(true);
    expect(mounts.includes('function writeSeed(')).toBe(true);
    expect(mounts.includes('const mountedIds = binding ? liveIds : seedIds;')).toBe(true);
  });

  test('a refresh never blanks the cached list', () => {
    // Clearing basesCache would flip `mounted` false for a beat, unmounting the
    // open panel and losing its tree.
    expect(mounts.includes('basesCache = null')).toBe(false);
    expect(mounts.includes('const refreshBases = ')).toBe(true);
  });

  test('a failed read is re-armed instead of wedging the entry off', () => {
    expect(mounts.includes('setAttempt((n) => n + 1)')).toBe(true);
    expect(mounts.includes('attempt]')).toBe(true);
  });

  test('the in-flight guard is inside the effect, not computed during render', () => {
    // Computed during render, two consumers mounted in the same commit both
    // pass it and both fire the request.
    expect(mounts.includes('if (bindingCache.has(targetKey) || bindingInflight.has(targetKey)) return;')).toBe(
      true
    );
    expect(mounts.includes('if (basesCache !== null || basesInflight) return;')).toBe(true);
  });

  test('binding events only re-render the sessions that share that target', () => {
    expect(mounts.includes('const notifyTarget =')).toBe(true);
    expect(mounts.includes('notifyTarget(key)')).toBe(true);
  });
});

describe('stale per-base state is pruned', () => {
  test('unmounting a base drops its cached level, expansion and selection', () => {
    expect(panel.includes('const live = new Set<string>(bases.map((base) => base.knowledge_base_id));')).toBe(
      true
    );
    expect(panel.includes('belongsToLiveBase')).toBe(true);
  });
});

describe('session knowledge panel is a preview, not an editor', () => {
  test('imports none of the knowledge mutation calls', () => {
    for (const mutation of ['writeFile', 'deleteFile', 'createFolder', 'deleteFolder', 'renameTreeEntry']) {
      expect(panel.includes(mutation)).toBe(false);
    }
  });

  test('every opened document is explicitly non-editable', () => {
    expect(panel.includes('editable: false')).toBe(true);
    // The tree only ever carries .md (backend `is_md` gate), so the preview type
    // is fixed rather than sniffed.
    expect(panel.includes("openPreview(file.content, 'markdown'")).toBe(true);
    expect(panel.includes("allow_open_in_system: file.source?.relationship !== 'managed'")).toBe(true);
  });

  test('reuses the surface preview column rather than rendering its own viewer', () => {
    expect(panel.includes('usePreviewContext')).toBe(true);
    expect(panel.includes('Markdown')).toBe(false);
  });
});

describe('tree keys are scoped per knowledge base', () => {
  test('a bare rel_path is never used as a node key', () => {
    // Two mounted bases can both hold README.md; unscoped keys would make them
    // share expand/select state.
    expect(panel.includes("const KEY_SEP = '::'")).toBe(true);
    expect(panel.includes('`${id}${KEY_SEP}${relPath}`')).toBe(true);
    expect(panel.includes("key: 'rel_path'")).toBe(false);
  });

  test('each node carries the base it belongs to', () => {
    expect(panel.includes('knowledgeBaseId: id')).toBe(true);
    expect(panel.includes('knowledge_base_id: node.knowledgeBaseId')).toBe(true);
  });
});

describe('expand-all is one level per root, not a recursive crawl', () => {
  test('fans out at most once per mounted base and tolerates a partial failure', () => {
    expect(panel.includes('Promise.allSettled(')).toBe(true);
    expect(panel.includes('Promise.all(')).toBe(false);
    // Already-loaded levels are not re-listed.
    expect(panel.includes('readableBases.filter((base) => !loadedRef.current[base.knowledge_base_id])')).toBe(
      true
    );
    expect(panel.includes('setExpandedKeys(expandable.map((base) => rootKeyOf(base.knowledge_base_id)))')).toBe(
      true
    );
  });

  test('the first-open latch is only set once the expansion actually succeeded', () => {
    expect(panel.includes('if (ok) autoExpandedRef.current = true;')).toBe(true);
  });

  test('does not reuse the detail page recursive expander', () => {
    expect(panel.includes('handleExpandAllTreeNodes')).toBe(false);
    expect(panel.includes('loadAllChildren')).toBe(false);
  });

  test('collapse-all clears every expanded key', () => {
    expect(panel.includes('setExpandedKeys([])')).toBe(true);
  });

  test('bases whose source directory is gone are never listed', () => {
    expect(panel.includes('bases.filter((base) => base.root_exists)')).toBe(true);
  });
});

describe('mount detection', () => {
  test('requires the binding master switch as well as a non-empty kb_ids', () => {
    // Matches useWorkpathKnowledge's `enabled && kb_ids.length` so the rail icon
    // and the session-list capability dot agree.
    expect(mounts.includes('binding?.enabled ? binding.kb_ids : []')).toBe(true);
  });

  test('caches bindings and bases module-wide with in-flight de-duplication', () => {
    expect(mounts.includes('const bindingCache = new Map')).toBe(true);
    expect(mounts.includes('const bindingInflight = new Set')).toBe(true);
    expect(mounts.includes('basesInflight')).toBe(true);
  });

  test('refreshes from the knowledge WS events instead of polling', () => {
    expect(mounts.includes('onBindingChanged')).toBe(true);
    expect(mounts.includes('onBaseCreated')).toBe(true);
    expect(mounts.includes('onBaseUpdated')).toBe(true);
    expect(mounts.includes('onBaseDeleted')).toBe(true);
  });
});

describe('registration covers every session kind', () => {
  test('both ChatConversation extra-tab sites go through the one helper', () => {
    // Historically these were two independent useMemos; adding a tab to only one
    // silently omitted it from half the conversation kinds.
    const occurrences = chatConversation.match(/useWorkspaceExtraTabs\(conversation\)/g) ?? [];
    expect(occurrences.length).toBe(2);
    expect(chatConversation.includes("key: 'conversation-terminals'")).toBe(false);
  });

  test('the knowledge entry is built in exactly one place for both surfaces', () => {
    // The descriptor used to be hand-copied into TerminalSessionPage, which is
    // the same two-copies divergence useWorkspaceExtraTabs was extracted to kill.
    expect(tabFactory.includes('if (!mounted) return [];')).toBe(true);
    expect(tabFactory.includes('SESSION_KNOWLEDGE_TAB_KEY')).toBe(true);
    expect(tabFactory.includes('<BookOne size={18} />')).toBe(true);
    expect(extraTabs.includes('useSessionKnowledgeTab(')).toBe(true);
    expect(terminalPage.includes('useSessionKnowledgeTab(')).toBe(true);
    // Neither host rebuilds the descriptor itself.
    for (const host of [extraTabs, terminalPage]) {
      expect(host.includes('SESSION_KNOWLEDGE_TAB_KEY,')).toBe(false);
      expect(host.includes('<BookOne')).toBe(false);
    }
  });

  test('the terminal surface now has an extraTabs channel on both consumers', () => {
    expect(terminalPage.includes('extraTabs={workspaceExtraTabs}')).toBe(true);
    expect(terminalRail.includes('extraTabs?: WorkspaceExtraTab[]')).toBe(true);
    expect(terminalRail.includes('extraTabs,\n')).toBe(true);
  });

  test('the terminal panel header resolves extra-tab titles generically', () => {
    // A hardcoded two-way ternary would show "项目" for a third tab.
    expect(
      terminalPage.includes('workspaceExtraTabs.find((tab) => tab.key === activeWorkspaceTab)?.title')
    ).toBe(true);
  });

  test('a terminal resolves its binding from its own session object', () => {
    // useTerminalSessions() filters out conversation-owned terminals, so an
    // id-plus-lookup resolution would silently fail for them.
    expect(terminalPage.includes("kind: 'terminal', session: { cwd: session.cwd")).toBe(true);
  });
});

describe('empty and unavailable states stay honest', () => {
  test('an empty listing is not asserted to mean an empty base', () => {
    // list_tree returns [] both for a genuinely empty base and for a 6s walk
    // budget exhaustion, so the copy must stay neutral.
    expect(panel.includes('knowledge.session.noDocs')).toBe(true);
    for (const overclaim of ['isEmpty', 'baseIsEmpty', 'emptyBase']) {
      expect(panel.includes(overclaim)).toBe(false);
    }
  });

  test('a missing source directory is surfaced on the root row', () => {
    expect(panel.includes('knowledge.mount.rootMissing')).toBe(true);
  });

  test('a failed read reports through the repo formatter without tearing down the tree', () => {
    // knowledgeErrorText surfaces the backend message; String(error) would dump
    // the method, path and JSON body at the user.
    expect(panel.includes('Message.error(knowledgeErrorText(')).toBe(true);
    expect(panel.includes('Message.error(String(')).toBe(false);
  });
});
