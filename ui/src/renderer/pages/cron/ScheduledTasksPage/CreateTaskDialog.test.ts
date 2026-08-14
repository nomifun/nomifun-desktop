/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./CreateTaskDialog.tsx', import.meta.url), 'utf8');

describe('CreateTaskDialog conversation id presentation', () => {
  test('formats and searches stable conversation UUIDs through shortSessionId without a # prefix', () => {
    expect(source.includes("import { shortSessionId } from '@renderer/utils/ui/shortId'")).toBe(true);
    expect(source.includes('const idLabel = shortSessionId(conv.id)')).toBe(true);
    expect(source.includes('const shortId = shortSessionId(conv.id).toLowerCase()')).toBe(true);
    expect(source.includes('`#${conv.id}`')).toBe(false);
  });
});

describe('CreateTaskDialog preset identity presentation', () => {
  test('uses the safe identity component instead of rendering avatar values as text', () => {
    expect(source.includes('<CronPresetOptionIdentity')).toBe(true);
    expect(source.includes("!preset.avatar.endsWith('.svg')")).toBe(false);
    expect(source.includes('<span>{preset.avatar}</span>')).toBe(false);
  });

  test('prevents presets that the backend cannot resolve for scheduled tasks from being selected', () => {
    expect(source.includes("const supportsCron = presetSupportsTarget(preset, 'cron')")).toBe(true);
    expect(source.includes('disabled={!supportsCron}')).toBe(true);
    expect(source.includes("if (!presetSupportsTarget(preset, 'cron'))")).toBe(true);
    expect(source.includes("aria-disabled={!supportsCron || undefined}")).toBe(true);
  });
});

describe('CreateTaskDialog Agent identity contract', () => {
  test('keys direct Agent options by stable AgentRegistry ID and persists that identity', () => {
    expect(source.includes('getCronAgentOptionValue(agent.agent_id)')).toBe(true);
    expect(source.includes('custom_agent_id: agent.agent_id')).toBe(true);
    expect(source.includes('value={`cli:${agentKey}`}')).toBe(false);
  });

  test('locks unsupported cross-runtime changes and preserves unchanged frozen preset snapshots', () => {
    expect(source.includes('disabled={isEditMode}')).toBe(true);
    expect(source.includes('hasCronAgentConfigurationChanged(editJob!, cliAgents')).toBe(true);
    expect(source.includes('...(agentConfigChanged ? { agent_config } : {})')).toBe(true);
  });
});
