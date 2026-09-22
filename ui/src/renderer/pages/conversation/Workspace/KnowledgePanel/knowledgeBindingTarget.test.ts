/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { DEFAULT_WORKPATH_KEY } from '@/renderer/pages/conversation/SessionList/utils/workpathKey';
import type { ConversationId } from '@/common/types/ids';
import {
  knowledgeBindingTargetKey,
  resolveKnowledgeBindingTarget,
  workpathDisplayForKnowledgeTarget,
} from './knowledgeBindingTarget';

describe('resolveKnowledgeBindingTarget', () => {
  test('a canonical conversation resolves its dedicated live AgentSession target', () => {
    expect(resolveKnowledgeBindingTarget({
      kind: 'conversation',
      sessionId: '0190f5fe-7c00-7a00-8abc-012345678901' as ConversationId,
    })).toEqual({
      kind: 'conversation',
      target_id: '0190f5fe-7c00-7a00-8abc-012345678901',
    });
  });

  test('a terminal resolves through its own session object, never an id lookup', () => {
    expect(
      resolveKnowledgeBindingTarget({
        kind: 'terminal',
        session: { cwd: '/tmp/proj', is_default_workpath: false },
      })?.kind
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
    expect(knowledgeBindingTargetKey({ kind: 'companion', target_id: 'x' })).toBe('companion:x');
    expect(knowledgeBindingTargetKey({ kind: 'conversation', target_id: 'x' })).toBe('conversation:x');
  });
});

describe('workpathDisplayForKnowledgeTarget', () => {
  test('does not present another mutable target id as a workspace scope', () => {
    expect(
      workpathDisplayForKnowledgeTarget({
        kind: 'companion',
        target_id: '0190f5fe-7c00-7a00-8abc-012345678902',
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
