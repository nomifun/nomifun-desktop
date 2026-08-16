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

const nomi = {
  agent_id: parseAgentId('0190f5fe-7c00-7a00-8000-000000000041'),
  name: 'Nomi',
  backend: 'nomi',
  agent_type: 'nomi',
  agent_source: 'builtin',
  enabled: true,
  available: true,
} as AgentMetadata;

describe('scheduled task agent presentation', () => {
  test('keeps the frozen preset name instead of replacing it with the backing runtime name', () => {
    const job = jobWithAgent('nomi', {
      backend: 'nomi',
      name: 'Bug 排查',
      preset_id: parsePresetReference('0190f5fe-7c00-7a00-8000-000000000011'),
    });

    expect(getJobAgentMeta(job, [nomi]).name).toBe('Bug 排查');
  });

  test('uses the detected runtime name for a direct Agent selection', () => {
    const job = jobWithAgent('nomi', {
      backend: 'nomi',
      name: 'Nomi',
      custom_agent_id: nomi.agent_id,
    });
    expect(getJobAgentMeta(job, [nomi]).name).toBe('Nomi');
  });

  test('resolves colliding custom Agents by stable ID before backend/type', () => {
    const other = {
      ...nomi,
      agent_id: parseAgentId('0190f5fe-7c00-7a00-8000-000000000042'),
      name: 'Security reviewer',
      backend: undefined,
    } as AgentMetadata;
    const selected = { ...nomi, name: 'Code reviewer', backend: undefined } as AgentMetadata;
    const job = jobWithAgent('nomi', { name: 'Old name', custom_agent_id: other.agent_id });
    expect(getJobAgentMeta(job, [selected, other]).name).toBe('Security reviewer');
  });

  test('keeps the frozen name when a stable custom Agent was deleted', () => {
    const deletedId = parseAgentId('0190f5fe-7c00-7a00-8000-000000000044');
    const job = jobWithAgent('nomi', {
      backend: 'nomi',
      name: 'Deleted security reviewer',
      custom_agent_id: deletedId,
    });

    expect(getJobAgentMeta(job, [nomi]).name).toBe('Deleted security reviewer');
  });

  test('keeps a preset frozen name for a retired runtime discriminant too', () => {
    const job = jobWithAgent('remote', {
      name: 'Operations preset',
      preset_id: parsePresetReference('0190f5fe-7c00-7a00-8000-000000000043'),
    });
    expect(getJobAgentMeta(job, []).name).toBe('Operations preset');
  });
});
