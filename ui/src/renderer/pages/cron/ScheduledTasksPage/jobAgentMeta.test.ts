/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ICronJob } from '@/common/adapter/ipcBridge';
import { parsePresetReference } from '@/common/types/agent/presetTypes';
import { parseAgentId } from '@/common/types/ids';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';
import { getJobAgentMeta } from './jobAgentMeta';

const jobWithAgent = (agent_type: string, agent_config: ICronJob['metadata']['agent_config']): ICronJob =>
  ({
    metadata: { agent_type, agent_config },
  }) as ICronJob;

const claude = {
  agent_id: parseAgentId('0190f5fe-7c00-7a00-8000-000000000041'),
  name: 'Claude Code',
  backend: 'claude',
  agent_type: 'acp',
} as AgentMetadata;

describe('scheduled task agent presentation', () => {
  test('keeps the frozen preset name instead of replacing it with the backing runtime name', () => {
    const job = jobWithAgent('acp', {
      backend: 'claude',
      name: 'Bug 排查',
      preset_id: parsePresetReference('0190f5fe-7c00-7a00-8000-000000000011'),
    });

    expect(getJobAgentMeta(job, [claude]).name).toBe('Bug 排查');
  });

  test('uses the detected runtime name for a direct Agent selection', () => {
    const job = jobWithAgent('acp', {
      backend: 'claude',
      name: 'Claude Code',
      custom_agent_id: claude.agent_id,
    });
    expect(getJobAgentMeta(job, [claude]).name).toBe('Claude Code');
  });

  test('resolves colliding custom ACP Agents by stable ID before backend/type', () => {
    const other = {
      ...claude,
      agent_id: parseAgentId('0190f5fe-7c00-7a00-8000-000000000042'),
      name: 'Security reviewer',
      backend: undefined,
    } as AgentMetadata;
    const selected = { ...claude, name: 'Code reviewer', backend: undefined } as AgentMetadata;
    const job = jobWithAgent('acp', { name: 'Old name', custom_agent_id: other.agent_id });
    expect(getJobAgentMeta(job, [selected, other]).name).toBe('Security reviewer');
  });

  test('keeps the frozen name when a stable custom Agent was deleted', () => {
    const deletedId = parseAgentId('0190f5fe-7c00-7a00-8000-000000000044');
    const job = jobWithAgent('acp', {
      backend: 'claude',
      name: 'Deleted security reviewer',
      custom_agent_id: deletedId,
    });

    expect(getJobAgentMeta(job, [claude]).name).toBe('Deleted security reviewer');
  });

  test('keeps a preset frozen name for non-ACP runtimes too', () => {
    const job = jobWithAgent('remote', {
      name: 'Operations preset',
      preset_id: parsePresetReference('0190f5fe-7c00-7a00-8000-000000000043'),
    });
    expect(getJobAgentMeta(job, []).name).toBe('Operations preset');
  });
});
