/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ConversationId } from '@/common/types/ids';
import { DEFAULT_WORKPATH_KEY } from '@/renderer/pages/conversation/SessionList/utils/workpathKey';
import { knowledgeBindingTargetKey, resolveKnowledgeBindingTarget } from './knowledgeBindingTarget';

const CONVERSATION_ID = '0190f5fe-7c00-7a00-8abc-012345678902' as ConversationId;
const COMPANION_ID = '0190f5fe-7c00-7a00-8abc-012345678901';

const conversation = (extra: Record<string, unknown> | undefined) =>
  resolveKnowledgeBindingTarget({ kind: 'conversation', conversationId: CONVERSATION_ID, extra });

describe('resolveKnowledgeBindingTarget', () => {
  // Mirrors crates/backend/nomifun-conversation/src/service.rs
  // `knowledge_binding_target_*` unit tests plus the mount dispatcher's
  // `preset_knowledge_binding` branch.
  test('a companion session reads the per-companion binding', () => {
    expect(conversation({ companion_id: COMPANION_ID, workspace: '/tmp/ws' })).toEqual({
      kind: 'companion',
      target_id: COMPANION_ID,
    });
  });

  test('companion_id outranks preset_knowledge_binding', () => {
    expect(
      conversation({
        companion_id: COMPANION_ID,
        preset_knowledge_binding: true,
        custom_workspace: true,
        workspace: '/tmp/ws',
      })
    ).toEqual({ kind: 'companion', target_id: COMPANION_ID });
  });

  test('a preset-bound conversation reads its own conversation-scoped binding', () => {
    // This is the branch KnowledgeControl's inline memo omits.
    expect(
      conversation({ preset_knowledge_binding: true, custom_workspace: true, workspace: '/tmp/ws' })
    ).toEqual({ kind: 'conversation', target_id: CONVERSATION_ID });
  });

  test('a plain custom-workspace conversation collapses to its workpath', () => {
    const target = conversation({ custom_workspace: true, workspace: '/tmp/ws' });
    expect(target.kind).toBe('workpath');
    expect(target.target_id).not.toBe(CONVERSATION_ID);
    expect(target.target_id.length).toBeGreaterThan(0);
  });

  test('a temporary-workspace conversation maps to the default workpath', () => {
    expect(conversation({ workspace: '/data/conversations/abc' })).toEqual({
      kind: 'workpath',
      target_id: DEFAULT_WORKPATH_KEY,
    });
  });

  test('a blank or non-string companion_id falls through instead of binding to it', () => {
    expect(conversation({ companion_id: '   ' }).kind).toBe('workpath');
    expect(conversation({ companion_id: '' }).kind).toBe('workpath');
    expect(conversation({ companion_id: 42 }).kind).toBe('workpath');
  });

  test('preset_knowledge_binding only triggers on a real boolean true', () => {
    expect(conversation({ preset_knowledge_binding: 'true' }).kind).toBe('workpath');
    expect(conversation({ preset_knowledge_binding: 1 }).kind).toBe('workpath');
    expect(conversation({ preset_knowledge_binding: false }).kind).toBe('workpath');
  });

  test('a missing extra bag does not throw', () => {
    expect(conversation(undefined)).toEqual({ kind: 'workpath', target_id: DEFAULT_WORKPATH_KEY });
  });

  test('a terminal resolves through its own session object, never an id lookup', () => {
    expect(
      resolveKnowledgeBindingTarget({
        kind: 'terminal',
        session: { cwd: '/tmp/proj', is_default_workpath: false },
      }).kind
    ).toBe('workpath');

    expect(
      resolveKnowledgeBindingTarget({
        kind: 'terminal',
        session: { cwd: '/tmp/proj', is_default_workpath: true },
      })
    ).toEqual({ kind: 'workpath', target_id: DEFAULT_WORKPATH_KEY });
  });
});

describe('knowledgeBindingTargetKey', () => {
  test('separates the two kinds that can share an id string', () => {
    expect(knowledgeBindingTargetKey({ kind: 'workpath', target_id: 'x' })).toBe('workpath:x');
    expect(knowledgeBindingTargetKey({ kind: 'conversation', target_id: 'x' })).toBe('conversation:x');
  });
});
