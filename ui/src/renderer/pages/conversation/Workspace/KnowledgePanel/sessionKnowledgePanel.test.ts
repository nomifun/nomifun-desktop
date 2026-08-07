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
const extraTabs = read('../../hooks/useWorkspaceExtraTabs.tsx');
const chatConversation = read('../../components/ChatConversation.tsx');
const terminalPage = read('../../../terminal/TerminalSessionPage.tsx');
const terminalRail = read('../../../terminal/TerminalWorkspaceRail.tsx');

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
  test('fans out exactly once per mounted base', () => {
    expect(panel.includes('readableBases.map(')).toBe(true);
    expect(panel.includes('setExpandedKeys(readableBases.map((base) => rootKeyOf(base.knowledge_base_id)))')).toBe(
      true
    );
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

  test('the helper contributes knowledge only when something is mounted', () => {
    expect(extraTabs.includes('if (knowledgeMounted) {')).toBe(true);
    expect(extraTabs.includes('SESSION_KNOWLEDGE_TAB_KEY')).toBe(true);
    expect(extraTabs.includes('<BookOne size={18} />')).toBe(true);
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

  test('a failed read reports without tearing down the tree', () => {
    expect(panel.includes('Message.error(String(error))')).toBe(true);
  });
});
