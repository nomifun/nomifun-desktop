/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ConversationId } from '@/common/types/ids';
import { DEFAULT_WORKPATH_KEY } from '@/renderer/pages/conversation/SessionList/utils/workpathKey';
import {
  knowledgeBindingTargetKey,
  resolveKnowledgeBindingTarget,
  workpathDisplayForKnowledgeTarget,
} from './knowledgeBindingTarget';

const CONVERSATION_ID = '0190f5fe-7c00-7a00-8abc-012345678902' as ConversationId;
const COMPANION_ID = '0190f5fe-7c00-7a00-8abc-012345678901';

const conversation = (extra: Record<string, unknown> | undefined) =>
  resolveKnowledgeBindingTarget({ kind: 'conversation', conversationId: CONVERSATION_ID, extra });

describe('resolveKnowledgeBindingTarget', () => {
  test('a companion session reads the per-companion binding', () => {
    expect(conversation({ companion_id: COMPANION_ID, workspace: '/tmp/ws' })).toEqual({
      kind: 'companion',
      target_id: COMPANION_ID,
    });
  });

  test('companion_id outranks ordinary conversation scope', () => {
    expect(
      conversation({
        companion_id: COMPANION_ID,
        custom_workspace: true,
        workspace: '/tmp/ws',
      })
    ).toEqual({ kind: 'companion', target_id: COMPANION_ID });
  });

  test('each work conversation owns its knowledge binding', () => {
    const target = conversation({ custom_workspace: true, workspace: '/tmp/ws' });
    expect(target).toEqual({ kind: 'conversation', target_id: CONVERSATION_ID });
  });

  test('temporary workspaces do not collapse separate conversations together', () => {
    expect(conversation({ workspace: '/data/conversations/abc' })).toEqual({
      kind: 'conversation',
      target_id: CONVERSATION_ID,
    });
  });

  test('a blank or non-string companion_id falls through instead of binding to it', () => {
    expect(conversation({ companion_id: '   ' }).kind).toBe('conversation');
    expect(conversation({ companion_id: '' }).kind).toBe('conversation');
    expect(conversation({ companion_id: 42 }).kind).toBe('conversation');
  });

  test('a missing extra bag does not throw', () => {
    expect(conversation(undefined)).toEqual({
      kind: 'conversation',
      target_id: CONVERSATION_ID,
    });
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

describe('workpathDisplayForKnowledgeTarget', () => {
  test('does not present a conversation id as a workspace scope', () => {
    expect(
      workpathDisplayForKnowledgeTarget({
        kind: 'conversation',
        target_id: CONVERSATION_ID,
      })
    ).toBeNull();
  });

  test('only displays the resolved workpath target', () => {
    expect(
      workpathDisplayForKnowledgeTarget({
        kind: 'workpath',
        target_id: '/tmp/project',
      })
    ).toBe('/tmp/project');
  });
});
