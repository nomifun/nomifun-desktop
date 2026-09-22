import { describe, expect, test } from 'bun:test';
import {
  requiredAgentResourcePickerKinds,
  resolveAgentResourceSelections,
  selectedKnowledgeResourceIds,
  selectedCapabilityIds,
  allowsMultipleMcpServers,
} from './agentResourceSelection';

describe('Agent resource selection contract', () => {
  test('resolves host-owned Browser, Computer and Scheduler identities without a picker', () => {
    expect(resolveAgentResourceSelections(['browser', 'computer', 'scheduler'], {})).toEqual({
      selections: [
        { resource_kind: 'browser', resource_id: 'managed-browser' },
        { resource_kind: 'computer', resource_id: 'local-desktop' },
        { resource_kind: 'scheduler', resource_id: 'installation-scheduler' },
      ],
      missingKinds: [],
    });
  });

  test('uses the enabled capability model for frozen multi-server MCP resources', () => {
    const capabilities = selectedCapabilityIds([
      { capability: { id: `nomi.mcp.v1.${'a'.repeat(64)}` } },
    ]);
    expect(allowsMultipleMcpServers(capabilities)).toBe(true);
    expect(allowsMultipleMcpServers([])).toBe(false);
    expect(allowsMultipleMcpServers(['knowledge'])).toBe(false);
    expect(resolveAgentResourceSelections(['mcp_server'], {
      mcp_servers: ['server-1', 'server-2', 'server-1'], mcp_server: 'legacy-server',
    })).toEqual({
      selections: [
        { resource_kind: 'mcp_server', resource_id: 'server-1' },
        { resource_kind: 'mcp_server', resource_id: 'server-2' },
      ],
      missingKinds: [],
    });
  });

  test('submits every frozen Knowledge base as kind/id while host resources use exact sentinels', () => {
    expect(selectedKnowledgeResourceIds({
      knowledge_bases: ['kb-1', 'kb-2', 'kb-1'],
      knowledge_base: 'legacy-kb',
    })).toEqual(['kb-1', 'kb-2']);
    expect(resolveAgentResourceSelections(
      ['workspace', 'project_memory', 'process_session', 'terminal', 'asset_library', 'knowledge_base'],
      { knowledge_bases: ['kb-1', 'kb-2', 'kb-1'] },
    )).toEqual({
      selections: [
        { resource_kind: 'asset_library', resource_id: 'creative-studio-assets' },
        { resource_kind: 'knowledge_base', resource_id: 'kb-1' },
        { resource_kind: 'knowledge_base', resource_id: 'kb-2' },
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

  test('allows optional channel absence while reporting unsupported kinds', () => {
    expect(resolveAgentResourceSelections(
      ['customer', 'channel', 'future_resource'],
      { customer: 'customer-1' },
    )).toEqual({
      selections: [{ resource_kind: 'customer', resource_id: 'customer-1' }],
      missingKinds: ['future_resource'],
    });
  });

  test('allows enhancement resources to remain unbound without blocking the Session', () => {
    expect(resolveAgentResourceSelections(
      ['workspace', 'knowledge_base', 'channel', 'robot', 'canvas', 'plugin', 'ssh_host'],
      {},
    )).toEqual({
      selections: [{ resource_kind: 'workspace', resource_id: 'default-workspace' }],
      missingKinds: [],
    });
    expect(resolveAgentResourceSelections(['computer'], {})).toEqual({
      selections: [{ resource_kind: 'computer', resource_id: 'local-desktop' }],
      missingKinds: [],
    });
  });

  test('covers every current first-party Agent resource kind', () => {
    const officialKinds = [
      'workspace', 'knowledge_base', 'project_memory', 'process_session', 'terminal',
      'mcp_server', 'companion', 'companion_memory', 'channel', 'robot', 'customer',
      'canvas', 'asset_library', 'plugin', 'browser', 'computer', 'scheduler',
    ];
    const value = {
      companion: 'companion-1',
      channel: 'channel-1',
      robot: 'robot-1',
      customer: 'customer-1',
      knowledge_bases: ['kb-1', 'kb-2'],
      mcp_server: 'mcp-1',
      canvas: 'canvas-1',
      plugin: 'plugin-1',
    };

    expect(requiredAgentResourcePickerKinds(officialKinds)).toEqual([
      'companion', 'customer', 'knowledge_base', 'channel', 'robot',
      'mcp_server', 'canvas', 'plugin',
    ]);
    const resolution = resolveAgentResourceSelections(officialKinds, value);
    expect(resolution.missingKinds).toEqual([]);
    expect(resolution.selections).toHaveLength(18);
  });
});
