import { describe, expect, test } from 'bun:test';
import { parseMcpServerId } from '@/common/types/ids';
import {
  buildSessionCapabilitySelection,
  defaultSessionCapabilityDraft,
  draftFromSessionCapabilitySelection,
} from './model';

describe('Session capability selection model', () => {
  test('encodes deselected auto skills separately from explicit skills', () => {
    const auto = new Set(['cron', 'skill-creator']);
    expect(buildSessionCapabilitySelection({
      skillNames: ['cron', 'pdf', 'pdf'],
      mcpServerIds: ['mcp-b', 'mcp-a', 'mcp-b'],
    }, auto)).toEqual({
      enabled_skills: ['pdf'],
      excluded_auto_skills: ['skill-creator'],
      mcp_server_ids: ['mcp-b', 'mcp-a'],
    });
  });

  test('round-trips the persisted queue selection into an editable draft', () => {
    const auto = new Set(['cron', 'skill-creator']);
    expect(draftFromSessionCapabilitySelection({
      enabled_skills: ['pdf'],
      excluded_auto_skills: ['cron'],
      mcp_server_ids: ['mcp-a'],
    }, auto)).toEqual({
      skillNames: ['pdf', 'skill-creator'],
      mcpServerIds: ['mcp-a'],
    });
  });

  test('keeps visible preset skills and defaults only enabled non-builtin MCP servers', () => {
    const enabledId = parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000001');
    expect(defaultSessionCapabilityDraft({
      skills: [{
        name: 'cron',
        description: 'Scheduled tasks',
        location: '',
        is_custom: false,
        source: 'builtin',
        auto: true,
      }],
      autoSkillNames: new Set(['cron']),
      mcpServers: [
        {
          mcp_server_id: enabledId,
          name: 'Filesystem',
          description: 'Workspace tools',
          enabled: true,
          builtin: false,
          transport: { type: 'stdio', command: 'filesystem' },
          created_at: 1,
          updated_at: 1,
          original_json: '{}',
        },
        {
          mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000002'),
          name: 'Disabled',
          enabled: false,
          builtin: false,
          transport: { type: 'stdio', command: 'disabled' },
          created_at: 1,
          updated_at: 1,
          original_json: '{}',
        },
        {
          mcp_server_id: parseMcpServerId('0190f5fe-7c00-7a00-8000-000000000003'),
          name: 'Built-in',
          enabled: true,
          builtin: true,
          transport: { type: 'stdio', command: 'builtin' },
          created_at: 1,
          updated_at: 1,
          original_json: '{}',
        },
      ],
    }, ['missing-skill'])).toEqual({
      skillNames: ['cron'],
      mcpServerIds: [enabledId],
    });
  });
});
