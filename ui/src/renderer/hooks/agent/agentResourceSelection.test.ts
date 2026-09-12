import { describe, expect, test } from 'bun:test';
import {
  requiredAgentResourcePickerKinds,
  resolveAgentResourceSelections,
} from './agentResourceSelection';

describe('Agent resource selection contract', () => {
  test('submits only kind/id while server-owned resources use exact sentinels', () => {
    expect(resolveAgentResourceSelections(
      ['workspace', 'project_memory', 'process_session', 'terminal', 'asset_library', 'knowledge_base'],
      { knowledge_base: 'kb-1' },
    )).toEqual({
      selections: [
        { resource_kind: 'asset_library', resource_id: 'creative-studio-assets' },
        { resource_kind: 'knowledge_base', resource_id: 'kb-1' },
        { resource_kind: 'process_session', resource_id: 'managed-process-session' },
        { resource_kind: 'project_memory', resource_id: 'default-project-memory' },
        { resource_kind: 'terminal', resource_id: 'managed-terminal' },
        { resource_kind: 'workspace', resource_id: 'default-workspace' },
      ],
      missingKinds: [],
    });
  });

  test('uses one companion choice for companion and companion memory', () => {
    expect(resolveAgentResourceSelections(
      ['companion', 'companion_memory'],
      { companion: 'companion-1' },
    )).toEqual({
      selections: [
        { resource_kind: 'companion', resource_id: 'companion-1' },
        { resource_kind: 'companion_memory', resource_id: 'companion-1' },
      ],
      missingKinds: [],
    });
    expect(requiredAgentResourcePickerKinds(['companion_memory', 'companion'])).toEqual(['companion']);
  });

  test('reports missing and unsupported kinds instead of inventing ids', () => {
    expect(resolveAgentResourceSelections(
      ['customer', 'channel', 'future_resource'],
      { customer: 'customer-1' },
    )).toEqual({
      selections: [{ resource_kind: 'customer', resource_id: 'customer-1' }],
      missingKinds: ['channel', 'future_resource'],
    });
  });

  test('covers every resource kind used by the seven official Agents', () => {
    const officialKinds = [
      'workspace', 'knowledge_base', 'project_memory', 'process_session', 'terminal',
      'mcp_server', 'companion', 'companion_memory', 'channel', 'robot', 'customer',
      'canvas', 'asset_library', 'generation_provider', 'miniapp',
    ];
    const value = {
      companion: 'companion-1',
      channel: 'channel-1',
      robot: 'robot-1',
      customer: 'customer-1',
      knowledge_base: 'kb-1',
      mcp_server: 'mcp-1',
      canvas: 'canvas-1',
      generation_provider: 'provider-1',
      miniapp: 'miniapp-1',
    };

    expect(requiredAgentResourcePickerKinds(officialKinds)).toEqual([
      'companion', 'customer', 'knowledge_base', 'channel', 'robot',
      'mcp_server', 'canvas', 'generation_provider', 'miniapp',
    ]);
    const resolution = resolveAgentResourceSelections(officialKinds, value);
    expect(resolution.missingKinds).toEqual([]);
    expect(resolution.selections).toHaveLength(15);
  });
});
