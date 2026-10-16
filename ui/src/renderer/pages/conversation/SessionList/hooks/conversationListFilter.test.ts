/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseExecutionStepId } from '@/common/types/ids';

import { isOrdinaryWorkConversation } from './conversationListFilter';

describe('ordinary conversation list ownership', () => {
  test('App project membership excludes only the confirmed application Session', () => {
    const ordinary = { extra: { workspace: 'C:/apps/project', official_template_key: 'app.developer' } };
    expect(isOrdinaryWorkConversation({ ...ordinary, app_project_id: 'owned-app' } as never)).toBe(false);
    expect(isOrdinaryWorkConversation(ordinary as never)).toBe(true);
    expect(isOrdinaryWorkConversation({ extra: { ...ordinary.extra, app_project_id: 'owned-app' } } as never)).toBe(true);
  });
  test('retained execution attempt transcripts stay out of the ordinary list', () => {
    const transcript = {
      execution_step_id: parseExecutionStepId('0190f5fe-7c00-7a00-8000-000000000001'),
      extra: {},
    };

    expect(isOrdinaryWorkConversation(transcript as never)).toBe(false);
  });

  test('ordinary conversations remain visible', () => {
    const conversation = {
      execution_step_id: undefined,
      extra: {},
    };

    expect(isOrdinaryWorkConversation(conversation as never)).toBe(true);
  });

  test('host-resolved companion ownership excludes every Session for the companion', () => {
    const companionSession = {
      execution_step_id: undefined,
      extra: { companion_id: '019b0000-0000-7000-8000-000000000001' },
    };
    expect(isOrdinaryWorkConversation(companionSession as never)).toBe(false);
  });

  test('an Agent template or capabilities do not claim companion product ownership', () => {
    const companionSession = {
      execution_step_id: undefined,
      extra: {
        custom_workspace: true,
        workspace: 'C:/data/companion/workspaces/1-mochi',
      },
      agent_snapshot: {
        preset_name: 'companion.default',
        enabled_capabilities: ['companion', 'companion.memory'],
      },
    };
    expect(isOrdinaryWorkConversation(companionSession as never)).toBe(true);
  });

  test('a robot Agent remains ordinary without host-resolved companion ownership', () => {
    expect(isOrdinaryWorkConversation({
      extra: {},
      agent_snapshot: { preset_name: 'companion.robot', enabled_capabilities: [] },
    } as never)).toBe(true);
  });

  test('a retired companion marker does not claim product ownership', () => {
    expect(isOrdinaryWorkConversation({
      extra: { companion_session: true },
    } as never)).toBe(true);
  });

  test('channel-sourced Sessions remain isolated from the work list', () => {
    expect(isOrdinaryWorkConversation({
      extra: { channel_platform: 'telegram' },
    } as never)).toBe(false);
  });
});
